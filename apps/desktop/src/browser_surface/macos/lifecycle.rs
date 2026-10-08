//! Production CEF bundle discovery and process-wide lifetime.
//!
//! A development binary outside a macOS application bundle is intentionally
//! unavailable. The native fixture has its own explicit bundle contract; the
//! product host accepts only the packaged framework/helper layout below.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nomifun_browser_macos::engine::{Engine, Paths};

const FRAMEWORK_NAME: &str = "Chromium Embedded Framework.framework";
const HELPER_NAME: &str = "NomiFun Helper";

#[cfg(target_arch = "aarch64")]
const BROWSER_RUNTIME: &str = include_str!("../../../browser-runtime.json");
#[cfg(target_arch = "x86_64")]
const BROWSER_RUNTIME: &str = include_str!("../../../browser-runtime-intel.json");
#[cfg(target_arch = "aarch64")]
const BROWSER_CPU_TYPE: u32 = 0x0100_000c;
#[cfg(target_arch = "x86_64")]
const BROWSER_CPU_TYPE: u32 = 0x0100_0007;

pub(crate) fn prepare(data_dir: &Path) -> Result<Arc<DeferredEngine>, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "macOS CEF executable path is unavailable".to_owned())?;
    prepare_executable(&executable, data_dir)
}

fn prepare_executable(executable: &Path, data_dir: &Path) -> Result<Arc<DeferredEngine>, String> {
    let mut paths = packaged_paths(executable, data_dir)?;
    // Resolve owned bundle paths without starting CEF or asking Keychain.
    // The process entry point separately preloads the library before workers.
    paths.main_bundle = paths.main_bundle.canonicalize().map_err(|_| "CEF main bundle is missing")?;
    paths.framework = owned_component(&paths.main_bundle, &paths.framework, "CEF framework")?;
    let library = owned_component(&paths.main_bundle, &paths.framework.join("Chromium Embedded Framework"), "CEF library")?;
    require_target_binary(&library, "CEF library")?;
    let expected: serde_json::Value = serde_json::from_str(BROWSER_RUNTIME)
        .expect("the compiled browser runtime contract is valid JSON");
    let metadata_path = owned_component(&paths.main_bundle, &paths.main_bundle.join("Contents/Resources/browser-cef/runtime.json"), "CEF runtime metadata")?;
    let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata_path)
        .map_err(|_| "CEF runtime metadata cannot be read")?)
        .map_err(|_| "CEF runtime metadata is invalid")?;
    for field in ["cef", "chromium", "crate", "architecture", "archive", "archive_sha1"] {
        if metadata.get(field) != expected.get(field) {
            return Err(format!("CEF runtime metadata does not match this application: {field}"));
        }
    }
    for helper in expected["helpers"].as_array().expect("compiled CEF helper contract") {
        let helper = helper.as_str().expect("compiled CEF helper name");
        let binary = owned_component(&paths.main_bundle,
            &paths.main_bundle.join("Contents/Frameworks").join(format!("{helper}.app/Contents/MacOS/{helper}")), helper)?;
        require_target_binary(&binary, helper)?;
        if std::fs::metadata(&binary).map_err(|_| format!("CEF helper cannot be inspected: {helper}"))?
            .permissions().mode() & 0o111 == 0 {
            return Err(format!("CEF helper is not executable: {helper}"));
        }
    }
    paths.helper = owned_component(&paths.main_bundle, &paths.helper, "CEF helper")?;
    for resource in expected["resources"].as_array().expect("compiled CEF resources contract") {
        let resource = resource.as_str().expect("compiled CEF resource name");
        let path = owned_component(&paths.main_bundle, &paths.framework.join("Resources").join(resource), resource)?;
        if !path.is_file() { return Err(format!("CEF browser resource is missing: {resource}")); }
    }
    Ok(Arc::new(DeferredEngine { paths: Mutex::new(Some(paths)), initialization: Arc::new(Initialization::default()) }))
}

fn owned_component(bundle: &Path, component: &Path, label: &str) -> Result<PathBuf, String> {
    let resolved = component.canonicalize().map_err(|_| format!("macOS built-in browser component is missing: {label}"))?;
    if !resolved.starts_with(bundle) {
        return Err(format!("macOS built-in browser component is outside its application: {label}"));
    }
    Ok(resolved)
}

fn require_target_binary(path: &Path, label: &str) -> Result<(), String> {
    let mut header = [0u8; 8];
    std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut header))
        .map_err(|_| format!("macOS built-in browser binary cannot be read: {label}"))?;
    // Each target ships its own pinned runtime. Reject incomplete files and
    // mixed-architecture helpers before advertising a usable native host.
    if header[..4] != [0xcf, 0xfa, 0xed, 0xfe]
        || u32::from_le_bytes(header[4..].try_into().unwrap()) != BROWSER_CPU_TYPE {
        return Err(format!("macOS built-in browser binary must match {}: {label}", std::env::consts::ARCH));
    }
    Ok(())
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
    /// # Safety
    /// The desktop entry point calls this before Tauri/Tokio/worker startup.
    pub(crate) unsafe fn preload_framework(&self) -> Result<(), String> {
        let paths = self.paths.lock().unwrap();
        let paths = paths.as_ref().ok_or("CEF startup paths were already consumed")?;
        unsafe { Engine::preload_framework(&paths.framework) }
    }

    pub(crate) async fn get(self: &Arc<Self>, app: &tauri::AppHandle) -> Result<Arc<Engine>, String> {
        let runtime=tokio::runtime::Handle::try_current().map_err(|_|"CEF guardian requires the retained host runtime")?;
        if self.initialization.begin()? {
            let owner = self.clone();
            if let Err(error) = app.run_on_main_thread(move || {
                let _runtime=runtime.enter();
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

    pub(crate) async fn shutdown_after_storage_close(&self) -> Result<(), String> {
        if let Some(engine)=self.initialization.close().await? {
            engine.shutdown_after_storage_close().await
        } else {Ok(())}
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
        data_root: data_dir.join("browser-v4").join("agent-sessions"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_header(cpu_type: u32) -> [u8; 8] {
        let mut header = [0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0];
        header[4..].copy_from_slice(&cpu_type.to_le_bytes());
        header
    }

    fn complete_bundle(root: &Path) -> PathBuf {
        let executable = root.join("NomiFun.app/Contents/MacOS/nomifun-desktop");
        let bundle = executable.parent().unwrap().parent().unwrap().parent().unwrap();
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        let framework = bundle.join("Contents/Frameworks/Chromium Embedded Framework.framework");
        std::fs::create_dir_all(framework.join("Resources")).unwrap();
        let header = binary_header(BROWSER_CPU_TYPE);
        std::fs::write(framework.join("Chromium Embedded Framework"), header).unwrap();
        let expected: serde_json::Value = serde_json::from_str(BROWSER_RUNTIME).unwrap();
        for helper in expected["helpers"].as_array().unwrap() {
            let name = helper.as_str().unwrap();
            let path = bundle.join("Contents/Frameworks").join(format!("{name}.app/Contents/MacOS/{name}"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, header).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        for resource in expected["resources"].as_array().unwrap() {
            let resource = resource.as_str().unwrap();
            std::fs::write(framework.join("Resources").join(resource), "fixture").unwrap();
        }
        let metadata = bundle.join("Contents/Resources/browser-cef/runtime.json");
        std::fs::create_dir_all(metadata.parent().unwrap()).unwrap();
        std::fs::write(metadata, serde_json::to_vec(&expected).unwrap()).unwrap();
        executable
    }

    #[test]
    fn availability_requires_the_complete_exact_runtime_and_all_helper_architectures() {
        let root = tempfile::tempdir().unwrap();
        let executable = complete_bundle(root.path());
        assert!(prepare_executable(&executable, &root.path().join("data")).is_ok());
        let helper = root.path().join("NomiFun.app/Contents/Frameworks/NomiFun Helper (Renderer).app/Contents/MacOS/NomiFun Helper (Renderer)");
        std::fs::remove_file(&helper).unwrap();
        assert!(prepare_executable(&executable, root.path()).err().unwrap().contains("Renderer"));
        let other_cpu = if BROWSER_CPU_TYPE == 0x0100_000c { 0x0100_0007 } else { 0x0100_000c };
        std::fs::write(&helper, binary_header(other_cpu)).unwrap();
        assert!(prepare_executable(&executable, root.path()).err().unwrap().contains(std::env::consts::ARCH));
        std::fs::write(&helper, binary_header(BROWSER_CPU_TYPE)).unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(prepare_executable(&executable, root.path()).err().unwrap().contains("not executable"));
    }

    #[test]
    fn mismatched_runtime_metadata_and_missing_resources_fail_before_initialization() {
        let root = tempfile::tempdir().unwrap();
        let executable = complete_bundle(root.path());
        let metadata = root.path().join("NomiFun.app/Contents/Resources/browser-cef/runtime.json");
        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
        value["cef"] = "wrong-version".into();
        std::fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(prepare_executable(&executable, root.path()).err().unwrap().contains("metadata"));
        complete_bundle(root.path());
        std::fs::remove_file(root.path().join("NomiFun.app/Contents/Frameworks/Chromium Embedded Framework.framework/Resources/icudtl.dat")).unwrap();
        assert!(prepare_executable(&executable, root.path()).err().unwrap().contains("icudtl.dat"));
    }

    #[test]
    fn other_architecture_metadata_cannot_advertise_a_usable_browser() {
        let root = tempfile::tempdir().unwrap();
        let executable = complete_bundle(root.path());
        let metadata = root.path().join("NomiFun.app/Contents/Resources/browser-cef/runtime.json");
        let other_runtime = if cfg!(target_arch = "aarch64") {
            include_str!("../../../browser-runtime-intel.json")
        } else {
            include_str!("../../../browser-runtime.json")
        };
        std::fs::write(metadata, other_runtime).unwrap();
        assert!(prepare_executable(&executable, root.path()).err().unwrap().contains("architecture"));
    }

    #[test]
    fn packaged_components_cannot_escape_the_application_bundle() {
        let root = tempfile::tempdir().unwrap();
        let executable = complete_bundle(root.path());
        let helper = root.path().join("NomiFun.app/Contents/Frameworks/NomiFun Helper.app/Contents/MacOS/NomiFun Helper");
        let outside = root.path().join("outside-helper");
        std::fs::rename(&helper, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &helper).unwrap();
        assert!(prepare_executable(&executable, root.path()).err().unwrap().contains("outside its application"));
    }

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
            data.join("browser-v4").join("agent-sessions")
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
