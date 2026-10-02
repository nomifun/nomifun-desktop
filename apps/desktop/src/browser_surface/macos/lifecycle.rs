//! Production CEF bundle discovery and process-wide lifetime.
//!
//! A development binary outside a macOS application bundle is intentionally
//! unavailable. The native fixture has its own explicit bundle contract; the
//! product host accepts only the packaged framework/helper layout below.

use std::path::Path;
use std::sync::{Arc, Mutex};

use nomifun_browser_macos::engine::{Engine, Paths};

const FRAMEWORK_NAME: &str = "Chromium Embedded Framework.framework";
const HELPER_NAME: &str = "NomiFun Helper";

pub(crate) fn prepare(data_dir: &Path) -> Result<Arc<DeferredEngine>, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "macOS CEF executable path is unavailable".to_owned())?;
    let mut paths = packaged_paths(&executable, data_dir)?;
    // Resolve the same owned bundle paths without loading CEF, starting its
    // thread pool or asking the system Keychain on a command-only startup.
    paths.framework = paths.framework.canonicalize().map_err(|_| "CEF framework is missing")?;
    paths.helper = paths.helper.canonicalize().map_err(|_| "CEF helper is missing")?;
    paths.main_bundle = paths.main_bundle.canonicalize().map_err(|_| "CEF main bundle is missing")?;
    if !paths.framework.join("Chromium Embedded Framework").is_file() || !paths.helper.is_file() {
        return Err("macOS CEF packaged binaries are missing".into());
    }
    Ok(Arc::new(DeferredEngine { paths: Mutex::new(Some(paths)), initialization: Arc::new(Initialization::default()) }))
}

#[derive(Default)]
struct InitializationGate { started: bool, closing: bool }

/// The dispatch callback, not the requesting future, owns initialization and
/// its one retained result. Shutdown can join it even after a caller cancels.
struct Initialization<T: Clone> {
    gate: Mutex<InitializationGate>,
    result: tokio::sync::watch::Sender<Option<Result<Option<T>, String>>>,
}
impl<T: Clone> Default for Initialization<T> {
    fn default() -> Self {
        Self { gate: Mutex::new(InitializationGate::default()), result: tokio::sync::watch::channel(None).0 }
    }
}
impl<T: Clone> Initialization<T> {
    fn begin(&self) -> Result<bool, String> {
        let mut gate = self.gate.lock().unwrap();
        if gate.closing { return Err("CEF host is closing".into()); }
        if gate.started { return Ok(false); }
        gate.started = true;
        Ok(true)
    }
    fn closing(&self) -> bool { self.gate.lock().unwrap().closing }
    fn finish(&self, value: Result<Option<T>, String>) { self.result.send_replace(Some(value)); }
    async fn settled(&self) -> Result<Option<T>, String> {
        let mut result = self.result.subscribe();
        loop {
            let current = result.borrow_and_update().clone();
            if let Some(value) = current { return value; }
            result.changed().await.map_err(|_| "CEF initialization result was lost")?;
        }
    }
    async fn close(&self) -> Result<Option<T>, String> {
        let started = {
            let mut gate = self.gate.lock().unwrap();
            gate.closing = true;
            gate.started
        };
        if !started { return Ok(None); }
        // Only an explicitly skipped initialization has no Engine. A real
        // initialization failure stays a failure, not manufactured cleanup.
        self.settled().await
    }
}

pub(crate) struct DeferredEngine {
    paths: Mutex<Option<Paths>>,
    initialization: Arc<Initialization<Arc<Engine>>>,
}
impl DeferredEngine {
    pub(crate) async fn get(self: &Arc<Self>, app: &tauri::AppHandle) -> Result<Arc<Engine>, String> {
        if self.initialization.begin()? {
            let owner = self.clone();
            if let Err(error) = app.run_on_main_thread(move || {
                if owner.initialization.closing() {
                    owner.initialization.finish(Ok(None));
                    return;
                }
                let paths = owner.paths.lock().unwrap().take().expect("single CEF initialization owns its paths");
                #[cfg(debug_assertions)]
                eprintln!("CEF_HOST phase=initialize_begin");
                let initialized = Engine::initialize(paths);
                #[cfg(debug_assertions)]
                eprintln!("CEF_HOST phase=initialize_done success={}", initialized.is_ok());
                owner.initialization.finish(initialized.map(Some));
            }) {
                self.initialization.finish(Err(format!("CEF main-thread initialization dispatch failed: {error}")));
            }
        }
        let engine = self.initialization.settled().await?.ok_or("CEF initialization skipped during host shutdown")?;
        if self.initialization.closing() { return Err("CEF host is closing".into()); }
        Ok(engine)
    }
    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        if let Some(engine) = self.initialization.close().await? {
            engine.shutdown().await
        } else {
            #[cfg(debug_assertions)]
            eprintln!("CEF_HOST phase=unused_closed");
            Ok(())
        }
    }
}

fn packaged_paths(executable: &Path, data_dir: &Path) -> Result<Paths, String> {
    let macos = executable
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "MacOS"))
        .ok_or_else(|| "macOS CEF is available only from a packaged application".to_owned())?;
    let contents = macos
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "Contents"))
        .ok_or_else(|| "macOS CEF application bundle layout is invalid".to_owned())?;
    let bundle = contents
        .parent()
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or_else(|| "macOS CEF application bundle root is invalid".to_owned())?;
    let frameworks = contents.join("Frameworks");
    Ok(Paths {
        framework: frameworks.join(FRAMEWORK_NAME),
        helper: frameworks
            .join(format!("{HELPER_NAME}.app"))
            .join("Contents/MacOS")
            .join(HELPER_NAME),
        main_bundle: bundle.to_path_buf(),
        // CEF Chrome requires every disk-backed request-context profile to be
        // a direct child of root_cache_path. BrowserProfileStore owns the
        // hashed AgentSession directories immediately below this root.
        data_root: data_dir.join("browser-v3").join("agent-sessions"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unused_native_host_closes_without_starting_or_late_admission() {
        let state = Initialization::<u8>::default();
        assert_eq!(state.close().await.unwrap(), None);
        assert!(!state.gate.lock().unwrap().started);
        assert!(state.begin().is_err());
    }

    #[tokio::test]
    async fn cancelled_waiter_keeps_single_initialization_owned_until_shutdown() {
        let state = Arc::new(Initialization::<Arc<()>>::default());
        assert!(state.begin().unwrap());
        assert!(!state.begin().unwrap());
        let waiter = tokio::spawn({ let state = state.clone(); async move { state.settled().await } });
        tokio::task::yield_now().await;
        waiter.abort();
        let close = tokio::spawn({ let state = state.clone(); async move { state.close().await } });
        tokio::task::yield_now().await;
        assert!(!close.is_finished(), "pending initialization is not proof of cleanup");
        let engine = Arc::new(());
        state.finish(Ok(Some(engine.clone())));
        assert!(Arc::ptr_eq(&close.await.unwrap().unwrap().unwrap(), &engine));
        assert!(state.begin().is_err());
    }

    #[tokio::test]
    async fn closing_queued_initialization_or_failed_dispatch_never_restarts_it() {
        for closing_before_callback in [true, false] {
            let state = Arc::new(Initialization::<u8>::default());
            assert!(state.begin().unwrap());
            if closing_before_callback {
                let close = tokio::spawn({ let state = state.clone(); async move { state.close().await } });
                tokio::task::yield_now().await;
                assert!(state.closing());
                state.finish(Ok(None));
                assert_eq!(close.await.unwrap().unwrap(), None);
            } else {
                state.finish(Err("main-thread dispatch failed".into()));
                assert!(!state.begin().unwrap(), "failed dispatch is cached, not retried");
                assert!(state.settled().await.is_err());
                assert!(state.close().await.is_err(), "initialization faults must not become cleanup success");
            }
            assert!(state.begin().is_err());
        }
    }

    #[test]
    fn resolves_only_the_owned_product_bundle_layout() {
        let root = tempfile::tempdir().unwrap();
        let executable = root
            .path()
            .join("NomiFun.app/Contents/MacOS/nomifun-desktop");
        let data = root.path().join("data");
        let paths = packaged_paths(&executable, &data).unwrap();
        assert_eq!(
            paths.framework,
            root.path().join(
                "NomiFun.app/Contents/Frameworks/Chromium Embedded Framework.framework"
            )
        );
        assert_eq!(
            paths.helper,
            root.path().join(
                "NomiFun.app/Contents/Frameworks/NomiFun Helper.app/Contents/MacOS/NomiFun Helper"
            )
        );
        assert_eq!(paths.main_bundle, root.path().join("NomiFun.app"));
        assert_eq!(
            paths.data_root,
            data.join("browser-v3").join("agent-sessions")
        );
        let key = nomifun_browser_platform::runtime::BrowserResourceKey {
            principal_id: "fixture-user".into(),
            agent_session_id: "fixture-session".into(),
            resource_binding_id: "browser:managed-browser".into(),
        };
        let nomifun_browser_platform::runtime::BrowserProfile::Persistent(profile) =
            nomifun_browser_platform::runtime::BrowserProfile::for_agent_session(
                &data, &key, false,
            )
        else {
            panic!("persistent Browser policy returned an ephemeral profile")
        };
        assert_eq!(profile.parent(), Some(paths.data_root.as_path()));

        assert!(packaged_paths(&root.path().join("nomifun-desktop"), &data).is_err());
        assert!(packaged_paths(
            &root.path().join("NomiFun/Contents/MacOS/nomifun-desktop"),
            &data
        )
        .is_err());
    }
}
