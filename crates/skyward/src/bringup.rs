//! Hardware bring-up only. No installer, decryption, snapshot mapping,
//! settings persistence, OTA requests, MTP or dual-system worker is started.
//! Root ADB remains a developer interface, not a read-only security boundary.
use crate::input::{Event, Inputs};
use anyhow::{ensure, Result};
use skyward_config::{ramdisk, DeviceConfig};
use skyward_sys::{bcb, Power};
use skyward_ui::app::{Key, Touch};
use skyward_ui::surface::kms::KmsSurface;
use skyward_ui::{Canvas, Fonts, Rgba, Surface};
use std::path::Path;
use std::time::Duration;

pub fn run(cfg: &DeviceConfig) -> Result<()> {
    ensure!(cfg.device.codename == "warhol", "bring-up image is for warhol only");
    skyward_sys::kmsg("warhol bring-up: pending recovery requests ignored; no data opened")?;
    let root = Path::new("/");
    let fonts = Fonts::load(root.join(ramdisk::FONT), 0, root.join(ramdisk::MONO))?;
    let mut surface = KmsSurface::open("/dev/dri/card0")?;
    eprintln!("bring-up display: {}", surface.describe());
    let (w, h) = surface.size();
    // Do not guess touchscreen control writes or start vendor HAL services.
    let mut input = Inputs::open(w, h, Vec::new())?;
    let mut canvas = Canvas::new(w, h);
    let scale = w as f32 / 400.0;
    let mut selected = 0;
    let mut finger = None;
    let mut count = 0u64;
    let mut dirty = true;
    let mut had_panel = input.has_panel();
    // Stock init sets the panel up. Only raise a zero backlight, retaining
    // an existing level and avoiding any Qualcomm-specific controls.
    if let Ok(dirs) = std::fs::read_dir("/sys/class/backlight") {
        for entry in dirs.flatten() {
            let p = entry.path();
            let read = |n| std::fs::read_to_string(p.join(n)).ok()?.trim().parse::<u32>().ok();
            if let (Some(0), Some(max)) = (read("brightness"), read("max_brightness")) {
                let _ = std::fs::write(p.join("brightness"), (max / 5).max(1).to_string());
            }
        }
    }
    loop {
        for event in input.read()? {
            dirty = true;
            match event {
                Event::Touch(Touch::Down(x, y) | Touch::Move(x, y), _) => {
                    finger = Some((x, y));
                    count += 1;
                }
                Event::Touch(Touch::Up, _) => finger = None,
                Event::Key(Key::VolumeUp | Key::VolumeDown, true) => selected ^= 1,
                Event::Key(Key::Power, true) => {
                    // Only an explicit physical-key action clears misc.
                    // Never execute the pending request recorded there.
                    bcb::clear(&bcb::misc())?;
                    skyward_sys::power(if selected == 0 { Power::System } else { Power::Bootloader })?;
                }
                _ => {}
            }
        }
        if input.has_panel() != had_panel {
            had_panel = input.has_panel();
            dirty = true;
        }
        if dirty {
            canvas.clear(Rgba::rgb(17, 22, 30));
            let lines = [
                "Skyward / warhol".to_string(),
                "硬件适配测试版 · 未完成真机验证".to_string(),
                format!("屏幕：{w} × {h}"),
                format!("触摸设备：{}", if had_panel { "已发现" } else { "未发现" }),
                format!("触摸事件：{count}"),
                "在屏幕上划动，检查触点位置。".to_string(),
                "音量键选择，电源键确认重启。".to_string(),
                "此页面不提供刷写、清除或解密。".to_string(),
                "ADB 已开启；日志仅保存在内存。".to_string(),
            ];
            for (i, line) in lines.iter().enumerate() {
                canvas.text(&fonts.body, line, 24.0 * scale, (52.0 + i as f32 * 33.0) * scale,
                    if i == 0 { 25.0 } else { 15.0 } * scale,
                    Rgba::rgb(225, 233, 243), Some(w as f32 - 48.0 * scale));
            }
            for (i, label) in ["重启到当前系统", "重启到 Bootloader"].iter().enumerate() {
                let y = h as f32 - (145.0 - i as f32 * 65.0) * scale;
                canvas.fill_round_rect(24.0 * scale, y, w as f32 - 48.0 * scale, 52.0 * scale, 12.0 * scale,
                    if i == selected { Rgba::rgb(48, 103, 173) } else { Rgba::rgb(38, 46, 58) });
                canvas.text(&fonts.body, label, 40.0 * scale, y + 33.0 * scale, 17.0 * scale,
                    Rgba::rgb(240, 245, 252), None);
            }
            if let Some((x, y)) = finger {
                canvas.fill_circle(x, y, 9.0 * scale, Rgba::rgb(80, 220, 170));
            }
            surface.present(&canvas)?;
            dirty = false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
