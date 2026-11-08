//! Optional external keyboard ownership. The voice Actor never performs USB
//! discovery, serial teardown or thread joins. Each requested instance fences
//! every event from earlier instances before any blocking work starts.
use super::{Message, OwnedTransport, initialize_transport_events, transport_events};
use nomifun_keyboard::service::Transport;
use std::{path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}};
use tokio::sync::mpsc;

#[derive(Clone)]
pub(super) struct Controller {
    pub owner: Arc<Mutex<Option<OwnedTransport>>>,
    epoch: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    root: PathBuf,
    production: bool,
    tx: mpsc::Sender<Message>,
    urgent: mpsc::UnboundedSender<Message>,
}
impl Controller {
    pub fn new(owner: Arc<Mutex<Option<OwnedTransport>>>, stop: Arc<AtomicBool>, root: PathBuf, production: bool,
        tx: mpsc::Sender<Message>, urgent: mpsc::UnboundedSender<Message>) -> Self {
        Self { owner, epoch: Arc::new(AtomicU64::new(0)), stop, root, production, tx, urgent }
    }
    pub fn epoch(&self) -> u64 { self.epoch.load(Ordering::Acquire) }
    pub fn accepts(&self, epoch: u64) -> bool { epoch == self.epoch() && !self.stop.load(Ordering::Acquire) }
    pub fn fence(&self) -> u64 { self.epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1) }
    pub fn invalidate(&self, epoch: u64) {
        let _ = self.epoch.compare_exchange(epoch, epoch.wrapping_add(1), Ordering::AcqRel, Ordering::Acquire);
    }
    pub fn request(&self, enabled: bool) -> u64 {
        let epoch = self.fence();
        let this = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = this.apply(epoch, enabled, || this.start());
            if this.epoch() == epoch {
                let _ = this.urgent.send(Message::ExternalSettled(epoch, result.map(|_| enabled)));
            }
        });
        epoch
    }
    fn start(&self) -> Result<(Transport, std::sync::mpsc::Receiver<nomifun_keyboard::service::Event>), String> {
        if self.production {
            let directory = nomifun_desktop_maintenance::ipc::runtime_dir().map_err(|error| error.to_string())?;
            Transport::start(directory).map_err(|error| error.to_string())
        } else {
            #[cfg(feature = "native-keyboard-validation")]
            {
                let directory = self.root.join("keyboard-transport");
                std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
                Transport::start_validation(directory).map_err(|error| error.to_string())
            }
            #[cfg(not(feature = "native-keyboard-validation"))]
            Err("isolated external input validation is unavailable".into())
        }
    }
    fn apply(&self, epoch: u64, enabled: bool, start: impl FnOnce() -> Result<(Transport, std::sync::mpsc::Receiver<nomifun_keyboard::service::Event>), String>) -> Result<(), String> {
        let mut owner = self.owner.lock().map_err(|_| "外置键盘所有权锁异常")?;
        // Acquiring a Mutex is not FIFO. In particular, an older disable task
        // must never take and stop the owner created by a later enable task.
        if self.epoch() != epoch || (enabled && self.stop.load(Ordering::Acquire)) { return Ok(()); }
        if let Some(mut previous) = owner.take() { previous.shutdown()?; }
        if !enabled || !self.accepts(epoch) { return Ok(()); }
        let (transport, events) = start()?;
        let mut next = OwnedTransport { transport, bridge: None };
        if !self.accepts(epoch) { return next.shutdown(); }
        let commands = next.transport.commands();
        initialize_transport_events(&events, commands.clone(), epoch, &self.tx)?;
        let tx = self.tx.clone();
        let urgent = self.urgent.clone();
        next.bridge = Some(std::thread::Builder::new().name("nomifun-external-keyboard-events".into())
            .spawn(move || transport_events(events, commands, epoch, tx, urgent)).map_err(|error| error.to_string())?);
        // A request or app shutdown may have arrived while native startup was
        // blocked. Never publish that superseded owner, and join it here.
        if !self.accepts(epoch) { return next.shutdown(); }
        *owner = Some(next);
        Ok(())
    }
}

#[cfg(all(test, feature = "native-keyboard-validation"))]
mod tests {
    use super::*;
    use std::{sync::mpsc as sync_channel, time::Duration};
    fn controller(root: &std::path::Path) -> (Controller, mpsc::Receiver<Message>, mpsc::UnboundedReceiver<Message>) {
        let (tx, data) = mpsc::channel(16); let (urgent, control) = mpsc::unbounded_channel();
        (Controller::new(Arc::new(Mutex::new(None)), Arc::new(AtomicBool::new(false)), root.into(), false, tx, urgent), data, control)
    }
    fn start_virtual(root: &std::path::Path) -> Result<(Transport, std::sync::mpsc::Receiver<nomifun_keyboard::service::Event>), String> {
        let directory = root.join("keyboard-transport");
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        Transport::start_validation(directory).map_err(|error| error.to_string())
    }
    fn lock_held(root: &std::path::Path) -> bool {
        Transport::validation_lock_held(&root.join("keyboard-transport")).unwrap()
    }
    #[test]
    fn disabled_controller_creates_no_runtime_directory_or_owner() {
        let root = tempfile::tempdir().unwrap();
        let (controller, _data, _control) = controller(root.path());
        let epoch = controller.fence();
        controller.apply(epoch, false, || panic!("disabled adapter must not start")).unwrap();
        assert!(controller.owner.lock().unwrap().is_none());
        assert!(!root.path().join("keyboard-transport").exists());
    }
    #[test]
    fn stale_enable_cannot_create_an_owner_after_disable_or_shutdown() {
        let root = tempfile::tempdir().unwrap(); let (controller, _data, _control) = controller(root.path());
        let old = controller.fence(); let current = controller.fence();
        controller.apply(current, false, || unreachable!()).unwrap();
        controller.apply(old, true, || panic!("stale start")).unwrap();
        controller.stop.store(true, Ordering::Release);
        controller.apply(current, true, || panic!("shutdown start")).unwrap();
        assert!(!root.path().join("keyboard-transport").exists());
    }

    #[test]
    fn current_enable_owns_real_transport_until_disable_has_joined_it() {
        let root = tempfile::tempdir().unwrap();
        let (controller, mut data, _control) = controller(root.path());
        let enabled = controller.fence();
        controller.apply(enabled, true, || start_virtual(root.path())).unwrap();
        assert!(controller.owner.lock().unwrap().is_some());
        assert!(lock_held(root.path()));
        assert!(matches!(data.try_recv(), Ok(Message::Connected(epoch, _)) if epoch == enabled));
        assert!(matches!(data.try_recv(), Ok(Message::Device(epoch, nomifun_keyboard::service::Event::Status(_))) if epoch == enabled));
        let disabled = controller.fence();
        controller.apply(disabled, false, || panic!("disable must not start a transport")).unwrap();
        assert!(controller.owner.lock().unwrap().is_none());
        assert!(!lock_held(root.path()), "disable must join the native owner before returning");
    }

    #[test]
    fn blocked_start_is_joined_without_publishing_after_disable_or_shutdown() {
        for shutdown in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (controller, mut data, _control) = controller(root.path());
            let enabled = controller.fence();
            let (entered, waiting) = sync_channel::channel();
            let (release, resume) = sync_channel::channel();
            let worker = controller.clone(); let directory = root.path().to_owned();
            let start = std::thread::spawn(move || worker.apply(enabled, true, || {
                // Native startup may block while the desired instance changes.
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                start_virtual(&directory)
            }));
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            let disabled = if shutdown {
                controller.stop.store(true, Ordering::Release);
                None
            } else { Some(controller.fence()) };
            release.send(()).unwrap();
            start.join().unwrap().unwrap();
            if let Some(disabled) = disabled {
                controller.apply(disabled, false, || panic!("superseding disable must not start")).unwrap();
            }
            assert!(controller.owner.lock().unwrap().is_none());
            assert!(!lock_held(root.path()), "a late successful startup must be stopped and joined");
            assert!(matches!(data.try_recv(), Err(mpsc::error::TryRecvError::Empty)), "stale startup must not publish Connected/Status");
        }
    }

    #[test]
    fn older_disable_waiting_for_mutex_cannot_take_newer_real_owner() {
        let root = tempfile::tempdir().unwrap();
        let (controller, _data, _control) = controller(root.path());
        let old = controller.fence();
        let mut slot = controller.owner.lock().unwrap();
        let (entered, waiting) = sync_channel::channel();
        let delayed = controller.clone();
        let disable = std::thread::spawn(move || {
            entered.send(()).unwrap();
            delayed.apply(old, false, || panic!("old disable must never start"))
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        // Model a later enable acquiring the non-FIFO mutex first and installing
        // its real worker while the older disable is waiting for this mutex.
        let latest = controller.fence();
        let (transport, events) = start_virtual(root.path()).unwrap();
        *slot = Some(OwnedTransport { transport, bridge: None });
        assert!(lock_held(root.path()));
        drop(slot);
        disable.join().unwrap().unwrap();
        assert_eq!(controller.epoch(), latest);
        assert!(controller.owner.lock().unwrap().is_some());
        assert!(lock_held(root.path()), "an old disable cannot stop the newer instance");
        let cleanup = controller.fence();
        controller.apply(cleanup, false, || unreachable!()).unwrap();
        drop(events);
        assert!(!lock_held(root.path()));
    }
}
