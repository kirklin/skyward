//! The page of one entry of the file manager: its name, its permissions,
//! and the editor its text opens in.

use super::*;

/// Why an entry was not renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameRefused {
    /// Another entry in the folder has the name, as the folder compares
    /// names: shared storage takes names in any case.
    Taken,
    /// What went wrong, as the system says it.
    Failed(String),
}

impl App {
    /// How renaming the entry the file page shows went: done, the folder
    /// comes back with the entry under its new name; refused, why.
    pub fn renamed(&mut self, outcome: Result<(), RenameRefused>, now: u64) {
        match outcome {
            Ok(()) => {
                self.leave(true);
                while matches!(self.screen(), Screen::Rename | Screen::FileInfo) {
                    self.stack.pop();
                }
                self.state.file = None;
                self.settle_page();
                self.refresh();
                self.say(self.state.lang.pick(["已重命名", "已重新命名", "Renamed"]), now);
            }
            Err(RenameRefused::Taken) => self.say_taken(now),
            Err(RenameRefused::Failed(why)) => self.report(&why, now),
        }
    }

    /// The entry the file page shows was to have the permission bits
    /// `wanted`: those it has now, or why it has not.
    pub fn mode_set(&mut self, wanted: u32, outcome: Result<u32, String>, now: u64) {
        let l = self.state.lang;
        let bits = match outcome {
            Ok(bits) => bits,
            Err(why) => return self.report(&tr!(l, "修改权限失败：{why}", "修改權限失敗：{why}", "Couldn't change the permissions: {why}"), now),
        };
        let entry = self.state.file.as_mut().expect("the permissions page shows an entry");
        entry.mode = (entry.mode & !0o7777) | bits;
        let name = entry.name.clone();
        if let Ok(entries) = &mut self.state.files.entries
            && let Some(listed) = entries.iter_mut().find(|e| e.name == name)
        {
            listed.mode = (listed.mode & !0o7777) | bits;
        }
        if bits != wanted {
            self.say(l.pick(["该位置不支持修改权限", "這個位置不支援修改權限", "Permissions can't be changed here"]), now);
        }
        self.refresh();
    }

    /// What the file at `path` holds, read to edit it: none if it holds
    /// more than [`editor::MAX_BYTES`]; or why it could not be read. Text
    /// opens on the editor page, wrapped to its card; anything else is
    /// turned down, saying why.
    pub fn text_opened(&mut self, f: &Fonts, path: String, read: Result<Option<Vec<u8>>, String>, now: u64) {
        let l = self.state.lang;
        let bytes = match read {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return self.say(l.pick(["暂不支持编辑超过 2 MB 的文件", "尚不支援編輯超過 2 MB 的檔案", "Files over 2 MB can't be edited"]), now),
            Err(why) => return self.report(&tr!(l, "读取失败：{why}", "讀取失敗：{why}", "Couldn't read it: {why}"), now),
        };
        let Some(text) = String::from_utf8(bytes).ok().filter(|text| !text.contains('\0')) else {
            return self.say(l.pick(["仅支持编辑 UTF-8 编码的文本文件", "僅支援編輯 UTF-8 編碼的文字檔", "Only UTF-8 text files can be edited"]), now);
        };
        let l = editor::layout(&self.theme(), f);
        self.state.editor = Some(Editor::new(path, &text, l.cols as usize, l.rows as usize));
        self.state.layer = Layer::Letters;
        self.unsaved_warned = None;
        self.open(Screen::Editor);
    }

    /// How writing the file the editor page shows went: done, it stands as
    /// saved; or why not.
    pub fn text_saved(&mut self, outcome: Result<(), String>, now: u64) {
        match outcome {
            Ok(()) => {
                self.state.editor.as_mut().expect("the editor page shows a file").saved();
                self.refresh();
                self.say(self.state.lang.pick(["已保存", "已儲存", "Saved"]), now);
            }
            Err(why) => self.report(&tr!(self.state.lang, "保存失败：{why}", "儲存失敗：{why}", "Couldn't save it: {why}"), now),
        }
    }

    /// The name typed for an entry is another entry's in the folder.
    pub(super) fn say_taken(&mut self, now: u64) {
        let name = &self.state.new_name;
        let said = tr!(self.state.lang, "该文件夹中已有\"{name}\"", "這個資料夾中已有「{name}」", "This folder already has “{name}”");
        self.say(&said, now);
    }

    /// Where the entry the file page shows is.
    pub(super) fn entry_path(&self) -> String {
        let entry = self.state.file.as_ref().expect("the file page shows an entry");
        self.state.files.inside(&entry.name)
    }

    /// A key of the keyboard on the editor page: a character or a space
    /// goes in at the cursor, Enter breaks the line there and backspace
    /// takes away what is before it; the layer keys change the keyboard.
    pub(super) fn edit_typed(&mut self, cap: Cap) {
        let layer = self.state.layer;
        let ed = self.state.editor.as_mut().expect("the editor page shows a file");
        match cap {
            Cap::Char(ch) if layer == Layer::Shifted => {
                ed.insert(ch.to_ascii_uppercase());
                self.state.layer = Layer::Letters;
            }
            Cap::Char(ch) => ed.insert(ch),
            Cap::Space => ed.insert(' '),
            Cap::Backspace => ed.backspace(),
            Cap::Enter => ed.newline(),
            Cap::Shift => self.state.layer = if layer == Layer::Shifted { Layer::Letters } else { Layer::Shifted },
            Cap::Show(shown) => self.state.layer = shown,
        }
        self.edits += 1;
        // The keyboard stays as it was while its keys do.
        if self.state.layer == layer {
            self.refresh_text();
        } else {
            self.refresh();
        }
    }

    /// The editor's text or cursor changed: draw again what lies above the
    /// keyboard, which stays as it was.
    pub(super) fn refresh_text(&mut self) {
        let t = self.theme();
        self.refresh_rows(0, (t.m.h as f32 - widgets::keyboard_height(&t)).floor() as u32);
    }

    /// A tap at page `x`, `y` on the editor's card puts the cursor between
    /// the characters nearest it.
    pub(super) fn place_caret(&mut self, f: &Fonts, x: f32, y: f32) {
        let l = editor::layout(&self.theme(), f);
        if !l.card.contains(x, y) {
            return;
        }
        let ed = self.state.editor.as_mut().expect("the editor page shows a file");
        let row = (((y - l.y0) / l.pitch).floor().max(0.0) as usize).min(l.rows as usize - 1);
        ed.place(ed.top() + row, (x - l.x0) / l.cell);
        self.edits += 1;
        self.refresh_text();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

    #[test]
    fn renaming_takes_the_name_typed() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        tap(&mut app, &f, &i, Action::RenameEntry);
        assert_eq!(app.screen(), Screen::Rename);
        assert_eq!(app.state.new_name, "a.zip", "the name it has to start from");
        for _ in 0..4 {
            tap(&mut app, &f, &i, Action::Type(Cap::Backspace));
        }
        type_text(&mut app, &f, &i, "b");
        let rename = Effect::Rename { path: "/sdcard/a.zip".into(), name: "ab".into() };
        assert_eq!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)), vec![rename]);
        app.renamed(Err(RenameRefused::Failed("rename /sdcard/a.zip to ab: No space left on device (os error 28)".into())), 0);
        assert_eq!(app.screen(), Screen::Rename, "a name refused can be typed again");
        app.toast = None;
        app.renamed(Err(RenameRefused::Taken), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("该文件夹中已有\"ab\""), "as the folder compares names");
        app.renamed(Ok(()), 0);
        assert_eq!(app.screen(), Screen::Files, "renamed, the folder shows again");
    }

    #[test]
    fn permissions_switch_a_bit_at_a_time_and_show_what_the_entry_took() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        tap(&mut app, &f, &i, Action::OpenPermissions);
        assert_eq!(app.screen(), Screen::Permissions);
        let bits: Vec<u32> = app.frame(&f, &i).hits.iter().filter_map(|h| if let Action::ModeBit(b) = h.action { Some(b) } else { None }).collect();
        assert_eq!(bits, [0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001, 0o4000, 0o2000, 0o1000]);
        let set = |mode| vec![Effect::SetMode { path: "/sdcard/a.zip".into(), mode }];
        assert_eq!(tap(&mut app, &f, &i, Action::ModeBit(0o002)), set(0o757), "everyone may write");
        app.mode_set(0o757, Ok(0o757), 0);
        assert_eq!(app.state.file.as_ref().unwrap().mode, 0o040_757, "its type stays");
        assert_eq!(tap(&mut app, &f, &i, Action::ModeBit(0o100)), set(0o657), "and back off");
        app.toast = None;
        app.mode_set(0o4757, Ok(0o757), 0);
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("该位置不支持修改权限"), "a filesystem that kept none of it");
        app.mode_set(0o755, Err("give /sdcard/a.zip the permissions 0755: Read-only file system (os error 30)".into()), 0);
        assert!(app.toast.as_ref().is_some_and(|t| t.0.starts_with("修改权限失败：give /sdcard/a.zip")), "{:?}", app.toast);
        assert_eq!(app.state.file.as_ref().unwrap().mode, 0o040_757, "as it last was");
        tap_back(&mut app, &f, &i, 0);
        tap_back(&mut app, &f, &i, 0);
        assert_eq!(app.state.files.entries.as_ref().unwrap()[1].mode, 0o040_757, "the folder knows it too");
    }

    #[test]
    fn a_links_page_offers_no_permissions_of_its_own() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        app.state.file = Some(entry("latest", FileKind::Link { target: "DCIM".into(), folder: true }));
        app.open(Screen::FileInfo);
        assert!(!app.frame(&f, &i).hits.iter().any(|h| h.action == Action::OpenPermissions));
        app.state.file = Some(entry("a.zip", FileKind::File));
        app.refresh();
        assert!(app.frame(&f, &i).hits.iter().any(|h| h.action == Action::OpenPermissions));
    }

    #[test]
    fn a_text_file_opens_to_edit_and_saves_what_was_typed() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Files));
        app.set_files("/data/adb".into(), Ok(vec![entry("run.sh", FileKind::File)]), LISTED);
        assert_eq!(tap(&mut app, &f, &i, Action::OpenEntry(0)), vec![Effect::OpenText("/data/adb/run.sh".into())]);
        app.text_opened(&f, "/data/adb/run.sh".into(), Ok(Some(b"echo hi\n".to_vec())), 0);
        assert_eq!(app.screen(), Screen::Editor);
        let frame = app.frame(&f, &i);
        assert_eq!(frame.button, None, "nothing waits to be saved");
        assert!(frame.caret.is_some());
        let keys: Vec<Action> = frame.hits.iter().map(|h| h.action).filter(|a| matches!(a, Action::TermKey(_))).collect();
        assert_eq!(keys, editor::EDITOR_KEYS.map(Action::TermKey));
        tap(&mut app, &f, &i, Action::Type(Cap::Shift));
        tap(&mut app, &f, &i, Action::Type(Cap::Char('x')));
        assert_eq!(app.state.layer, Layer::Letters, "a capital lasts one letter");
        tap(&mut app, &f, &i, Action::Type(Cap::Space));
        tap(&mut app, &f, &i, Action::TermKey(TermKey::Tab));
        tap(&mut app, &f, &i, Action::Type(Cap::Enter));
        assert_eq!(app.frame(&f, &i).button, Some((widgets::Glyph::Done, Action::SaveText)));
        let save = vec![Effect::SaveText { path: "/data/adb/run.sh".into(), text: "X \t\necho hi\n".into() }];
        assert_eq!(tap_button(&mut app, &f, &i, 0), save);
        app.text_saved(Err("rename /data/adb/.run.sh.0.saving to /data/adb/run.sh: Read-only file system (os error 30)".into()), 0);
        assert!(toast(&app).is_some_and(|t| t.starts_with("保存失败：rename")), "{:?}", app.toast);
        assert_eq!(app.frame(&f, &i).button, Some((widgets::Glyph::Done, Action::SaveText)), "still to save");
        assert_eq!(tap_button(&mut app, &f, &i, 0), save);
        app.text_saved(Ok(()), 0);
        assert_eq!(toast(&app), Some("已保存"));
        assert_eq!(app.frame(&f, &i).button, None);
        tap_back(&mut app, &f, &i, 0);
        assert_eq!(app.screen(), Screen::Files, "a file saved leaves at once");
        assert_eq!(app.state.editor, None);
    }

    #[test]
    fn leaving_the_editor_with_changes_unsaved_takes_going_back_again() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        app.set_files("/tmp".into(), Ok(vec![entry("a.txt", FileKind::File)]), LISTED);
        app.text_opened(&f, "/tmp/a.txt".into(), Ok(Some(b"a".to_vec())), 0);
        tap(&mut app, &f, &i, Action::Type(Cap::Char('b')));
        tap_back(&mut app, &f, &i, 1000);
        assert_eq!(app.screen(), Screen::Editor, "the first back only warns");
        assert_eq!(toast(&app), Some("修改尚未保存，再次返回将放弃修改"));
        tap_back(&mut app, &f, &i, 1000 + TOAST_MS);
        assert_eq!(app.screen(), Screen::Editor, "once the warning is gone, back warns again");
        tap_back(&mut app, &f, &i, 1000 + TOAST_MS + 500);
        assert_eq!(app.screen(), Screen::Files, "back again while it shows leaves");
        assert_eq!(app.state.editor, None, "without the changes");
    }

    #[test]
    fn files_the_editor_does_not_take_are_turned_down_saying_why() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        let refused = [
            (Ok(None), "暂不支持编辑超过 2 MB 的文件"),
            (Ok(Some(vec![0xff, 0xfe, b'a'])), "仅支持编辑 UTF-8 编码的文本文件"),
            (Ok(Some(b"\x7fELF\x02\x01\x01\0".to_vec())), "仅支持编辑 UTF-8 编码的文本文件"),
        ];
        for (read, said) in refused {
            app.toast = None;
            app.text_opened(&f, "/sdcard/a.bin".into(), read, 0);
            assert_eq!(app.screen(), Screen::Files);
            assert_eq!(toast(&app), Some(said));
        }
        app.text_opened(&f, "/sdcard/a.bin".into(), Err("open /sdcard/a.bin: Permission denied (os error 13)".into()), 0);
        assert_eq!(toast(&app), Some("读取失败：open /sdcard/a.bin: Permission denied (os error 13)"));
        assert_eq!(app.state.editor, None);
    }

    #[test]
    fn the_file_page_edits_a_file_and_nothing_else() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        assert_eq!(tap(&mut app, &f, &i, Action::EditEntry), vec![Effect::OpenText("/sdcard/a.zip".into())]);
        for kind in [folder(2), FileKind::Link { target: "a.zip".into(), folder: false }, FileKind::Special] {
            app.state.file = Some(entry("other", kind));
            app.refresh();
            assert!(!app.frame(&f, &i).hits.iter().any(|h| h.action == Action::EditEntry), "{:?}", app.state.file);
        }
    }

    #[test]
    fn the_file_page_shows_a_file_as_saved() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        app.text_opened(&f, "/sdcard/a.zip".into(), Ok(Some(b"x".to_vec())), 0);
        let saved = FileRow { size: 9, modified: LISTED, ..entry("a.zip", FileKind::File) };
        app.set_files("/sdcard".into(), Ok(vec![entry("DCIM", folder(2)), saved.clone()]), LISTED);
        assert_eq!(app.state.file, Some(saved));
    }

    #[test]
    fn a_tap_on_the_text_moves_the_cursor_and_a_drag_or_a_flick_scrolls_it() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Files, W, H, PPD);
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        app.text_opened(&f, "/tmp/long.txt".into(), Ok(Some(text.into_bytes())), 0);
        let (card, pitch) = app.frame(&f, &i).scrollback.expect("the editor's text scrolls");
        let l = editor::layout(&app.theme(), &f);
        let ed = |app: &App| app.state.editor.clone().expect("the editor page shows a file");
        app.touch(Touch::Down(l.x0 + 4.2 * l.cell, l.y0 + 2.5 * l.pitch), &f, 0);
        app.touch(Touch::Up, &f, 0);
        assert_eq!(ed(&app).cursor(), (2, 4), "after \"line\" on the third row");
        let (x, y) = (card.x + card.w / 2.0, card.y + card.h / 2.0);
        app.touch(Touch::Down(x, y), &f, 100);
        app.touch(Touch::Move(x, y - pitch * 5.5), &f, 110);
        app.touch(Touch::Up, &f, 400);
        assert_eq!(ed(&app).top(), 5, "up shows the lines after");
        assert_eq!(ed(&app).cursor(), (2, 4), "a drag leaves the cursor where it was");
        assert_eq!(app.top().1, 0.0, "the page itself stays put");
        tap(&mut app, &f, &i, Action::TermKey(TermKey::Down));
        assert_eq!((ed(&app).cursor(), ed(&app).top()), ((3, 4), 3), "moving the cursor brings it into view");
        app.touch(Touch::Down(x, y), &f, 1000);
        for k in 1..=5 {
            app.touch(Touch::Move(x, y - pitch * k as f32), &f, 1000 + k * 8);
        }
        app.touch(Touch::Up, &f, 1045);
        let lifted = ed(&app).top();
        app.tick(1200);
        let gliding = ed(&app).top();
        assert!(gliding > lifted + 5, "a flick glides on: {lifted} to {gliding}");
        app.touch(Touch::Down(x, y), &f, 1210);
        app.touch(Touch::Up, &f, 1220);
        app.tick(2000);
        assert_eq!(ed(&app).top(), gliding, "a finger on it stops it there");
        assert_eq!(ed(&app).cursor(), (3, 4), "and puts no cursor down");
    }

    #[test]
    fn a_name_the_folder_cannot_take_is_turned_down_before_renaming() {
        let (f, i) = (fonts(), info());
        let mut app = in_sdcard(&f, &i);
        hold_entry(&mut app, &f, &i, 1);
        tap(&mut app, &f, &i, Action::RenameEntry);
        let refused =
            [("", "请输入名称"), ("..", "名称不能为\".\"或\"..\""), ("x/y", "名称不能包含\"/\""), ("DCIM", "该文件夹中已有\"DCIM\"")];
        for (name, said) in refused {
            app.state.new_name = name.into();
            app.toast = None;
            assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty(), "{name:?} is not renamed to");
            assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some(said));
            assert_eq!(app.screen(), Screen::Rename);
        }
        app.state.new_name = "a.zip".into();
        assert!(tap(&mut app, &f, &i, Action::Type(Cap::Enter)).is_empty());
        assert_eq!(app.screen(), Screen::FileInfo, "the name left as it was, the page goes");
    }
}
