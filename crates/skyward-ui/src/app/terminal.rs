//! The terminal page: its shell's screen, and the keys that go to it.

use super::*;

/// How the terminal page's shell ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermEnd {
    /// It exited with this code.
    Exited(i32),
    /// This signal stopped it.
    Signalled(i32),
    /// Neither, as the system says it.
    Other(String),
}

impl App {
    /// How many rows and columns the terminal's screen holds.
    pub fn term_size(&self, f: &Fonts) -> (u16, u16) {
        let t = self.theme();
        let l = term::layout(&t, f, t.m.content_top);
        (l.rows, l.cols)
    }

    /// A shell has started for the terminal page, on a blank screen of
    /// `rows` by `cols`.
    pub fn term_started(&mut self, rows: u16, cols: u16) {
        self.term = Some(Term::new(rows, cols));
        self.refresh();
    }

    /// What the shell wrote.
    pub fn term_output(&mut self, bytes: &[u8]) {
        let term = self.term.as_mut().expect("only a shell that started writes");
        term.feed(bytes);
        if self.screen() == Screen::Terminal {
            self.refresh();
        }
    }

    /// The shell has ended as `end` says; its screen says so, and that
    /// Enter starts another.
    pub fn term_ended(&mut self, end: TermEnd) {
        let (l, term) = (self.state.lang, self.term.as_mut().expect("only a shell that started ends"));
        let how = fmt::term_end(l, &end);
        term.feed(tr!(l, "\r\n[{how}。按「回车」重新启动]\r\n", "\r\n[{how}。按「Enter」重新啟動]\r\n", "\r\n[{how}. Press Enter to start it again]\r\n").as_bytes());
        term.running = false;
        if self.screen() == Screen::Terminal {
            self.refresh();
        }
    }

    /// A key of the keyboard on the terminal page. The layer keys change
    /// the keyboard; every other key goes to the shell, and once the shell
    /// has ended, Enter starts another.
    pub(super) fn term_typed(&mut self, cap: Cap) -> Vec<Effect> {
        match cap {
            Cap::Shift => self.state.layer = if self.state.layer == Layer::Shifted { Layer::Letters } else { Layer::Shifted },
            Cap::Show(layer) => self.state.layer = layer,
            _ => {}
        }
        let shifted = self.state.layer == Layer::Shifted;
        if shifted && matches!(cap, Cap::Char(_)) {
            self.state.layer = Layer::Letters;
        }
        self.refresh();
        let Some(term) = self.term.as_mut() else { return Vec::new() };
        if !term.running {
            return if cap == Cap::Enter { vec![Effect::StartShell(self.shell_folder.clone())] } else { Vec::new() };
        }
        let bytes = term.typed(cap, shifted);
        if bytes.is_empty() {
            Vec::new()
        } else {
            vec![Effect::TermInput(bytes)]
        }
    }

    /// Scroll the terminal's screen or the editor's text as a finger goes
    /// from `last` to `y`: a row for each row's height.
    pub(super) fn scroll_history(&mut self, last: f32, y: f32, carry: f32, pitch: f32) -> Drag {
        let carry = carry + (y - last);
        let rows = (carry / pitch).trunc();
        if rows != 0.0 {
            self.scroll_rows(rows as isize);
        }
        Drag::History { last: y, carry: carry - rows * pitch, pitch }
    }

    /// Scroll as a finger going down `rows` rows' heights does, or up for
    /// fewer than none: the terminal's screen back to what the shell wrote
    /// before, the editor's text to the lines before.
    pub(super) fn scroll_rows(&mut self, rows: isize) {
        if self.screen() == Screen::Editor {
            self.state.editor.as_mut().expect("the editor page shows a file").scroll(-rows);
            self.refresh_text();
        } else if let Some(term) = self.term.as_mut() {
            term.scroll(rows);
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

    #[test]
    fn the_terminal_sends_every_key_to_its_shell() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Settings, W, H, PPD);
        assert_eq!(tap(&mut app, &f, &i, Action::Open(Screen::Terminal)), vec![Effect::StartShell("/".into())], "the page asks for a shell the first time");
        let (rows, cols) = app.term_size(&f);
        assert!(rows >= 20 && cols >= 40, "a screen of {rows} rows by {cols}");
        app.term_started(rows, cols);
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Char('l'))), vec![Effect::TermInput(b"l".to_vec())]);
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)), vec![Effect::TermInput(b"\r".to_vec())]);
        assert!(tap(&mut app, &f, &i, Action::TermKey(TermKey::Ctrl)).is_empty());
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Char('c'))), vec![Effect::TermInput(vec![0x03])]);
        assert_eq!(tap(&mut app, &f, &i, Action::TermKey(TermKey::Up)), vec![Effect::TermInput(b"\x1b[A".to_vec())]);
        app.term_output(b"ishtar:/ # ");
        tap_back(&mut app, &f, &i, 0);
        assert!(tap(&mut app, &f, &i, Action::Open(Screen::Terminal)).is_empty(), "the same shell goes on");
        app.term_ended(TermEnd::Exited(0));
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Char('l'))).is_empty(), "nothing runs to take the keys");
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)), vec![Effect::StartShell("/".into())]);
    }

    #[test]
    fn the_file_managers_button_opens_the_terminal_in_the_folder_shown() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        app.set_files("/sdcard/Download".into(), Ok(vec![entry("a.txt", FileKind::File)]), 0);
        assert_eq!(app.frame(&f, &i).button, Some((widgets::Glyph::Terminal, Action::TerminalHere)));
        assert_eq!(tap_button(&mut app, &f, &i, 0), vec![Effect::StartShell("/sdcard/Download".into())], "a shell starts there");
        assert_eq!(app.screen(), Screen::Terminal);
        let (rows, cols) = app.term_size(&f);
        app.term_started(rows, cols);
        app.term_ended(TermEnd::Exited(0));
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)), vec![Effect::StartShell("/sdcard/Download".into())], "Enter starts it again there");
        app.term_started(rows, cols);
        tap_back(&mut app, &f, &i, 0);
        app.set_files("/data/adb".into(), Ok(Vec::new()), 0);
        assert_eq!(tap_button(&mut app, &f, &i, 0), vec![Effect::ShellTo("/data/adb".into())], "the shell running goes there");
        assert_eq!(app.screen(), Screen::Terminal);
    }

    #[test]
    fn the_bars_round_button_darkens_under_a_finger_on_the_frames_drawn() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        app.set_files("/data/adb".into(), Ok(vec![entry("a.txt", FileKind::File)]), LISTED);
        let t = app.theme();
        let (cx, cy, _) = widgets::button_disc(&t);
        let k = (((cy - t.m.dp(5.0)) as u32 * W + (cx + t.m.dp(5.0)) as u32) * 4) as usize;
        let mut c = Canvas::new(W, H);
        let mut pixel = |app: &mut App, now: u64| {
            app.tick(now);
            app.compose(&mut c, &f, &i, "12:00", now);
            c.data[k..k + 4].to_vec()
        };
        let (x, y) = widgets::button_rect(&t).centre();
        let rest = pixel(&mut app, 0);
        app.touch(Touch::Down(x, y), &f, 100);
        assert_ne!(pixel(&mut app, 100), rest, "held down, its disc darkens");
        assert_eq!(app.touch(Touch::Up, &f, 150), vec![Effect::StartShell("/data/adb".into())]);
        for now in (150..1000).step_by(8) {
            pixel(&mut app, now);
        }
        assert_eq!(app.screen(), Screen::Terminal, "the terminal slides in");
        app.text_opened(&f, "/data/adb/a.txt".into(), Ok(Some(b"a".to_vec())), 1000);
        app.state.editor.as_mut().expect("the editor page shows a file").insert('b');
        app.refresh();
        for now in (1000..1500).step_by(8) {
            pixel(&mut app, now);
        }
        app.touch(Touch::Down(x, y), &f, 1500);
        pixel(&mut app, 1500);
        assert_eq!(app.touch(Touch::Up, &f, 1550), vec![Effect::SaveText { path: "/data/adb/a.txt".into(), text: "ba".into() }]);
    }

    #[test]
    fn a_drag_on_the_terminal_scrolls_back_through_what_the_shell_wrote() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Terminal, W, H, PPD);
        let (rows, cols) = app.term_size(&f);
        app.term_started(rows, cols);
        let many: String = (0..100).map(|n| format!("line {n}\r\n")).collect();
        app.term_output(many.as_bytes());
        let (card, pitch) = app.frame(&f, &i).scrollback.expect("the terminal's screen scrolls back");
        let (x, y) = (card.x + card.w / 2.0, card.y + card.h / 3.0);
        app.touch(Touch::Down(x, y), &f, 0);
        app.touch(Touch::Move(x, y + pitch * 5.5), &f, 10);
        app.touch(Touch::Up, &f, 20);
        assert_eq!(app.term.as_ref().expect("the shell started").screen().scrollback(), 5, "down shows five older rows");
        assert_eq!(app.top().1, 0.0, "the page itself stays put");
    }
}
