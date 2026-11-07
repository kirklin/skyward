use skyward_info::{Battery, Boot, Charge, Cpu, DeviceInfo, Storage};
use skyward_ui::app::{Action, FileKind, FileRow, Key, Screen, Touch};
use skyward_ui::pages::Band;
use skyward_ui::widgets::{self, Cap};
use skyward_ui::{pages, App, Canvas, Ctx, Fonts, Theme};
use skyward_vab::{Slot, SnapshotState};
use std::time::Instant;

// A Xiaomi 13 Ultra panel: 1440 × 3200 at 560 dpi, 120 frames a second.
const W: u32 = 1440;
const H: u32 = 3200;
const PPD: f32 = 3.5;
const FRAME_US: u64 = 8333;
/// What a frame may take to draw, in ms, leaving the rest of the frame for
/// copying it to the panel's buffer and handing it over.
const BUDGET_MS: f64 = 5.5;

/// The moves timed, each from the page it starts on.
const MOVES: [&str; 12] = ["press", "open", "drag", "fold", "fling", "back", "search", "about", "files", "dark", "charge", "renders"];

/// Times what the UI draws: whole pages rendered, and every frame of the
/// moves people make, driven the way the runtime drives the app. Run with
/// the directory holding MiSansVF.ttf and KirkMono-Regular.ttf, then the
/// moves to time (all of them if none is named) and, after `x`, how many
/// times over to make each; `renders` times the pages rendered, and
/// `renders:Settings` one of them.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).expect("the directory holding MiSansVF.ttf and KirkMono-Regular.ttf");
    let f = Fonts::load(format!("{dir}/MiSansVF.ttf"), 0, format!("{dir}/KirkMono-Regular.ttf")).expect("load the fonts");
    let i = info();
    let mut named: Vec<&str> = Vec::new();
    let mut times = 1;
    let mut rest = args[2..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "x" => times = rest.next().and_then(|n| n.parse().ok()).expect("a count after x"),
            a => named.push(a),
        }
    }
    if named.is_empty() {
        named = MOVES.to_vec();
    }
    for name in named {
        for k in 0..times {
            let quiet = k + 1 < times;
            match name.split_once(':') {
                Some(("renders", page)) => renders(&f, &i, Some(page), quiet),
                Some(("hits", page)) => hits(page, &f, &i),
                _ => run(name, &f, &i, quiet),
            }
        }
    }
}

fn run(name: &str, f: &Fonts, i: &DeviceInfo, quiet: bool) {
    match name {
        "renders" => renders(f, i, None, quiet),
        "press" => {
            let mut p = Panel::new(Screen::Home, f, i, quiet);
            let (x, y) = p.at(Action::Open(Screen::Settings));
            p.touch(Touch::Down(x, y));
            p.run(400);
            p.report("a row held down");
        }
        "open" => {
            let mut p = Panel::new(Screen::Home, f, i, quiet);
            p.tap(Action::Open(Screen::Settings));
            p.run(1000);
            p.report("the settings page sliding in");
        }
        "drag" => {
            let mut p = Panel::new(Screen::Settings, f, i, quiet);
            p.drag(2400.0, 1900.0, 1000);
            p.report("the settings page dragged slowly");
        }
        "fold" => {
            let mut p = Panel::new(Screen::Home, f, i, quiet);
            p.drag(2400.0, 2260.0, 600);
            p.run(800);
            p.report("the home page's search bar folding, let go past halfway");
        }
        "fling" => {
            let mut p = Panel::new(Screen::Settings, f, i, quiet);
            p.fling(2600.0, 1200.0);
            p.run(2500);
            p.report("the settings page flung");
        }
        "back" => {
            let mut p = Panel::new(Screen::Home, f, i, quiet);
            p.tap(Action::Open(Screen::Settings));
            p.run(1000);
            p.skip();
            let (x, y) = widgets::back_rect(&p.app.theme()).centre();
            p.touch(Touch::Down(x, y));
            p.run(100);
            p.report("`<` held down");
            p.touch(Touch::Up);
            p.run(1000);
            p.report("the settings page sliding off");
        }
        "search" => {
            let mut p = Panel::new(Screen::Home, f, i, quiet);
            p.tap(Action::Open(Screen::Search));
            p.run(800);
            p.report("the search bar rising into the search page");
            for c in ['w', 'i', 'p', 'e'] {
                p.tap(Action::Type(Cap::Char(c)));
                p.run(150);
            }
            p.report("typing a search");
            p.tap(Action::CloseSearch);
            p.run(800);
            p.report("the search bar going back down");
        }
        "about" => {
            let mut p = Panel::new(Screen::Settings, f, i, quiet);
            p.tap(Action::Open(Screen::About));
            p.run(1000);
            p.report("the About page sliding in");
            p.run(2000);
            p.report("the About page's field moving");
        }
        "files" => {
            let mut p = Panel::new(Screen::Files, f, i, quiet);
            let many = (0..5000).map(|k| FileRow {
                name: format!("IMG_{k:05}.jpg"),
                kind: FileKind::File,
                size: 4_200_000,
                modified: 1_792_348_260,
                mode: 0o100_644,
                uid: 1023,
                gid: 1023,
            });
            p.app.set_files("/sdcard/DCIM/Camera".into(), Ok(many.collect()), 1_792_348_260 + 86_400);
            p.run(100);
            p.skip();
            p.fling(2600.0, 600.0);
            p.run(3000);
            p.report("a folder of 5000 photos flung");
        }
        "dark" => {
            let mut p = Panel::new(Screen::Settings, f, i, quiet);
            p.press(Key::Power);
            p.run(600);
            p.report("the settings page going dark as the panel goes off");
            p.press(Key::Power);
            p.run(600);
            p.report("the settings page coming up as the panel lights");
        }
        "charge" => {
            let mut p = Panel::new(Screen::Settings, f, i, quiet);
            let now = p.now();
            p.app.plugged(now, true);
            p.run(500);
            p.report("the charging screen coming over the settings page");
            p.run(3000);
            p.report("the charging screen's tide");
            p.run(1000);
            p.report("the charging screen going for the page");
        }
        _ => panic!("no move named {name}; the moves are {MOVES:?}"),
    }
}

/// Where on the unscrolled page named `page` each control is, for driving
/// the panel itself: its centre and what it does.
fn hits(page: &str, f: &Fonts, i: &DeviceInfo) {
    let screens = [Screen::Home, Screen::Settings, Screen::Advanced, Screen::About, Screen::Search];
    let screen = screens.into_iter().find(|s| format!("{s:?}") == page).unwrap_or_else(|| panic!("no page {page} among {screens:?}"));
    let mut p = Panel::new(screen, f, i, true);
    let (x, y) = widgets::back_rect(&p.app.theme()).centre();
    println!("{x:.0} {y:.0} Back");
    for h in &p.app.frame(f, i).hits {
        let (x, y) = h.rect.centre();
        println!("{:.0} {:.0} {:?}", x, y, h.action);
    }
}

/// How long each page takes to render whole and its first screen, in the
/// theme and held down; only the page named `only`, when one is.
fn renders(f: &Fonts, i: &DeviceInfo, only: Option<&str>, quiet: bool) {
    let state = skyward_ui::app::State::default();
    let screens = [Screen::Home, Screen::Settings, Screen::Advanced, Screen::About, Screen::Search, Screen::Install, Screen::Reboot, Screen::Wipe, Screen::Backup, Screen::Specs];
    for t in [Theme::light(W, H, PPD), Theme::light(W, H, PPD).pressed()] {
        for screen in screens.into_iter().filter(|s| only.is_none_or(|o| format!("{s:?}") == o)) {
            let time = |band| {
                let cx = Ctx { t: &t, f, info: i, state: &state, term: None, band };
                let mut ms: Vec<f64> = (0..5)
                    .map(|_| {
                        let t0 = Instant::now();
                        std::hint::black_box(pages::render(screen, &cx));
                        t0.elapsed().as_secs_f64() * 1000.0
                    })
                    .collect();
                ms.sort_by(f64::total_cmp);
                (ms[2], ms[4])
            };
            let band = |rows| Some(Band { rows, cols: (0, W) });
            let (whole, first, laid) = (time(None), time(band((0, H))), time(band((0, 0))));
            if !quiet {
                let held = if t.pal.card == Theme::light(W, H, PPD).pal.card { "" } else { " held down" };
                println!(
                    "render {screen:?}{held}: {:.1} ms (slowest {:.1}), its first screen {:.1} ms (slowest {:.1}), laid out only {:.2} ms",
                    whole.0, whole.1, first.0, first.1, laid.0
                );
            }
        }
    }
}

/// The app on a 120 Hz panel: time passes a frame at a time, and a frame is
/// drawn whenever the app wants one, timed.
struct Panel<'a> {
    app: App,
    f: &'a Fonts,
    i: &'a DeviceInfo,
    out: Canvas,
    us: u64,
    ms: Vec<f64>,
    quiet: bool,
}

impl<'a> Panel<'a> {
    /// The app showing `screen`, settled, nothing timed yet.
    fn new(screen: Screen, f: &'a Fonts, i: &'a DeviceInfo, quiet: bool) -> Panel<'a> {
        let mut app = App::new(screen, W, H, PPD);
        app.set_screen_corners(140.0);
        let mut p = Panel { app, f, i, out: Canvas::new(W, H), us: 0, ms: Vec::new(), quiet };
        p.run(100);
        p.skip();
        p
    }

    fn now(&self) -> u64 {
        self.us / 1000
    }

    /// Let `ms` milliseconds pass, frame by frame.
    fn run(&mut self, ms: u64) {
        let end = self.us + ms * 1000;
        while self.us < end {
            self.frame();
        }
    }

    fn frame(&mut self) {
        self.us += FRAME_US;
        let now = self.now();
        self.app.tick(now);
        if self.app.take_dirty() {
            let t0 = Instant::now();
            self.app.compose(&mut self.out, self.f, self.i, "12:00", now);
            self.ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }

    fn touch(&mut self, ev: Touch) {
        let now = self.now();
        self.app.touch(ev, self.f, now);
    }

    /// A quick press of `key`.
    fn press(&mut self, key: Key) {
        let now = self.now();
        self.app.key(key, now);
        self.frame();
        let now = self.now();
        self.app.key_up(key, now);
    }

    /// Where the page's control doing `action` is on screen.
    fn at(&mut self, action: Action) -> (f32, f32) {
        let frame = self.app.frame(self.f, self.i);
        frame.hits.iter().find(|h| h.action == action).unwrap_or_else(|| panic!("{action:?} is on the page")).rect.centre()
    }

    /// A quick tap on the control doing `action`.
    fn tap(&mut self, action: Action) {
        let (x, y) = self.at(action);
        self.touch(Touch::Down(x, y));
        self.frame();
        self.touch(Touch::Up);
    }

    /// A finger dragging from `y0` to `y1` over `ms`, resting before it
    /// lifts so the page stays.
    fn drag(&mut self, y0: f32, y1: f32, ms: u64) {
        self.touch(Touch::Down(700.0, y0));
        let steps = ms * 1000 / FRAME_US;
        for k in 1..=steps {
            self.touch(Touch::Move(700.0 + (k % 2) as f32, y0 + (y1 - y0) * k as f32 / steps as f32));
            self.frame();
        }
        self.run(120);
        self.touch(Touch::Up);
        self.run(50);
    }

    /// A finger flicking from `y0` to `y1` in a tenth of a second.
    fn fling(&mut self, y0: f32, y1: f32) {
        self.touch(Touch::Down(700.0, y0));
        for k in 1..=12u64 {
            self.touch(Touch::Move(700.0 + (k % 2) as f32, y0 + (y1 - y0) * k as f32 / 12.0));
            self.frame();
        }
        self.touch(Touch::Up);
    }

    fn skip(&mut self) {
        self.ms.clear();
    }

    /// Print how long the frames drawn since the last report took.
    fn report(&mut self, what: &str) {
        let mut ms = std::mem::take(&mut self.ms);
        if self.quiet {
            return;
        }
        if ms.is_empty() {
            println!("{what}: no frames");
            return;
        }
        let over = ms.iter().filter(|&&m| m > BUDGET_MS).count();
        let worst = (0..ms.len()).max_by(|&a, &b| ms[a].total_cmp(&ms[b])).expect("frames were drawn") + 1;
        ms.sort_by(f64::total_cmp);
        let at = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
        println!(
            "{what}: {} frames, median {:.2} ms, 90% {:.2}, slowest {:.2} (frame {worst}), {over} over {BUDGET_MS} ms",
            ms.len(),
            at(0.5),
            at(0.9),
            at(1.0)
        );
    }
}

fn info() -> DeviceInfo {
    DeviceInfo {
        model: "Xiaomi 13 Ultra".into(),
        codename: "ishtar".into(),
        soc: None,
        cpu: Cpu { cores: 8, max_khz: Some(3_187_200) },
        ram_kib: 15_556_972,
        panel: Some((W, H)),
        screen: Some(6.73),
        os_version: None,
        android: None,
        locale: None,
        security_patch: None,
        kernel: "5.15.178".into(),
        storage: Storage { total: 496_550_785_024, free: None },
        // A battery with a temperature and an A/B phone, as the status bar
        // shows them on every frame.
        battery: Some(Battery {
            level: 64,
            charge: Charge::Charging,
            temp: Some(352),
            health: None,
            cycles: None,
            design: None,
            full: None,
            voltage: None,
            quick: false,
            plugged: true,
        }),
        slot: Some(Slot::B),
        snapshot: SnapshotState::None,
        boot: Boot { unlocked: None, verified: None },
    }
}
