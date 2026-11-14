use super::*;
use super::testing::*;
use skyward_info::Charge;

    #[test]
    fn tablet_sidebar_opens_dual_boot_and_stays_there_while_switching() {
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let (f, info) = (fonts(), info());
        let mut app = App::new(Screen::Home, 3048, 2032, 2.5);
        app.configure_device(&cfg);
        app.tick(0);
        let mut canvas = Canvas::new(3048, 2032);
        app.compose(&mut canvas, &f, &info, "12:00", 1000);
        app.state.dual_boot = true;
        app.refresh();
        app.compose(&mut canvas, &f, &info, "12:00", 1100);
        let (x, y) = app.tablet.as_ref().unwrap().section_center(Screen::Systems);
        app.touch(Touch::Down(x, y), &f, 1200);
        let effects = app.touch(Touch::Up, &f, 1210);
        assert_eq!(app.screen(), Screen::Systems);
        assert!(effects.contains(&Effect::ReadSystems));
        app.state.systems_work = Some(SystemsWork::Switching(Who::Other));
        app.compose(&mut canvas, &f, &info, "12:00", 1300);
        let (x, y) = app.tablet.as_ref().unwrap().section_center(Screen::Wipe);
        app.touch(Touch::Down(x, y), &f, 1400);
        assert!(app.touch(Touch::Up, &f, 1410).is_empty());
        assert_eq!(app.screen(), Screen::Systems);
        assert_eq!(app.state.systems_work, Some(SystemsWork::Switching(Who::Other)));
    }

    #[test]
    fn tablet_rotation_can_compose_about_before_the_next_tick() {
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let (f, info) = (fonts(), info());
        let mut app = App::new(Screen::Home, 3048, 2032, 2.5);
        app.configure_device(&cfg);
        app.tick(1000);
        let mut canvas = Canvas::new(3048, 2032);
        app.compose(&mut canvas, &f, &info, "12:00", 1000);
        for (w, h) in [(2032, 3048), (3048, 2032)] {
            app.resize(w, h);
            canvas = Canvas::new(w, h);
            // The runtime needs this snapshot before it starts the turn animation.
            app.compose(&mut canvas, &f, &info, "12:00", 1500);
            assert_eq!(app.screen(), Screen::About);
        }
    }

    #[test]
    fn resizing_a_tablet_keeps_its_page_and_work_in_both_directions() {
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let mut app = App::new(Screen::Installing, 3048, 2032, 2.5);
        app.configure_device(&cfg);
        app.state.code = "123456".into();
        app.state.install = Install::Flashing { partition: "system", stage: None };
        for (width, height) in [(2032, 3048), (3048, 2032), (2032, 3048)] {
            app.resize(width, height);
            assert!(app.tablet_mode);
            assert_eq!(app.w, width - width / 3);
            assert_eq!(app.h, height);
            assert_eq!(app.screen(), Screen::Installing);
            assert_eq!(app.state.code, "123456");
            assert!(matches!(app.state.install, Install::Flashing { partition: "system", .. }));
            app.resize(width, height);
            assert_eq!(app.w, width - width / 3);
        }
    }

    #[test]
    fn rotation_waits_for_release_and_obeys_the_switch_and_fastboot() {
        use crate::rotation::Rotation;
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let mut app = App::new(Screen::Home, 3048, 2032, 2.5);
        app.configure_device(&cfg);
        let f = fonts();
        app.touch(Touch::Down(10.0,10.0), &f, 1000);
        app.follow_rotation(Rotation::Right);
        assert_eq!(app.rotation(), Rotation::Native);
        app.touch(Touch::Up, &f, 1010);
        app.follow_rotation(Rotation::Right);
        assert_eq!(app.rotation(), Rotation::Right);
        app.state.auto_rotate = false;
        app.follow_rotation(Rotation::Left);
        assert_eq!(app.rotation(), Rotation::Right, "off keeps the current direction");
        app.state.auto_rotate = true;
        app.sleep(2000);
        assert!(!app.rotation_enabled());
        app.follow_rotation(Rotation::Left);
        assert_eq!(app.rotation(), Rotation::Right);
        app.display_on = true;
        app.stack.push((Screen::Fastboot, 0.0, Focus::Unset));
        assert!(!app.rotation_enabled());
        assert_eq!(app.rotation(), Rotation::Native);
        app.follow_rotation(Rotation::Left);
        assert_eq!(app.state.rotation, Rotation::Right);
        app.stack.pop();
        assert_eq!(app.rotation(), Rotation::Right);
    }

    #[test]
    fn tablet_fastboot_stays_fullscreen_and_recovery_restores_the_sidebar() {
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let mut app = App::new(Screen::Fastboot, 3048, 2032, 2.5);
        app.configure_device(&cfg);
        assert_eq!(app.w, 3048);
        assert!(app.tablet.is_none());
        assert!(!app.tablet_mode);
        let f = fonts();
        let i = info();
        let mut canvas = Canvas::new(3048, 2032);
        app.tick(1000);
        app.compose(&mut canvas, &f, &i, "12:00", 1000);
        app.enter_recovery();
        app.configure_device(&cfg);
        assert_eq!(app.w, 2032);
        assert!(app.tablet.is_some());
        assert!(app.tablet_mode);
        assert_eq!(app.screen(), Screen::About);
        app.tick(1100);
        app.compose(&mut canvas, &f, &i, "12:00", 1100);
    }

    #[test]
    fn tablet_navigation_stays_beside_details_and_hides_unsupported_vibration() {
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let f = fonts();
        let i = info();
        let mut app = App::new(Screen::Home, 3048, 2032, 2.5);
        app.state.data = Data::Open;
        app.configure_device(&cfg);
        app.configure_device(&cfg);
        assert_eq!(app.w, 2032, "configuration does not shrink the detail pane twice");
        assert_eq!(app.screen(), Screen::About);
        app.tick(0);
        let mut canvas = Canvas::new(3048, 2032);
        app.compose(&mut canvas, &f, &i, "12:00", 0);
        let mut global = app.theme();
        global.m.w = 3048;
        let mut bar = Canvas::filled(3048, 100, global.pal.bg);
        widgets::status_bar(&mut bar, &global, &f, &app.bar("12:00", &i), global.pal.status);
        assert_eq!(canvas.crop(0, 0, 350, 100).data, bar.crop(0, 0, 350, 100).data,
            "the time uses the left edge of the physical display");
        let (x, y) = app.tablet.as_ref().unwrap().section_center(Screen::Settings);
        app.touch(Touch::Down(x, y), &f, 1000);
        app.touch(Touch::Up, &f, 1010);
        assert_eq!(app.screen(), Screen::Settings);
        assert_eq!(app.tablet_selection(), Screen::Settings);
        let frame = app.frame(&f, &i);
        assert!(!frame.back, "the section is selected from the fixed sidebar");
        assert!(frame.hits.iter().all(|h| !matches!(h.action,
            Action::Slider(Slider::Vibration) | Action::Toggle(Switch::Vibrate))));
        let track = frame.hits.iter().find(|h| h.action == Action::Slider(Slider::Brightness)).unwrap().rect;
        app.end_turn();
        app.compose(&mut canvas, &f, &i, "12:00", 1100);
        // Input coordinates include the sidebar offset; the original controls do not.
        let x = 1016.0 + track.x;
        let cy = track.y + track.h/2.0;
        app.touch(Touch::Down(x, cy), &f, 1200);
        app.touch(Touch::Up, &f, 1210);
        assert!((app.state.brightness - 1.0/4095.0).abs() < 0.00001);
        assert!(search::find_available("vibr", Lang::En, |_| true, app.state.supports_vibration).is_empty());
        app.back(1300);
        assert_eq!(app.screen(), Screen::Settings, "back cannot expose the phone home menu");
        app.stack.push((Screen::Installing, 0.0, Focus::Unset));
        app.state.install = Install::Flashing { partition: "recovery", stage: None };
        assert!(app.tablet_section(Screen::Wipe, 1400).is_empty());
        assert_eq!(app.screen(), Screen::Installing, "sidebar navigation cannot leave partition writes");
    }

    #[test]
    fn a_sidebar_touch_cannot_wake_the_tablet_or_open_a_page_just_after_power_wakes_it() {
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let mut app = App::new(Screen::Home, 3048, 2032, 2.5);
        app.configure_device(&cfg);
        app.display_on = false;
        let f = fonts();
        app.touch(Touch::Down(150.0, 1700.0), &f, 1000);
        assert!(!app.display_on, "only the power key wakes the dark panel");
        app.touch(Touch::Up, &f, 5000);
        press(&mut app, Key::Power, 6000);
        assert!(app.display_on);
        app.touch(Touch::Down(150.0, 1700.0), &f, 6010);
        app.touch(Touch::Up, &f, 6020);
        assert_eq!(app.screen(), Screen::About);
    }

    #[test]
    fn tablet_screen_off_fade_covers_the_sidebar_and_detail() {
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Home, 3048, 2032, 2.5);
        app.configure_device(&cfg);
        app.tick(0);
        let mut canvas = Canvas::new(3048, 2032);
        app.compose(&mut canvas, &f, &i, "12:00", 1000);
        assert!(canvas.data.iter().any(|&v| v != 0));
        press(&mut app, Key::Power, 1100);
        app.tick(1465);
        app.compose(&mut canvas, &f, &i, "12:00", 1465);
        assert!(canvas.data.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]),
            "both panes and the full-width status bar fade to black");
    }

    #[test]
    fn tablet_charging_covers_both_panes_and_dismisses_without_selecting_a_section() {
        let cfg = skyward_config::DeviceConfig::parse(include_str!("../../../../devices/sheng.toml")).unwrap();
        let f = fonts();
        let i = DeviceInfo { battery: Some(battery_at(64, Charge::Charging)), ..info() };
        let mut app = App::new(Screen::Home, 3048, 2032, 2.5);
        app.configure_device(&cfg);
        app.tick(0);
        let mut canvas = Canvas::new(3048, 2032);
        app.compose(&mut canvas, &f, &i, "12:00", 1000);
        assert!(app.dither.is_some(), "the About page has a detail-width dither");
        let (x, y) = app.tablet.as_ref().unwrap().section_center(Screen::Settings);
        app.plugged(1100, true);
        app.tick(2000);
        app.compose(&mut canvas, &f, &i, "12:00", 2000);
        let center_foot = ((canvas.h - 4) * canvas.w + canvas.w / 2) as usize * 4;
        assert!(canvas.data[center_foot + 2] > 40, "charging art spans the physical display");
        app.touch(Touch::Down(x, y), &f, 2010);
        app.touch(Touch::Up, &f, 2020);
        assert!(!app.charge_shown());
        assert_eq!(app.screen(), Screen::About, "dismissing the overlay does not press the sidebar beneath");
        app.tick(2400);
        app.compose(&mut canvas, &f, &i, "12:00", 2400);
        assert_eq!(app.w, 2032, "the detail pane keeps its own width after charging");
    }
