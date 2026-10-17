//! Real private D-Bus protocol/lifecycle tests. The mock exports the native
//! portal wire interfaces; it is not evidence of a compositor or OS grant.

use super::*;
use std::sync::{Mutex as StdMutex, atomic::AtomicU32};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

#[derive(Default)]
struct State {
    mode: AtomicU32,
    started: AtomicBool,
    session_closes: AtomicU32,
    request_closes: AtomicU32,
    notifications: AtomicU32,
    selection: StdMutex<Option<Values>>,
}

struct Fixture {
    _daemon: Child,
    server: Connection,
    client: Connection,
    state: Arc<State>,
}

impl Fixture {
    async fn new(mode: u32) -> Self {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("private portal protocol tests require dbus-daemon");
        let mut lines = BufReader::new(daemon.stdout.take().unwrap()).lines();
        let address = tokio::time::timeout(Duration::from_secs(3), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let state = Arc::new(State::default());
        state.mode.store(mode, Ordering::Release);
        let server = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name(DESTINATION)
            .unwrap()
            .serve_at(DESKTOP_PATH, Remote(state.clone()))
            .unwrap()
            .serve_at(DESKTOP_PATH, Capture(state.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .method_timeout(Duration::from_secs(1))
            .build()
            .await
            .unwrap();
        Self {
            _daemon: daemon,
            server,
            client,
            state,
        }
    }
}

struct NativeSession(Arc<State>);
#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl NativeSession {
    async fn close(&self) {
        self.0.session_closes.fetch_add(1, Ordering::AcqRel);
    }
}

struct NativeRequest(Arc<State>);
#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl NativeRequest {
    async fn close(&self) {
        self.0.request_closes.fetch_add(1, Ordering::AcqRel);
    }
}

struct Remote(Arc<State>);
#[zbus::interface(name = "org.freedesktop.portal.RemoteDesktop")]
impl Remote {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn available_device_types(&self) -> u32 {
        3
    }

    async fn create_session(
        &self,
        options: Values,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = header.sender().unwrap().as_str();
        let token = String::try_from(options["session_handle_token"].try_clone().unwrap()).unwrap();
        let handle = format!(
            "{DESKTOP_PATH}/session/{}/{}",
            sender.trim_start_matches(':').replace('.', "_"),
            token
        );
        connection
            .object_server()
            .at(handle.as_str(), NativeSession(self.0.clone()))
            .await?;
        let mut values = Values::new();
        values.insert("session_handle".into(), string_value(handle).unwrap());
        respond(
            connection,
            sender,
            &options,
            self.0.clone(),
            Some((0, values)),
        )
        .await
    }

    async fn select_devices(
        &self,
        _session: OwnedObjectPath,
        options: Values,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let copied = options
            .iter()
            .map(|(key, value)| (key.clone(), value.try_clone().unwrap()))
            .collect();
        *self.0.selection.lock().unwrap() = Some(copied);
        respond(
            connection,
            header.sender().unwrap().as_str(),
            &options,
            self.0.clone(),
            Some((0, Values::new())),
        )
        .await
    }

    async fn start(
        &self,
        _session: OwnedObjectPath,
        _parent: String,
        options: Values,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        self.0.started.store(true, Ordering::Release);
        let mode = self.0.mode.load(Ordering::Acquire);
        let response = if mode == 2 {
            None
        } else if mode == 1 {
            Some((1, Values::new()))
        } else {
            let mut properties = Values::new();
            properties.insert(
                "position".into(),
                OwnedValue::try_from(zbus::zvariant::Value::from((10_i32, 20_i32))).unwrap(),
            );
            properties.insert(
                "size".into(),
                OwnedValue::try_from(zbus::zvariant::Value::from((640_i32, 480_i32))).unwrap(),
            );
            let mut result = Values::new();
            result.insert("devices".into(), OwnedValue::from(3_u32));
            result.insert(
                "restore_token".into(),
                string_value("replacement-token".to_owned()).unwrap(),
            );
            result.insert(
                "streams".into(),
                OwnedValue::try_from(zbus::zvariant::Value::from(vec![(42_u32, properties)]))
                    .unwrap(),
            );
            Some((0, result))
        };
        respond(
            connection,
            header.sender().unwrap().as_str(),
            &options,
            self.0.clone(),
            response,
        )
        .await
    }

    async fn notify_keyboard_keysym(
        &self,
        _session: OwnedObjectPath,
        _options: Values,
        keysym: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        assert!(keysym > 0);
        assert!(state <= 1);
        self.0.notifications.fetch_add(1, Ordering::AcqRel);
        if self.0.mode.load(Ordering::Acquire) == 4 {
            std::future::pending::<()>().await;
        }
        if self.0.mode.load(Ordering::Acquire) == 3 {
            Err(zbus::fdo::Error::Failed(
                "fixture input result lost".to_owned(),
            ))
        } else {
            Ok(())
        }
    }
}

struct Capture(Arc<State>);
#[zbus::interface(name = "org.freedesktop.portal.ScreenCast")]
impl Capture {
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        1
    }
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        5
    }
    async fn select_sources(
        &self,
        _session: OwnedObjectPath,
        options: Values,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        assert!(!options.contains_key("persist_mode"));
        assert!(!options.contains_key("restore_token"));
        respond(
            connection,
            header.sender().unwrap().as_str(),
            &options,
            self.0.clone(),
            Some((0, Values::new())),
        )
        .await
    }
}

async fn respond(
    connection: &Connection,
    sender: &str,
    options: &Values,
    state: Arc<State>,
    response: Option<(u32, Values)>,
) -> zbus::fdo::Result<OwnedObjectPath> {
    let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
    let path = OwnedObjectPath::try_from(format!(
        "{DESKTOP_PATH}/request/{}/{}",
        sender.trim_start_matches(':').replace('.', "_"),
        token
    ))
    .unwrap();
    connection
        .object_server()
        .at(path.clone(), NativeRequest(state))
        .await?;
    // Deliberately emit before the method reply to exercise the real protocol
    // race covered by the preinstalled Response subscription.
    if let Some(response) = response {
        connection
            .emit_signal(Some(sender), path.as_str(), REQUEST, "Response", &response)
            .await?;
    }
    Ok(path)
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn private_portal_roundtrip_observes_native_reply_and_rotates_token() {
    let fixture = Fixture::new(0).await;
    let (session, token) =
        create_session(fixture.client.clone(), Some("consumed-token".into()), None)
            .await
            .unwrap();
    assert_eq!(session.streams()[0].node_id, 42);
    assert_eq!(session.streams()[0].position, Some((10, 20)));
    assert_eq!(token.as_deref(), Some("replacement-token"));
    let selection = fixture.state.selection.lock().unwrap();
    assert_eq!(
        u32::try_from(&selection.as_ref().unwrap()["persist_mode"]).unwrap(),
        1
    );
    assert_eq!(
        String::try_from(
            selection.as_ref().unwrap()["restore_token"]
                .try_clone()
                .unwrap()
        )
        .unwrap(),
        "consumed-token"
    );
    drop(selection);
    session.keyboard_keysym(0x0100_4e2d, true).await.unwrap();
    session.keyboard_keysym(0x0100_4e2d, false).await.unwrap();
    assert_eq!(fixture.state.notifications.load(Ordering::Acquire), 2);
    session.close().await.unwrap();
    session.close().await.unwrap();
    assert_eq!(fixture.state.session_closes.load(Ordering::Acquire), 1);
    assert!(session.is_closed());
}

#[tokio::test]
async fn private_portal_cancelled_native_dialog_closes_session() {
    let fixture = Fixture::new(1).await;
    let (sender, receiver) = watch::channel(None);
    let task = tokio::spawn(create_session(fixture.client.clone(), None, Some(sender)));
    let pending = PendingCreation {
        id: 1,
        abort: task.abort_handle(),
        cleanup: receiver,
    };
    let error = match task.await.unwrap() {
        Ok(_) => panic!("cancelled grant accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "WAYLAND_PERMISSION_CANCELLED");
    settle_pending(pending).await.unwrap();
    assert_eq!(fixture.state.session_closes.load(Ordering::Acquire), 1);
    assert_eq!(fixture.state.request_closes.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn private_portal_interrupt_closes_pending_request_and_session() {
    let fixture = Fixture::new(2).await;
    let (sender, receiver) = watch::channel(None);
    let task = tokio::spawn(create_session(fixture.client.clone(), None, Some(sender)));
    wait_until(|| fixture.state.started.load(Ordering::Acquire)).await;
    settle_pending(PendingCreation {
        id: 2,
        abort: task.abort_handle(),
        cleanup: receiver,
    })
    .await
    .unwrap();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("interrupted portal creation continued"),
    }
    assert_eq!(fixture.state.request_closes.load(Ordering::Acquire), 1);
    assert_eq!(fixture.state.session_closes.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn private_portal_unknown_input_is_not_replayed_and_native_close_is_remembered() {
    let fixture = Fixture::new(3).await;
    let (session, _) = create_session(fixture.client.clone(), None, None)
        .await
        .unwrap();
    let error = session.keyboard_keysym(65, true).await.unwrap_err();
    assert_eq!(error.code(), "WAYLAND_INPUT_RESULT_UNKNOWN");
    assert_eq!(fixture.state.notifications.load(Ordering::Acquire), 1);
    assert!(session.is_closed());
    session.close().await.unwrap();
    assert_eq!(fixture.state.session_closes.load(Ordering::Acquire), 1);
    assert_eq!(
        session.keyboard_keysym(65, true).await.unwrap_err().code(),
        "WAYLAND_SESSION_CLOSED"
    );
    assert_eq!(fixture.state.notifications.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn private_portal_revocation_invalidates_existing_session_without_reauthorization() {
    let fixture = Fixture::new(0).await;
    let (session, _) = create_session(fixture.client.clone(), None, None)
        .await
        .unwrap();
    fixture
        .server
        .emit_signal(
            None::<&str>,
            session.handle.as_str(),
            SESSION,
            "Closed",
            &(Values::new(),),
        )
        .await
        .unwrap();
    wait_until(|| session.is_closed()).await;
    session.close().await.unwrap();
    assert_eq!(fixture.state.session_closes.load(Ordering::Acquire), 0);
    assert_eq!(
        session.keyboard_keysym(65, true).await.unwrap_err().code(),
        "WAYLAND_SESSION_CLOSED"
    );
    assert_eq!(fixture.state.notifications.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn private_portal_cancelled_inflight_input_revokes_native_session() {
    let fixture = Fixture::new(4).await;
    let (session, _) = create_session(fixture.client.clone(), None, None)
        .await
        .unwrap();
    let input_session = session.clone();
    let input = tokio::spawn(async move { input_session.keyboard_keysym(65, true).await });
    wait_until(|| fixture.state.notifications.load(Ordering::Acquire) == 1).await;
    input.abort();
    assert!(input.await.unwrap_err().is_cancelled());
    wait_until(|| fixture.state.session_closes.load(Ordering::Acquire) == 1).await;
    assert!(session.is_closed());
    session.close().await.unwrap();
    assert_eq!(fixture.state.session_closes.load(Ordering::Acquire), 1);
    assert_eq!(fixture.state.notifications.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn private_portal_owner_loss_is_closed_without_fabricating_cleanup_proof() {
    let fixture = Fixture::new(0).await;
    let (session, _) = create_session(fixture.client.clone(), None, None)
        .await
        .unwrap();
    fixture.server.release_name(DESTINATION).await.unwrap();
    wait_until(|| session.is_closed()).await;
    assert!(!session.close_confirmed.load(Ordering::Acquire));
    assert_eq!(
        session.keyboard_keysym(65, true).await.unwrap_err().code(),
        "WAYLAND_SESSION_CLOSED"
    );
    assert_eq!(fixture.state.notifications.load(Ordering::Acquire), 0);
    assert_eq!(
        session.close().await.unwrap_err().code(),
        "WAYLAND_SESSION_CLOSE_UNKNOWN"
    );
}

#[tokio::test]
async fn private_portal_capture_flag_cancellation_joins_its_exact_native_cleanup() {
    let fixture = Fixture::new(2).await;
    let (sender, cleanup) = watch::channel(None);
    let native = tokio::spawn(create_session(fixture.client.clone(), None, Some(sender)));
    let pending = PendingCreation {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        abort: native.abort_handle(),
        cleanup,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let capture = tokio::spawn(await_creation(native, pending, cancel));
    wait_until(|| fixture.state.started.load(Ordering::Acquire)).await;
    flag.store(true, Ordering::Release);
    let error = match capture.await.unwrap() {
        Ok(_) => panic!("cancelled screenshot kept a native grant"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "WAYLAND_PERMISSION_CANCELLED");
    // These are acknowledged by the private native service before returning
    // cancellation, not merely queued cleanup messages from a detached worker.
    assert_eq!(fixture.state.request_closes.load(Ordering::Acquire), 1);
    assert_eq!(fixture.state.session_closes.load(Ordering::Acquire), 1);
    fixture.state.mode.store(0, Ordering::Release);
    let (fresh, _) = create_session(fixture.client.clone(), None, None)
        .await
        .unwrap();
    assert!(!fresh.is_closed());
    fresh.keyboard_keysym(65, true).await.unwrap();
    fresh.keyboard_keysym(65, false).await.unwrap();
    assert_eq!(fixture.state.session_closes.load(Ordering::Acquire), 1);
    fresh.close().await.unwrap();
}

#[tokio::test]
async fn private_portal_cancelled_queued_input_sends_no_new_event_and_allows_release() {
    let fixture = Fixture::new(0).await;
    let (session, _) = create_session(fixture.client.clone(), None, None)
        .await
        .unwrap();
    session.keyboard_keysym(65, true).await.unwrap();
    let gate = session.input_gate.lock().await;
    let cancel = Arc::new(AtomicBool::new(false));
    let queued_session = session.clone();
    let queued_cancel = cancel.clone();
    let queued = tokio::spawn(async move {
        queued_session
            .keyboard_keysym_cancellable(queued_cancel, 66, true)
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    cancel.store(true, Ordering::Release);
    drop(gate);
    assert_eq!(
        queued.await.unwrap().unwrap_err().code(),
        "WAYLAND_INPUT_CANCELLED"
    );
    assert_eq!(fixture.state.notifications.load(Ordering::Acquire), 1);
    assert!(!session.is_closed());
    // Releasing the exact earlier key bypasses the operation's cancellation.
    session.keyboard_keysym(65, false).await.unwrap();
    assert_eq!(fixture.state.notifications.load(Ordering::Acquire), 2);
    session.close().await.unwrap();
}
