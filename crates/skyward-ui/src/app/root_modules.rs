use super::*;

impl State {
    fn listed_root_modules(&self) -> Option<&[RootModule]> {
        self.root_modules.as_ref()?.as_ref().ok().map(Vec::as_slice)
    }

    pub(crate) fn selected_root_module(&self) -> Option<&RootModule> {
        let id = self.root_module_id.as_ref()?;
        self.listed_root_modules()?.iter().find(|module| &module.id == id)
    }
}

pub(crate) fn root_changes(module: &RootModule) -> Vec<RootChange> {
    if !module.editable {
        return Vec::new();
    }
    if module.remove {
        return vec![RootChange::UndoRemove];
    }
    vec![if module.disabled { RootChange::Enable } else { RootChange::Disable }, RootChange::Remove]
}

impl App {
    pub fn set_root_modules(&mut self, listed: Result<Vec<RootModule>, String>) {
        self.state.root_modules = Some(listed);
        self.state.root_change = None;
        self.refresh();
    }

    pub fn root_module_changed(&mut self, outcome: Result<(), String>, now: u64) {
        let message = match &outcome {
            Ok(()) => self.state.lang.pick(texts::common::DONE),
            Err(why) => why,
        };
        self.report(message, now);
    }

    pub(super) fn open_root_module(&mut self, at: usize) {
        if self.state.data.storage_open() && let Some(module) = self.state.listed_root_modules().and_then(|modules| modules.get(at)) {
            self.state.root_module_id = Some(module.id.clone());
            self.state.root_change = None;
            self.open(Screen::RootModule);
        }
    }

    pub(super) fn choose_root_change(&mut self, change: RootChange) {
        if self.state.data.storage_open() && self.state.selected_root_module().is_some_and(|module| root_changes(module).contains(&change)) {
            self.state.root_change = Some(change);
            self.refresh();
        }
    }

    pub(super) fn confirm_root_change(&mut self) -> Vec<Effect> {
        let Some(change) = self.state.root_change.take() else { return Vec::new() };
        if self.state.data.storage_open() && let Some(module) = self.state.selected_root_module().filter(|module| root_changes(module).contains(&change)) {
            return vec![Effect::SetRootModule { id: module.id.clone(), change }];
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

    fn demo(editable: bool) -> RootModule {
        RootModule { id: "demo".into(), name: "Demo".into(), disabled: false, remove: false, editable }
    }

    #[test]
    fn modifying_a_module_requires_open_data_and_a_swipe() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::RootModules, W, H, PPD);
        assert!(app.arrive().is_empty());
        app.set_root_modules(Ok(vec![demo(true)]));
        assert!(app.act(Action::OpenRootModule(0), 0).is_empty());
        assert_eq!(app.screen(), Screen::RootModules);
        app.data_waiting(None);
        assert_eq!(app.arrive(), vec![Effect::ReadRootModules]);

        app.act(Action::OpenRootModule(0), 0);
        assert_eq!(app.screen(), Screen::RootModule);
        assert!(tap(&mut app, &f, &i, Action::ChooseRootChange(RootChange::Remove)).is_empty());
        assert_eq!(swipe(&mut app, &f, &i, Confirm::RootModule), vec![Effect::SetRootModule { id: "demo".into(), change: RootChange::Remove }]);
    }

    #[test]
    fn modules_with_unhandled_boot_state_cannot_be_changed() {
        let mut app = App::new(Screen::RootModules, W, H, PPD);
        app.data_waiting(None);
        app.set_root_modules(Ok(vec![demo(false)]));
        app.act(Action::OpenRootModule(0), 0);
        app.act(Action::ChooseRootChange(RootChange::Disable), 0);
        assert!(app.confirmed(Confirm::RootModule, 0).is_empty());
    }
}
