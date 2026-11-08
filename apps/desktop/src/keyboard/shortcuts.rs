//! OS-wide voice controls. Registration owns no microphone, provider, serial
//! port or application-maintenance resource; edges enter the existing Actor.
use nomifun_keyboard::document::Settings;
use std::{collections::{BTreeMap, HashSet}, sync::{Arc, Mutex}};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action { Toggle, Beautify, Cancel, Dictate }

type Callback = Arc<dyn Fn(Action, bool) + Send + Sync>;

pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new().build()
}

#[derive(Default)]
struct RouteState { action: Option<Action>, pressed: bool }
struct Route { state: Mutex<RouteState>, callback: Callback }
impl Route {
    fn event(&self, pressed: bool) {
        let action = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some(action) = state.action else { return; };
            if state.pressed == pressed { return; }
            state.pressed = pressed;
            action
        };
        (self.callback)(action, pressed);
    }
    fn set_action(&self, action: Option<Action>) {
        let released = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.action == action { return; }
            let released = state.pressed.then_some(state.action).flatten();
            state.pressed = false;
            state.action = action;
            released
        };
        if let Some(action) = released { (self.callback)(action, false); }
    }
    fn action(&self) -> Option<Action> {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).action
    }
}

#[derive(Clone)]
struct Slot { key: Shortcut, route: Arc<Route> }
type Slots = BTreeMap<u32, Slot>;
type Plan = BTreeMap<u32, (Shortcut, Action)>;

fn plan(settings: &Settings) -> Result<Plan, String> {
    let mut result = BTreeMap::new();
    if !settings.enabled { return Ok(result); }
    let mut seen = HashSet::new();
    for (value, action, name) in [
        (&settings.shortcuts.toggle, Action::Toggle, "显示字幕"),
        (&settings.shortcuts.beautify, Action::Beautify, "美化文字"),
        (&settings.shortcuts.cancel, Action::Cancel, "取消"),
        (&settings.shortcuts.dictate, Action::Dictate, "按住说话"),
    ] {
        if value.trim().is_empty() { continue; }
        let key = value.trim().parse::<Shortcut>().map_err(|error| format!("{name}快捷键无效：{error}"))?;
        if !key.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER) {
            return Err(format!("{name}快捷键需要包含 Ctrl、Alt 或 Command/Win 修饰键"));
        }
        if !seen.insert(key.id()) { return Err("语音键盘的快捷键不能重复".into()); }
        result.insert(key.id(), (key, action));
    }
    Ok(result)
}
pub(crate) fn validate(settings: &Settings) -> Result<(), String> { plan(settings).map(|_| ()) }

trait Backend {
    fn register(&self, slot: &Slot) -> Result<(), String>;
    fn unregister(&self, key: Shortcut) -> Result<(), String>;
}
struct NativeBackend(tauri::AppHandle);
impl Backend for NativeBackend {
    fn register(&self, slot: &Slot) -> Result<(), String> {
        let route = slot.route.clone();
        self.0.global_shortcut().on_shortcut(slot.key, move |_, _, event| {
            // Do not call OS registration from this callback: the plugin holds
            // its dispatch lock. Only enqueue a bounded semantic edge.
            route.event(event.state == ShortcutState::Pressed);
        }).map_err(|error| format!("系统快捷键 {} 注册失败，可能已被其他应用占用：{error}", slot.key))
    }
    fn unregister(&self, key: Shortcut) -> Result<(), String> {
        self.0.global_shortcut().unregister(key).map_err(|error| format!("释放系统快捷键 {key} 失败：{error}"))
    }
}

/// Keep existing OS registrations while reserving new combinations. Common
/// combinations retain their native slot, so action swaps do not conflict with
/// ourselves. Failed changes restore the last configuration and report any OS
/// failure to restore it; inactive leftovers remain tracked for later cleanup.
fn reconcile(slots: &mut Slots, desired: Plan, callback: &Callback, backend: &impl Backend) -> Result<(), String> {
    let original = slots.clone();
    let original_actions: BTreeMap<_, _> = slots.iter().map(|(id, slot)| (*id, slot.route.action())).collect();
    let changed = (|| {
        for (id, (key, _)) in &desired {
            if slots.contains_key(id) { continue; }
            let slot = Slot { key: *key, route: Arc::new(Route { state: Mutex::new(RouteState::default()), callback: callback.clone() }) };
            backend.register(&slot)?;
            slots.insert(*id, slot);
        }
        for (id, slot) in &original {
            if desired.contains_key(id) { continue; }
            backend.unregister(slot.key)?;
            slot.route.set_action(None);
            slots.remove(id);
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = changed {
        let mut recovery = Vec::new();
        let additions: Vec<_> = slots.iter().filter(|(id, _)| !original.contains_key(id)).map(|(id, slot)| (*id, slot.clone())).collect();
        for (id, slot) in additions {
            slot.route.set_action(None);
            match backend.unregister(slot.key) {
                Ok(()) => { slots.remove(&id); }
                Err(error) => recovery.push(error),
            }
        }
        for (id, slot) in &original {
            if !slots.contains_key(id) {
                match backend.register(slot) {
                    Ok(()) => { slots.insert(*id, slot.clone()); }
                    Err(error) => recovery.push(error),
                }
            }
            if slots.contains_key(id) { slot.route.set_action(original_actions[id]); }
        }
        return Err(if recovery.is_empty() { error } else { format!("{error}；旧快捷键恢复未完成：{}", recovery.join("；")) });
    }
    for (id, (_, action)) in desired {
        if let Some(slot) = slots.get(&id) { slot.route.set_action(Some(action)); }
    }
    Ok(())
}

pub(crate) struct Controller { backend: NativeBackend, slots: Slots, callback: Callback }
pub(crate) struct RegistrationSnapshot(Plan);
impl Controller {
    pub(crate) fn new(app: tauri::AppHandle, callback: impl Fn(Action, bool) + Send + Sync + 'static) -> Self {
        Self { backend: NativeBackend(app), slots: Slots::new(), callback: Arc::new(callback) }
    }
    pub(crate) fn reconfigure(&mut self, settings: &Settings) -> Result<usize, String> {
        reconcile(&mut self.slots, plan(settings)?, &self.callback, &self.backend)?;
        Ok(self.registered())
    }
    pub(crate) fn registered(&self) -> usize {
        self.slots.values().filter(|slot| slot.route.action().is_some()).count()
    }
    pub(crate) fn checkpoint(&self) -> RegistrationSnapshot {
        RegistrationSnapshot(self.slots.iter().filter_map(|(id, slot)| slot.route.action().map(|action| (*id, (slot.key, action)))).collect())
    }
    pub(crate) fn restore(&mut self, snapshot: RegistrationSnapshot) -> Result<usize, String> {
        if let Err(error) = reconcile(&mut self.slots, snapshot.0, &self.callback, &self.backend) {
            // A saved settings write failed. Never keep the unsaved bindings
            // executable if an external application prevents exact recovery.
            let cleanup = self.shutdown();
            return Err(match cleanup { Ok(()) => error, Err(cleanup) => format!("{error}；{cleanup}") });
        }
        Ok(self.registered())
    }
    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        // Disable callbacks even if the OS refuses an unregister operation.
        for slot in self.slots.values() { slot.route.set_action(None); }
        reconcile(&mut self.slots, Plan::new(), &self.callback, &self.backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashSet};
    #[derive(Default)]
    struct Os { registered: RefCell<HashSet<u32>>, conflicts: RefCell<HashSet<u32>>, refuse_release: RefCell<HashSet<u32>> }
    impl Backend for Os {
        fn register(&self, slot: &Slot) -> Result<(), String> {
            if self.conflicts.borrow().contains(&slot.key.id()) || !self.registered.borrow_mut().insert(slot.key.id()) { return Err("occupied".into()); }
            Ok(())
        }
        fn unregister(&self, key: Shortcut) -> Result<(), String> {
            if self.refuse_release.borrow().contains(&key.id()) { return Err("OS refused release".into()); }
            if !self.registered.borrow_mut().remove(&key.id()) { return Err("not registered".into()); }
            Ok(())
        }
    }
    fn fixture() -> (Slots, Callback, Arc<Mutex<Vec<(Action, bool)>>>, Os) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let collect = events.clone();
        (Slots::new(), Arc::new(move |action, pressed| collect.lock().unwrap().push((action, pressed))), events, Os::default())
    }
    #[test]
    fn shortcut_parse_normalizes_duplicate_aliases_and_rejects_typing_keys() {
        let mut settings = Settings::default();
        assert_eq!(plan(&settings).unwrap().len(), 4);
        settings.shortcuts.toggle = "Ctrl+Shift+A".into();
        settings.shortcuts.dictate = "Control+Shift+KeyA".into();
        assert!(plan(&settings).unwrap_err().contains("重复"));
        settings.shortcuts.dictate = "A".into();
        assert!(plan(&settings).unwrap_err().contains("修饰键"));
        settings.shortcuts.dictate = "Shift+A".into();
        assert!(plan(&settings).unwrap_err().contains("修饰键"));
        settings.shortcuts.dictate.clear();
        settings.enabled = false;
        assert!(plan(&settings).unwrap().is_empty());
    }
    #[test]
    fn external_conflict_keeps_previous_bindings_and_removes_partial_new_reservations() {
        let (mut slots, callback, _, backend) = fixture();
        let mut settings = Settings::default();
        reconcile(&mut slots, plan(&settings).unwrap(), &callback, &backend).unwrap();
        let before = backend.registered.borrow().clone();
        settings.shortcuts.dictate = "Ctrl+Alt+F8".into();
        settings.shortcuts.toggle = "Ctrl+Alt+F9".into();
        backend.conflicts.borrow_mut().insert(settings.shortcuts.toggle.parse::<Shortcut>().unwrap().id());
        assert!(reconcile(&mut slots, plan(&settings).unwrap(), &callback, &backend).is_err());
        assert_eq!(*backend.registered.borrow(), before);
        assert_eq!(slots.len(), 4);
    }
    #[test]
    fn swaps_remap_actions_without_self_conflict_and_hold_edges_are_not_repeated() {
        let (mut slots, callback, events, backend) = fixture();
        let mut settings = Settings::default();
        reconcile(&mut slots, plan(&settings).unwrap(), &callback, &backend).unwrap();
        let old_key = settings.shortcuts.dictate.parse::<Shortcut>().unwrap().id();
        slots[&old_key].route.event(true);
        slots[&old_key].route.event(true);
        std::mem::swap(&mut settings.shortcuts.dictate, &mut settings.shortcuts.toggle);
        reconcile(&mut slots, plan(&settings).unwrap(), &callback, &backend).unwrap();
        slots[&old_key].route.event(false);
        slots[&old_key].route.event(true);
        assert_eq!(*events.lock().unwrap(), vec![(Action::Dictate, true), (Action::Dictate, false), (Action::Toggle, true)]);
        reconcile(&mut slots, Plan::new(), &callback, &backend).unwrap();
        assert!(backend.registered.borrow().is_empty());
        assert!(slots.is_empty());
    }
    #[test]
    fn failed_release_restores_actions_for_already_removed_os_bindings() {
        let (mut slots, callback, events, backend) = fixture();
        let settings = Settings::default();
        reconcile(&mut slots, plan(&settings).unwrap(), &callback, &backend).unwrap();
        let before = backend.registered.borrow().clone();
        let key = *slots.first_key_value().unwrap().0;
        let action = slots[&key].route.action().unwrap();
        slots[&key].route.event(true);
        backend.refuse_release.borrow_mut().insert(*slots.last_key_value().unwrap().0);
        assert!(reconcile(&mut slots, Plan::new(), &callback, &backend).is_err());
        assert_eq!(*backend.registered.borrow(), before);
        assert_eq!(slots[&key].route.action(), Some(action));
        slots[&key].route.event(true);
        assert_eq!(*events.lock().unwrap(), vec![(action, true), (action, false), (action, true)]);
    }
}
