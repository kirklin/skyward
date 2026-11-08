use crate::canvas::Canvas;
use anyhow::Result;

/// A present target. The device backend scans out through KMS; the host
/// preview backend encodes PNGs. Keeping the renderer behind this trait is
/// what lets the whole UI be built and reviewed without the phone.
pub trait Surface {
    fn size(&self) -> (u32, u32);
    /// Hand `canvas` to the display. It returns once the frame before it is
    /// on the panel and this one is on its way, so the next frame is drawn
    /// while this one goes out.
    fn present(&mut self, canvas: &Canvas) -> Result<()>;
    /// Wait until the frame presented last is on the panel.
    fn settle(&mut self) -> Result<()>;
}

/// How Qualcomm's display driver drives a mode, from the text of the
/// connector's mode_properties blob: key=value lines, a block per mode_name.
#[cfg(any(target_os = "android", test))]
#[derive(Debug, PartialEq, Eq)]
struct SdeMode {
    /// Layer mixers in the mode's topology, each blending one vertical
    /// strip of the screen.
    mixers: u32,
    dsc: bool,
}

#[cfg(any(target_os = "android", test))]
impl SdeMode {
    fn find(text: &str, name: &str) -> Result<SdeMode> {
        let mut current = false;
        let (mut topology, mut dsc) = (None, false);
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            if key == "mode_name" {
                if current {
                    break;
                }
                current = value == name;
            } else if current {
                match key {
                    "topology" => topology = Some(value),
                    "dsc_mode" => dsc = value == "1",
                    _ => {}
                }
            }
        }
        let topology = topology.ok_or_else(|| anyhow::anyhow!("mode_properties describes no topology for {name}"))?;
        let mixers = if topology.contains("quadpipe") {
            4
        } else if topology.contains("dualpipe") {
            2
        } else {
            1
        };
        Ok(SdeMode { mixers, dsc })
    }
}

/// Sub-pixel rendering by the display processor. A panel that shows two of
/// the three colours at each pixel, and leaves the rendering for that to the
/// display processor, names its layout as spr_pack_type in the connector's
/// capabilities blob; without the rendering, its red sub-pixels show the
/// blue of the frame. The CRTC takes the setup in SDE_SPR_INIT_CFG_V1, or
/// in SDE_SPR_INIT_CFG_V2, which adds the cfg18 fields after the first's.
#[cfg(any(target_os = "android", test))]
mod spr {
    use anyhow::{bail, Result};

    /// Red and blue taken at every other pixel, green at each.
    const DECIMATION: [u16; 4] = [1, 0, 1, 0];
    /// Two colour phases, eight steps apart.
    const COLOUR_PHASE_STEP: u16 = 8;
    const COLOUR_PHASE_REPEAT: u16 = 2;
    const COLOUR_PHASES: [i32; 24] = [-2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, -2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    /// Filtering through four taps, with their coefficients.
    const FOUR_TAP: u16 = 2;
    const FOUR_TAP_COEFFICIENTS: [i32; 16] = [128, 256, 128, 0, 86, 241, 164, 21, 52, 204, 204, 52, 21, 164, 241, 86];
    const ADAPTIVE_YYGM: u16 = 0;
    const ADAPTIVE_STRENGTHS: [u16; 5] = [0, 4, 8, 12, 16];
    const OPR_GAINS: [u16; 4] = [341, 341, 341, 0];
    const OPR_OFFSETS: [u16; 32] = [
        0, 132, 264, 396, 529, 661, 793, 925, 1057, 1189, 1321, 1453, 1586, 1718, 1850, 1982, 2114, 2246, 2378, 2510, 2643, 2775, 2907,
        3039, 3171, 3303, 3435, 3567, 3700, 3832, 3964, 4095,
    ];

    /// The layout the capabilities blob names, among its key=value lines.
    pub fn pack_type(capabilities: &str) -> Option<&str> {
        capabilities.lines().find_map(|line| line.strip_prefix("spr_pack_type="))
    }

    /// The setup for a panel laid out as `pack_type`, as the CRTC property
    /// `property` takes it: the driver's structure, its fields in order and
    /// aligned as the phone's C compiler aligns them.
    pub fn setup(pack_type: &str, property: &str) -> Result<Vec<u8>> {
        if pack_type != "pentile" {
            bail!("the panel's sub-pixels are laid out as {pack_type}, and only a pentile layout is set up");
        }
        let cfg18 = match property {
            "SDE_SPR_INIT_CFG_V1" => false,
            "SDE_SPR_INIT_CFG_V2" => true,
            other => bail!("no sub-pixel rendering setup is laid out for {other}"),
        };
        let mut out = Vec::new();
        // flags
        out.extend(0u64.to_ne_bytes());
        // cfg0 to cfg9: the block on, no white sub-pixel, and the phases,
        // filter and adaptive mode
        put16(&mut out, &[1, 1, 1, 0, 0, COLOUR_PHASE_STEP, COLOUR_PHASE_REPEAT, FOUR_TAP, ADAPTIVE_YYGM, 0]);
        // cfg10
        out.extend(0u32.to_ne_bytes());
        // cfg11 to cfg15
        put16(&mut out, &DECIMATION);
        put16(&mut out, &[0; 4]);
        put16(&mut out, &OPR_GAINS);
        put16(&mut out, &ADAPTIVE_STRENGTHS);
        put16(&mut out, &OPR_OFFSETS);
        // cfg16 and cfg17, on four bytes
        out.extend([0; 2]);
        put32(&mut out, &FOUR_TAP_COEFFICIENTS);
        put32(&mut out, &COLOUR_PHASES);
        // cfg18_en and cfg18, off
        if cfg18 {
            out.extend([0; 9]);
        }
        // The structure ends on eight bytes, as its flags are aligned.
        out.resize(out.len().next_multiple_of(8), 0);
        Ok(out)
    }

    fn put16(out: &mut Vec<u8>, values: &[u16]) {
        for v in values {
            out.extend(v.to_ne_bytes());
        }
    }

    fn put32(out: &mut Vec<u8>, values: &[i32]) {
        for v in values {
            out.extend(v.to_ne_bytes());
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn u16_at(b: &[u8], at: usize) -> u16 {
            u16::from_ne_bytes([b[at], b[at + 1]])
        }

        fn i32_at(b: &[u8], at: usize) -> i32 {
            i32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
        }

        #[test]
        fn the_layout_is_read_from_the_capabilities() {
            let text = "panel name=xiaomi o2 42 02 0a cmd mode dsc dsi panel\nspr_pack_type=pentile\nspr_pack_type_mode=BG-RG Type B\n";
            assert_eq!(pack_type(text), Some("pentile"));
            assert_eq!(pack_type("panel name=xiaomi 42 02 0a cmd mode dsc dsi panel\nspr_pack_type_mode=BG-RG Type B\n"), None, "the mode alone names none");
        }

        #[test]
        fn the_pentile_setup_is_laid_out_as_the_driver_s_structure() {
            let v1 = setup("pentile", "SDE_SPR_INIT_CFG_V1").unwrap();
            let v2 = setup("pentile", "SDE_SPR_INIT_CFG_V2").unwrap();
            assert_eq!((v1.len(), v2.len()), (296, 304));
            assert_eq!(v1[..292], v2[..292], "the second adds fields after the first's");
            assert_eq!(v2[292..], [0; 12], "cfg18 off");
            assert_eq!(u16_at(&v1, 8), 1, "cfg0, the block on");
            assert_eq!((u16_at(&v1, 18), u16_at(&v1, 20), u16_at(&v1, 22), u16_at(&v1, 24)), (8, 2, 2, 0), "cfg5 to cfg8");
            assert_eq!((u16_at(&v1, 32), u16_at(&v1, 34)), (1, 0), "cfg11: red decimated, green not");
            assert_eq!(u16_at(&v1, 68), 132, "cfg15 starts at 66");
            assert_eq!(u16_at(&v1, 128), 4095, "and ends at 130");
            assert_eq!((i32_at(&v1, 132), i32_at(&v1, 136)), (128, 256), "cfg16 starts at 132");
            assert_eq!((i32_at(&v1, 196), i32_at(&v1, 200), i32_at(&v1, 244)), (-2, 2, 2), "cfg17 starts at 196");
        }

        #[test]
        fn only_a_pentile_layout_and_the_known_properties_are_set_up() {
            assert!(setup("rgbw", "SDE_SPR_INIT_CFG_V2").is_err());
            assert!(setup("pentile", "SDE_SPR_INIT_CFG_V3").is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISHTAR: &str = "mode_name=1440x3200x60cmd\nbit_clk_rate=1360000000\ndsc_mode=1\ntopology=sde_dualpipe_dscmerge\n\
                          mode_name=1440x3200x120cmd\nsubmode_idx=1\ndsc_mode=1\ntopology=sde_dualpipe_dscmerge\n\
                          mode_name=1080x2400x60cmd\ndsc_mode=0\ntopology=sde_singlepipe\n";

    #[test]
    fn reads_the_topology_of_the_named_mode() {
        assert_eq!(SdeMode::find(ISHTAR, "1440x3200x120cmd").unwrap(), SdeMode { mixers: 2, dsc: true });
        assert_eq!(SdeMode::find(ISHTAR, "1080x2400x60cmd").unwrap(), SdeMode { mixers: 1, dsc: false });
        assert!(SdeMode::find(ISHTAR, "1440x3200x90cmd").is_err());
    }
}

#[cfg(target_os = "android")]
pub mod kms {
    use super::*;
    use anyhow::{anyhow, Context};
    use drm::buffer::{self, DrmFourcc, DrmModifier, PlanarBuffer};
    use drm::control::dumbbuffer::DumbBuffer;
    use drm::control::{
        atomic, connector, framebuffer, plane, property, AtomicCommitFlags, Device as ControlDevice, Event, FbCmd2Flags, ModeTypeFlags, PlaneType,
        ResourceHandle,
    };
    use drm::Device;
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
    use std::time::{Duration, Instant};

    /// How long the display may take to complete a commit.
    const COMMIT_TIMEOUT: Duration = Duration::from_secs(2);

    struct Card(File);

    impl AsFd for Card {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.0.as_fd()
        }
    }

    impl Device for Card {}
    impl ControlDevice for Card {}

    /// A single-plane dumb buffer presented to ADDFB2, so the framebuffer
    /// keeps the buffer's own pixel format.
    struct Planar<'a>(&'a DumbBuffer);

    impl PlanarBuffer for Planar<'_> {
        fn size(&self) -> (u32, u32) {
            buffer::Buffer::size(self.0)
        }
        fn format(&self) -> DrmFourcc {
            buffer::Buffer::format(self.0)
        }
        fn modifier(&self) -> Option<DrmModifier> {
            None
        }
        fn pitches(&self) -> [u32; 4] {
            [buffer::Buffer::pitch(self.0), 0, 0, 0]
        }
        fn handles(&self) -> [Option<buffer::Handle>; 4] {
            [Some(buffer::Buffer::handle(self.0)), None, None, None]
        }
        fn offsets(&self) -> [u32; 4] {
            [0; 4]
        }
    }

    /// Byte order of the scanout buffer against the canvas's RGBA.
    #[derive(Clone, Copy, Debug)]
    enum Order {
        Rgba,
        Bgra,
    }

    struct Buf {
        db: DumbBuffer,
        fb: framebuffer::Handle,
        /// The buffer mapped for writing for as long as the surface lives;
        /// mapping it per frame costs a page fault per page on every frame.
        map: *mut u8,
        len: usize,
    }

    /// A KMS object's properties by name, with their current values.
    struct Props(HashMap<String, (property::Handle, u64)>);

    impl Props {
        fn read<T: ResourceHandle>(card: &Card, handle: T) -> Result<Self> {
            let values = card.get_properties(handle).context("read properties")?;
            let mut map = HashMap::new();
            for (&id, &value) in values.iter() {
                let info = card.get_property(id).context("read property")?;
                map.insert(info.name().to_string_lossy().into_owned(), (id, value));
            }
            Ok(Props(map))
        }

        fn get(&self, name: &str) -> Result<(property::Handle, u64)> {
            self.0.get(name).copied().ok_or_else(|| anyhow!("KMS property {name} missing"))
        }

        fn handle(&self, name: &str) -> Result<property::Handle> {
            Ok(self.get(name)?.0)
        }

        fn value(&self, name: &str) -> Result<u64> {
            Ok(self.get(name)?.1)
        }

        fn line(&self) -> String {
            let mut parts: Vec<String> = self.0.iter().map(|(name, (_, value))| format!("{name}={value}")).collect();
            parts.sort();
            parts.join(" ")
        }
    }

    fn open_card(path: &str) -> Result<Card> {
        let file = OpenOptions::new().read(true).write(true).open(path).with_context(|| format!("open {path}"))?;
        let card = Card(file);
        card.set_client_capability(drm::ClientCapability::UniversalPlanes, true).context("enable universal planes")?;
        card.set_client_capability(drm::ClientCapability::Atomic, true).context("enable atomic modesetting")?;
        Ok(card)
    }

    /// The value of the choice named `choice` of an enum property.
    fn enum_value(card: &Card, id: property::Handle, choice: &str) -> Result<u64> {
        let info = card.get_property(id).context("read property")?;
        match info.value_type() {
            property::ValueType::Enum(e) => e.values().1.iter().find(|v| v.name().to_str() == Ok(choice)).map(|v| v.value()),
            _ => None,
        }
        .ok_or_else(|| anyhow!("{} offers no {choice}", info.name().to_string_lossy()))
    }

    /// Built-in panel interfaces. Writeback and external outputs are never
    /// the recovery's screen.
    fn is_panel(interface: connector::Interface) -> bool {
        matches!(interface, connector::Interface::DSI | connector::Interface::EmbeddedDisplayPort | connector::Interface::LVDS)
    }

    /// Commit without blocking and wait for the flip event, so a commit the
    /// display never completes surfaces as an error instead of an endless
    /// wait.
    fn commit(card: &Card, req: atomic::AtomicModeReq, flags: AtomicCommitFlags) -> Result<()> {
        card.atomic_commit(flags | AtomicCommitFlags::NONBLOCK | AtomicCommitFlags::PAGE_FLIP_EVENT, req).context("atomic commit")?;
        flipped(card)
    }

    /// Wait for the flip event of the commit made last, or fail once the
    /// display has kept it waiting too long.
    fn flipped(card: &Card) -> Result<()> {
        let deadline = Instant::now() + COMMIT_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let mut pfd = libc::pollfd { fd: card.as_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let n = unsafe { libc::poll(&mut pfd, 1, left.as_millis() as i32) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err).context("wait for the display");
            }
            if n == 0 {
                return Err(anyhow!("the display did not complete the commit within {} ms", COMMIT_TIMEOUT.as_millis()));
            }
            for event in card.receive_events().context("read display events")? {
                if let Event::PageFlip(_) = event {
                    return Ok(());
                }
            }
        }
    }

    /// Connectors with their modes and properties, encoders with the CRTC
    /// they feed, CRTCs with their mode and properties, and the planes in
    /// use: what a device has for scanout and how it is wired right now.
    pub fn report(path: &str) -> Result<String> {
        use std::fmt::Write;
        let card = open_card(path)?;
        let res = card.resource_handles().context("read KMS resources")?;
        let mut out = String::new();
        for &h in res.connectors() {
            let c = card.get_connector(h, true).context("read connector")?;
            let encoder = c.current_encoder().map(u32::from);
            writeln!(out, "connector {} {:?} {:?} encoder {encoder:?} {} modes", u32::from(h), c.interface(), c.state(), c.modes().len())?;
            let props = Props::read(&card, h)?;
            writeln!(out, "  {}", props.line())?;
            if is_panel(c.interface()) {
                if let Ok((_, blob)) = props.get("mode_properties") {
                    let text = card.get_property_blob(blob).context("read mode_properties")?;
                    writeln!(out, "  mode_properties: {}", String::from_utf8_lossy(&text).replace('\n', " | "))?;
                }
                if let Ok((_, blob)) = props.get("capabilities") {
                    let text = card.get_property_blob(blob).context("read capabilities")?;
                    writeln!(out, "  capabilities: {}", String::from_utf8_lossy(&text).replace('\n', " | "))?;
                }
                for m in c.modes() {
                    let (w, hgt) = m.size();
                    writeln!(out, "  mode {:?} {w}x{hgt} {} Hz clock {} type {:?} flags {:?}", m.name(), m.vrefresh(), m.clock(), m.mode_type(), m.flags())?;
                }
            }
        }
        for &h in res.encoders() {
            let e = card.get_encoder(h).context("read encoder")?;
            let possible: Vec<u32> = res.filter_crtcs(e.possible_crtcs()).into_iter().map(u32::from).collect();
            writeln!(out, "encoder {} {:?} crtc {:?} possible {possible:?}", u32::from(h), e.kind(), e.crtc().map(u32::from))?;
        }
        for &h in res.crtcs() {
            let i = card.get_crtc(h).context("read CRTC")?;
            let mode = i.mode().map(|m| format!("{:?} {} Hz", m.name(), m.vrefresh()));
            writeln!(out, "crtc {} mode {mode:?} fb {:?}", u32::from(h), i.framebuffer().map(u32::from))?;
            writeln!(out, "  {}", Props::read(&card, h)?.line())?;
        }
        for p in card.plane_handles().context("list planes")? {
            let i = card.get_plane(p).context("read plane")?;
            let props = Props::read(&card, p)?;
            writeln!(
                out,
                "plane {} type {} crtc {:?} fb {:?} possible {} formats {}",
                u32::from(p),
                props.value("type")?,
                i.crtc().map(u32::from),
                i.framebuffer().map(u32::from),
                res.filter_crtcs(i.possible_crtcs()).len(),
                i.formats().len()
            )?;
            if i.crtc().is_some() {
                writeln!(out, "  {}", props.line())?;
            }
        }
        Ok(out)
    }

    /// The panel connector's CRTC, from the encoder driving it.
    fn panel_crtc(card: &Card) -> Result<drm::control::crtc::Handle> {
        let res = card.resource_handles().context("read KMS resources")?;
        for &h in res.connectors() {
            let info = card.get_connector(h, false).context("read connector")?;
            if is_panel(info.interface()) && info.state() == connector::State::Connected {
                let encoder = info.current_encoder().ok_or_else(|| anyhow!("the panel has no encoder; is anything showing?"))?;
                return card.get_encoder(encoder).context("read encoder")?.crtc().ok_or_else(|| anyhow!("the panel's encoder drives no CRTC"));
            }
        }
        Err(anyhow!("no connected built-in panel"))
    }

    /// The framebuffer the panel scans out right now, as tightly packed RGBA
    /// rows: what the display controller sends to the screen. Reading another
    /// client's framebuffer needs root.
    pub fn capture(path: &str) -> Result<Canvas> {
        let card = open_card(path)?;
        let crtc = panel_crtc(&card)?;
        let fb = card.get_crtc(crtc).context("read CRTC")?.framebuffer().ok_or_else(|| anyhow!("nothing is scanned out"))?;
        let info = card.get_framebuffer(fb).context("read framebuffer")?;
        if info.bpp() != 32 {
            return Err(anyhow!("framebuffer has {} bits per pixel, not 32", info.bpp()));
        }
        let handle = info.buffer().ok_or_else(|| anyhow!("no buffer handle for the framebuffer; capturing needs root"))?;
        let (w, h) = info.size();
        let pitch = info.pitch() as usize;
        let map = drm_ffi::mode::dumbbuffer::map(card.as_fd(), handle.into(), 0, 0).context("map the framebuffer")?;
        let len = pitch * h as usize;
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, card.as_fd().as_raw_fd(), map.offset as libc::off_t) };
        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error()).context("mmap the framebuffer");
        }
        let src = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
        let mut c = Canvas::new(w, h);
        let row = w as usize * 4;
        for y in 0..h as usize {
            c.data[y * row..(y + 1) * row].copy_from_slice(&src[y * pitch..y * pitch + row]);
        }
        unsafe { libc::munmap(ptr, len) };
        Ok(c)
    }

    /// Direct scanout on the built-in panel: its fastest mode on the CRTC
    /// driving it, three dumb buffers taken in turn on its primary planes,
    /// and every other connector and plane taken off that CRTC.
    /// How the display stands.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Power {
        On,
        /// The panel rests in its low-power mode, the display processor
        /// stopped.
        Resting,
        Off,
    }

    pub struct KmsSurface {
        card: Card,
        bufs: [Buf; 3],
        /// The buffer the next frame goes into.
        back: usize,
        /// A frame was committed whose flip has not come yet.
        flipping: bool,
        /// The planes showing the buffer, one per layer mixer, with their
        /// FB_ID properties.
        planes: Vec<(plane::Handle, property::Handle)>,
        /// The CRTC driving the panel, with its ACTIVE and MODE_ID
        /// properties.
        crtc: drm::control::crtc::Handle,
        active: property::Handle,
        mode_id: property::Handle,
        /// The panel's preferred mode, which it comes up in, and its
        /// fastest, which it switches to after, where that is another.
        preferred: property::Value<'static>,
        fastest: Option<property::Value<'static>>,
        /// The sub-pixel rendering setup the CRTC takes with each mode,
        /// where the panel leaves its sub-pixel rendering to the display.
        spr: Option<(property::Handle, property::Value<'static>)>,
        /// The panel's connector, with its low-power switch where the
        /// display driver has one, and the switch's values for running and
        /// for resting.
        conn: connector::Handle,
        lp: Option<(property::Handle, u64, u64)>,
        power: Power,
        /// The display came on again in the panel's preferred mode, and the
        /// panel has yet to switch to its fastest.
        slow: bool,
        w: u32,
        h: u32,
        order: Order,
        summary: String,
    }

    impl KmsSurface {
        pub fn open(path: &str) -> Result<Self> {
            let card = open_card(path)?;
            let res = card.resource_handles().context("read KMS resources")?;

            let mut panel = None;
            for &h in res.connectors() {
                let info = card.get_connector(h, true).context("read connector")?;
                if is_panel(info.interface()) && info.state() == connector::State::Connected {
                    panel = Some(info);
                    break;
                }
            }
            let conn = panel.ok_or_else(|| anyhow!("no connected built-in panel"))?;
            let preferred = conn
                .modes()
                .iter()
                .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                .ok_or_else(|| anyhow!("the panel reports no preferred mode"))?;
            // The fastest refresh at the preferred size, the first listed among
            // equals: Android scrolls at it, and the 13 Ultra's touch reports
            // come at the same 120 Hz.
            let mode = *conn
                .modes()
                .iter()
                .filter(|m| m.size() == preferred.size())
                .rev()
                .max_by_key(|m| m.vrefresh())
                .expect("the preferred mode itself has the preferred size");
            let (w, h) = (mode.size().0 as u32, mode.size().1 as u32);

            // The CRTC already driving the panel, or the first one an
            // encoder of this connector can drive.
            let current = match conn.current_encoder() {
                Some(e) => card.get_encoder(e).context("read encoder")?.crtc(),
                None => None,
            };
            let crtc = match current {
                Some(c) => c,
                None => {
                    let mut found = None;
                    for &e in conn.encoders() {
                        let enc = card.get_encoder(e).context("read encoder")?;
                        if let Some(&c) = res.filter_crtcs(enc.possible_crtcs()).first() {
                            found = Some(c);
                            break;
                        }
                    }
                    found.ok_or_else(|| anyhow!("no CRTC can drive the panel"))?
                }
            };

            // Qualcomm's display driver blends a mode on one or more layer
            // mixers, each owning a vertical strip of the screen; the 13
            // Ultra's dual-pipe topology has two. The pipeline is set up the
            // way Android's compositor sets it up: one plane per mixer
            // covering its strip, the mode's DSC variant selected, and frame
            // starts posted to the hardware. With one plane across both
            // mixers, no DSC selection and the default trigger, the 13
            // Ultra's panel tore while scrolling.
            let conn_props = Props::read(&card, conn.handle())?;
            let layout = match conn_props.get("mode_properties") {
                Ok((_, blob)) => {
                    let text = card.get_property_blob(blob).context("read mode_properties")?;
                    SdeMode::find(&String::from_utf8_lossy(&text), &mode.name().to_string_lossy())?
                }
                Err(_) => SdeMode { mixers: 1, dsc: false },
            };
            let strips = layout.mixers as usize;
            if w % layout.mixers != 0 {
                return Err(anyhow!("{w} pixels do not split into {strips} strips"));
            }

            // Primary planes that can drive the CRTC, those already on it
            // first so the boot splash's planes are reused; planes on the
            // CRTC that are not used are switched off.
            let mut candidates: Vec<(plane::Handle, Vec<u32>, Props, bool)> = Vec::new();
            let mut attached = Vec::new();
            for p in card.plane_handles().context("list planes")? {
                let info = card.get_plane(p).context("read plane")?;
                let props = Props::read(&card, p)?;
                let on_crtc = info.crtc() == Some(crtc);
                if on_crtc {
                    attached.push(p);
                }
                if props.value("type")? == PlaneType::Primary as u64 && res.filter_crtcs(info.possible_crtcs()).contains(&crtc) {
                    candidates.push((p, info.formats().to_vec(), props, on_crtc));
                }
            }
            candidates.sort_by_key(|c| !c.3);
            if candidates.len() < strips {
                return Err(anyhow!("{strips} primary planes needed, {} can drive the CRTC", candidates.len()));
            }
            candidates.truncate(strips);

            let pick = [
                (DrmFourcc::Xbgr8888, Order::Rgba),
                (DrmFourcc::Abgr8888, Order::Rgba),
                (DrmFourcc::Xrgb8888, Order::Bgra),
                (DrmFourcc::Argb8888, Order::Bgra),
            ];
            let (fourcc, order) = pick
                .into_iter()
                .find(|(f, _)| candidates.iter().all(|c| c.1.contains(&(*f as u32))))
                .ok_or_else(|| anyhow!("the primary planes share none of the 32-bit RGB formats"))?;

            let make = || -> Result<Buf> {
                let db = card.create_dumb_buffer((w, h), fourcc, 32).context("create dumb buffer")?;
                let fb = card.add_planar_framebuffer(&Planar(&db), FbCmd2Flags::empty()).context("add framebuffer")?;
                let len = buffer::Buffer::pitch(&db) as usize * h as usize;
                let offset = drm_ffi::mode::dumbbuffer::map(card.as_fd(), buffer::Buffer::handle(&db).into(), 0, 0).context("map dumb buffer")?.offset;
                let fd = card.as_fd().as_raw_fd();
                let map = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, offset as libc::off_t) };
                if map == libc::MAP_FAILED {
                    return Err(std::io::Error::last_os_error()).context("mmap dumb buffer");
                }
                Ok(Buf { db, fb, map: map.cast(), len })
            };
            let bufs = [make()?, make()?, make()?];

            let crtc_id = u64::from(u32::from(crtc));
            let mut req = atomic::AtomicModeReq::new();
            for &other in res.connectors().iter().filter(|&&c| c != conn.handle()) {
                let props = Props::read(&card, other)?;
                if props.value("CRTC_ID")? == crtc_id {
                    req.add_property(other, props.handle("CRTC_ID")?, property::Value::CRTC(None));
                }
            }
            let crtc_props = Props::read(&card, crtc)?;
            let blob = card.create_property_blob(&mode).context("create mode blob")?;
            let first = card.create_property_blob(preferred).context("create mode blob")?;
            req.add_property(conn.handle(), conn_props.handle("CRTC_ID")?, property::Value::CRTC(Some(crtc)));
            // The mode is DSC-compressed: select its DSC variant, as Android's
            // compositor does.
            if layout.dsc {
                let (id, _) = conn_props.get("dsc_mode")?;
                req.add_property(conn.handle(), id, property::Value::UnsignedRange(enum_value(&card, id, "dsc_enabled")?));
            }
            // Frame starts are posted to the hardware: the driver's own
            // default on display hardware of this generation, and Android's
            // compositor sets the same.
            if let Ok((id, _)) = conn_props.get("frame_trigger_mode") {
                req.add_property(conn.handle(), id, property::Value::UnsignedRange(enum_value(&card, id, "posted_start")?));
            }
            let (mode_id, active) = (crtc_props.handle("MODE_ID")?, crtc_props.handle("ACTIVE")?);
            req.add_property(crtc, mode_id, first);
            req.add_property(crtc, active, property::Value::Boolean(true));
            // A panel whose sub-pixel rendering is the display processor's
            // has it set up with each mode set: this one, the switch to the
            // fastest refresh, and each time the display comes on again.
            let spr = match conn_props.get("capabilities") {
                Ok((_, caps)) => {
                    let text = card.get_property_blob(caps).context("read capabilities")?;
                    match spr::pack_type(&String::from_utf8_lossy(&text)) {
                        Some(pack_type) => {
                            let name = ["SDE_SPR_INIT_CFG_V2", "SDE_SPR_INIT_CFG_V1"]
                                .into_iter()
                                .find(|name| crtc_props.get(name).is_ok())
                                .ok_or_else(|| anyhow!("the panel leaves its {pack_type} sub-pixel rendering to a CRTC that takes no setup for it"))?;
                            let setup = card.create_property_blob(spr::setup(pack_type, name)?.as_slice()).context("create sub-pixel rendering blob")?;
                            Some((crtc_props.handle(name)?, setup, name))
                        }
                        None => None,
                    }
                }
                Err(_) => None,
            };
            if let Some((id, setup, _)) = spr {
                req.add_property(crtc, id, setup);
            }
            for &other in attached.iter().filter(|&&p| !candidates.iter().any(|c| c.0 == p)) {
                let props = Props::read(&card, other)?;
                req.add_property(other, props.handle("FB_ID")?, property::Value::Framebuffer(None));
                req.add_property(other, props.handle("CRTC_ID")?, property::Value::CRTC(None));
            }
            let strip = w / layout.mixers;
            let mut planes = Vec::new();
            for (i, (plane, _, props, _)) in candidates.iter().enumerate() {
                let x = strip * i as u32;
                let fb_prop = props.handle("FB_ID")?;
                req.add_property(*plane, fb_prop, property::Value::Framebuffer(Some(bufs[0].fb)));
                req.add_property(*plane, props.handle("CRTC_ID")?, property::Value::CRTC(Some(crtc)));
                req.add_property(*plane, props.handle("SRC_X")?, property::Value::UnsignedRange((x as u64) << 16));
                req.add_property(*plane, props.handle("SRC_Y")?, property::Value::UnsignedRange(0));
                req.add_property(*plane, props.handle("SRC_W")?, property::Value::UnsignedRange((strip as u64) << 16));
                req.add_property(*plane, props.handle("SRC_H")?, property::Value::UnsignedRange((h as u64) << 16));
                req.add_property(*plane, props.handle("CRTC_X")?, property::Value::SignedRange(x as i64));
                req.add_property(*plane, props.handle("CRTC_Y")?, property::Value::SignedRange(0));
                req.add_property(*plane, props.handle("CRTC_W")?, property::Value::UnsignedRange(strip as u64));
                req.add_property(*plane, props.handle("CRTC_H")?, property::Value::UnsignedRange(h as u64));
                planes.push((*plane, fb_prop));
            }
            // The panel comes up in its preferred mode, then switches to the
            // fastest, as `to_fastest` says why.
            commit(&card, req, AtomicCommitFlags::ALLOW_MODESET).context("set the panel mode")?;
            let fastest = (mode.vrefresh() != preferred.vrefresh()).then_some(blob);

            let ids: Vec<u32> = planes.iter().map(|p| u32::from(p.0)).collect();
            let summary = format!(
                "{:?} connector {} mode {:?} {w}x{h} {} Hz, crtc {}, planes {ids:?} {:?}{}",
                conn.interface(),
                u32::from(conn.handle()),
                mode.name(),
                mode.vrefresh(),
                u32::from(crtc),
                fourcc,
                spr.as_ref().map(|(_, _, name)| format!(", sub-pixel rendering through {name}")).unwrap_or_default()
            );
            let spr = spr.map(|(id, setup, _)| (id, setup));
            let lp = match conn_props.get("LP") {
                Ok((id, _)) => Some((id, enum_value(&card, id, "ON")?, enum_value(&card, id, "LP2")?)),
                Err(_) => None,
            };
            let conn = conn.handle();
            let s = KmsSurface {
                card,
                bufs,
                back: 1,
                flipping: false,
                planes,
                crtc,
                active,
                mode_id,
                preferred: first,
                fastest,
                spr,
                conn,
                lp,
                power: Power::On,
                slow: false,
                w,
                h,
                order,
                summary,
            };
            s.to_fastest()?;
            Ok(s)
        }

        pub fn describe(&self) -> &str {
            &self.summary
        }

        pub fn is_on(&self) -> bool {
            self.power == Power::On
        }

        pub fn is_off(&self) -> bool {
            self.power == Power::Off
        }

        /// Let the panel rest, ready to light again at once: in its
        /// low-power mode, as Android rests it while dozing, with the
        /// display processor stopped. The 13 Ultra's lights from there in
        /// 13 ms, where coming on from off takes 158, most of it the panel
        /// waiting 120 ms after leaving sleep as its own commands say.
        /// Where the display driver has no low-power mode, the display
        /// turns off. A touch controller that follows the panel, as
        /// Xiaomi's do, rests with it either way.
        pub fn rest(&mut self) -> Result<()> {
            if self.power != Power::On {
                return Ok(());
            }
            let Some((id, _, rest)) = self.lp else { return self.turn_off() };
            self.settle()?;
            let mut req = atomic::AtomicModeReq::new();
            req.add_property(self.conn, id, property::Value::UnsignedRange(rest));
            self.card.atomic_commit(AtomicCommitFlags::empty(), req).context("rest the panel")?;
            self.power = Power::Resting;
            Ok(())
        }

        /// Turn the display off: the panel and the display processor feeding
        /// it stop drawing power. The next frame presented turns it on
        /// again.
        pub fn turn_off(&mut self) -> Result<()> {
            if self.power == Power::Off {
                return Ok(());
            }
            self.settle()?;
            let mut req = atomic::AtomicModeReq::new();
            req.add_property(self.crtc, self.active, property::Value::Boolean(false));
            // Qualcomm's display driver sends no event for a commit that
            // turns its CRTC off, so this one is made to return once it is
            // done.
            self.card.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req).context("turn the display off")?;
            self.power = Power::Off;
            Ok(())
        }

        /// Switch a panel that came on again in its preferred mode to its
        /// fastest. It is called once the backlight is on, which need not
        /// wait the frames the switch takes.
        pub fn speed_up(&mut self) -> Result<()> {
            if self.slow {
                self.settle()?;
                self.to_fastest()?;
                self.slow = false;
            }
            Ok(())
        }

        /// The panel, up in its preferred mode, switched to its fastest.
        /// Powered up in the fast mode directly, the display controller runs
        /// at the fast rate but the panel keeps its own: the 13 Ultra's sends
        /// its TE at 60 Hz, and every frame waits two TEs. Switching a
        /// running panel sends it the commands for the new rate, as Android's
        /// switches do.
        fn to_fastest(&self) -> Result<()> {
            let Some(fastest) = self.fastest else { return Ok(()) };
            let mut req = atomic::AtomicModeReq::new();
            req.add_property(self.crtc, self.mode_id, fastest);
            if let Some((id, setup)) = self.spr {
                req.add_property(self.crtc, id, setup);
            }
            commit(&self.card, req, AtomicCommitFlags::ALLOW_MODESET).context("switch the panel to its fastest refresh")
        }
    }

    impl Drop for KmsSurface {
        fn drop(&mut self) {
            for buf in &self.bufs {
                unsafe { libc::munmap(buf.map.cast(), buf.len) };
            }
        }
    }

    impl Surface for KmsSurface {
        fn size(&self) -> (u32, u32) {
            (self.w, self.h)
        }

        /// A command-mode panel is sent a frame from one of its TE signals
        /// through most of the time to the next (6.8 of 8.3 ms on the 13
        /// Ultra at 120 Hz), and the frame's flip comes as the sending
        /// starts. A frame is committed the moment the flip of the one
        /// before comes, to be sent from the next TE, and drawn while that
        /// one is sent: into the buffer of the frame before both, which the
        /// panel finished reading before the frame now being sent started.
        /// A display turned off comes on again with the frame, in the
        /// panel's preferred mode, as it first came up.
        fn present(&mut self, canvas: &Canvas) -> Result<()> {
            if (canvas.w, canvas.h) != (self.w, self.h) {
                return Err(anyhow!("canvas {}x{} does not match the display {}x{}", canvas.w, canvas.h, self.w, self.h));
            }
            let order = self.order;
            let buf = &self.bufs[self.back];
            let pitch = buffer::Buffer::pitch(&buf.db) as usize;
            let row = self.w as usize * 4;
            let dst = unsafe { std::slice::from_raw_parts_mut(buf.map, buf.len) };
            for y in 0..self.h as usize {
                let src = &canvas.data[y * row..(y + 1) * row];
                let out = &mut dst[y * pitch..y * pitch + row];
                match order {
                    Order::Rgba => out.copy_from_slice(src),
                    Order::Bgra => {
                        for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                            *o = [s[2], s[1], s[0], s[3]];
                        }
                    }
                }
            }
            let mut req = atomic::AtomicModeReq::new();
            for &(plane, fb_prop) in &self.planes {
                req.add_property(plane, fb_prop, property::Value::Framebuffer(Some(buf.fb)));
            }
            let mut flags = AtomicCommitFlags::NONBLOCK | AtomicCommitFlags::PAGE_FLIP_EVENT;
            if self.power != Power::On {
                // The panel runs again, and coming on from off it would
                // otherwise rest again as it last did.
                if let Some((id, run, _)) = self.lp {
                    req.add_property(self.conn, id, property::Value::UnsignedRange(run));
                }
            }
            if self.power == Power::Off {
                req.add_property(self.crtc, self.active, property::Value::Boolean(true));
                req.add_property(self.crtc, self.mode_id, self.preferred);
                if let Some((id, setup)) = self.spr {
                    req.add_property(self.crtc, id, setup);
                }
                flags |= AtomicCommitFlags::ALLOW_MODESET;
            }
            self.settle()?;
            self.card.atomic_commit(flags, req).context(if self.power == Power::On { "flip" } else { "turn the display on" })?;
            if self.power == Power::Off {
                self.slow = self.fastest.is_some();
            }
            self.power = Power::On;
            self.flipping = true;
            self.back = (self.back + 1) % self.bufs.len();
            Ok(())
        }

        fn settle(&mut self) -> Result<()> {
            if self.flipping {
                flipped(&self.card).context("flip")?;
                self.flipping = false;
            }
            Ok(())
        }
    }
}
