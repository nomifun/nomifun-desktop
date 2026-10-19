//! Real Event + authenticated pipe/ACK fixtures. No UAC or desktop input.
use super::*;
use super::tests::security;
use std::sync::{atomic::AtomicUsize, mpsc};

#[derive(Default)]
struct Extension(AtomicUsize);
impl WindowsExecutionExtension for Extension {
    fn call(&self, _: Value) -> Result<Value, SystemExecutionError> {
        Ok(json!({"calls": self.0.fetch_add(1, Ordering::SeqCst) + 1}))
    }
    fn shutdown(&self) -> Result<(), SystemExecutionError> { Ok(()) }
}

fn runner(extension: Arc<dyn WindowsExecutionExtension>) -> Arc<Runner> {
    let mut runner = Runner::new(); runner.extension = Some(extension); Arc::new(runner)
}

struct Fixture {
    facility: Arc<Facility>,
    worker: thread::JoinHandle<()>,
    ack: mpsc::Receiver<()>,
}

fn fixture(count: usize, slots: Arc<RpcDispatchSlots>, runners: HashMap<Uuid, Arc<Runner>>) -> Fixture {
    let name = format!(r"\\.\pipe\NomiFun.SystemExecution.Cancellation.{}", Uuid::now_v7());
    let first = listener(&name, &security(), true).unwrap();
    let next_name = name.clone();
    let (acknowledged, ack) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut pending = first;
        let peer = Arc::new(process(std::process::id()).unwrap());
        let runners = Arc::new(Mutex::new(runners));
        let mut workers = Vec::new();
        for index in 0..count {
            connect_pipe(&pending, &peer, None, Instant::now() + Duration::from_secs(10)).unwrap();
            let pipe = pending;
            if index + 1 < count { pending = listener(&next_name, &security(), false).unwrap(); }
            else { pending = event().unwrap(); }
            let runners = runners.clone(); let peer = peer.clone();
            let slots = slots.clone(); let acknowledged = acknowledged.clone();
            workers.push(thread::spawn(move || {
                let stopping = AtomicBool::new(false);
                handle_rpc(&pipe, &peer, &runners, &stopping, &event().unwrap(), &FacilityAdmission::default(), &slots).unwrap();
                assert!(!stopping.load(Ordering::Acquire));
                assert!(!runners.lock().unwrap().is_empty(), "one request cannot retire another client");
                // handle_rpc returns only after consuming this exact reply ACK.
                acknowledged.send(()).unwrap();
                unsafe { DisconnectNamedPipe(pipe.0); }
            }));
        }
        for worker in workers { worker.join().unwrap(); }
    });
    Fixture { facility: Arc::new(Facility { pipe:name, process:process(std::process::id()).unwrap(),
        stopped:AtomicBool::new(false), foreground_admission:FacilityAdmission::default() }), worker, ack }
}

fn wait_for_queue(slots: &RpcDispatchSlots) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while slots.waiting.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline, "original request never reached the full native admission queue");
        thread::yield_now();
    }
}

#[test]
fn full_business_slots_accept_early_and_queued_stop_with_exact_ack_and_keep_siblings() {
    let slots = Arc::new(RpcDispatchSlots::new());
    let permits = (0..BUSINESS_RPC_SLOTS).map(|_| slots.admit("poll", || false).unwrap().unwrap()).collect::<Vec<_>>();
    let a = Uuid::now_v7(); let b = Uuid::now_v7();
    let extension_a = Arc::new(Extension::default()); let extension_b = Arc::new(Extension::default());
    let fixture = fixture(4, slots.clone(), HashMap::from([(a, runner(extension_a.clone())), (b, runner(extension_b.clone()))]));
    let request = || json!({"op":"extension","request":{"operation":"fixture"}});

    let early = CancellationSource::new(a).unwrap();
    early.signal().unwrap(); // Stop precedes all helper registration and slot acquisition.
    let error = fixture.facility.exchange_controlled(a, request(), Some(&early)).unwrap_err();
    assert_eq!(error.code, "SYSTEM_EXECUTION_CANCELLED_BEFORE_DISPATCH");
    assert_eq!(error.details["user_code_not_started"], true);
    fixture.ack.recv_timeout(Duration::from_secs(2)).unwrap();

    let source = Arc::new(CancellationSource::new(a).unwrap());
    let waiting = thread::spawn({ let facility = fixture.facility.clone(); let source = source.clone(); move || {
        facility.exchange_controlled(a, json!({"op":"extension","request":{"operation":"queued"}}), Some(&source))
    }});
    wait_for_queue(&slots);
    source.signal().unwrap();
    let error = waiting.join().unwrap().unwrap_err();
    assert_eq!(error.code, "SYSTEM_EXECUTION_CANCELLED_BEFORE_DISPATCH");
    assert_eq!(error.details["cleanup_proven"], true);
    fixture.ack.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(*slots.active.lock().unwrap(), BUSINESS_RPC_SLOTS, "Stop cannot consume or return another request's permit");
    assert_eq!(extension_a.0.load(Ordering::SeqCst), 0);
    assert_eq!(extension_b.0.load(Ordering::SeqCst), 0);
    drop(permits);

    source.signal().unwrap(); early.signal().unwrap(); // Old Stop cannot bind either successor.
    let sibling = CancellationSource::new(b).unwrap();
    assert_eq!(fixture.facility.exchange_controlled(b, request(), Some(&sibling)).unwrap()["calls"], 1);
    let successor = CancellationSource::new(a).unwrap();
    assert_eq!(fixture.facility.exchange_controlled(a, request(), Some(&successor)).unwrap()["calls"], 1);
    fixture.ack.recv_timeout(Duration::from_secs(2)).unwrap();
    fixture.ack.recv_timeout(Duration::from_secs(2)).unwrap();
    fixture.worker.join().unwrap();
    assert_eq!(*slots.active.lock().unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_gui_waiter_signals_but_original_rpc_worker_retains_event_and_ack_owner() {
    let slots = Arc::new(RpcDispatchSlots::new());
    let permits = (0..BUSINESS_RPC_SLOTS).map(|_| slots.admit("poll", || false).unwrap().unwrap()).collect::<Vec<_>>();
    let id = Uuid::now_v7(); let extension = Arc::new(Extension::default());
    let fixture = fixture(1, slots.clone(), HashMap::from([(id, runner(extension.clone()))]));
    let admission = Arc::new(tokio::sync::RwLock::new(()));
    let client = Arc::new(crate::SystemExecutionClient { native: Arc::new(Client { facility:fixture.facility.clone(), id }), admission:admission.clone() });
    let original = tokio::spawn({ let client = client.clone(); async move {
        client.call_windows_controlled(json!({"op":"extension","request":{}}), &tokio_util::sync::CancellationToken::new()).await
    }});
    wait_for_queue(&slots);
    assert!(admission.try_write().is_err());
    original.abort();
    assert!(original.await.unwrap_err().is_cancelled());
    fixture.ack.recv_timeout(Duration::from_secs(2)).unwrap();
    fixture.worker.join().unwrap();
    assert_eq!(extension.0.load(Ordering::SeqCst), 0);
    assert_eq!(*slots.active.lock().unwrap(), BUSINESS_RPC_SLOTS);
    tokio::time::timeout(Duration::from_secs(2), async { let _joined = admission.write().await; }).await.unwrap();
    // This fixture has no real facility listener after the exact request.
    fixture.facility.stopped.store(true, Ordering::Release);
    drop(client); drop(permits);
}

#[test]
fn stop_after_native_dispatch_keeps_original_result_and_reply_ack() {
    struct HeldExtension { entered:mpsc::SyncSender<()>, release:Mutex<mpsc::Receiver<()>> }
    impl WindowsExecutionExtension for HeldExtension {
        fn call(&self, _:Value)->Result<Value,SystemExecutionError> { panic!("controlled native path required") }
        fn call_controlled(&self, _:Value, cancellation:WindowsCancellation)->Result<Value,SystemExecutionError> {
            cancellation.require_open()?;
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            assert!(cancellation.is_cancelled());
            Ok(json!({"original_native_result":true}))
        }
        fn shutdown(&self)->Result<(),SystemExecutionError> {Ok(())}
    }
    let (entered, entry) = mpsc::sync_channel(1); let (release, wait) = mpsc::channel();
    let id = Uuid::now_v7(); let source = Arc::new(CancellationSource::new(id).unwrap());
    let fixture = fixture(1, Arc::new(RpcDispatchSlots::new()), HashMap::from([(id,runner(Arc::new(HeldExtension {entered,release:Mutex::new(wait)})))]));
    let call = thread::spawn({ let source=source.clone();let facility=fixture.facility.clone(); move || {
        facility.exchange_controlled(id,json!({"op":"extension","request":{}}),Some(&source))
    }});
    entry.recv_timeout(Duration::from_secs(2)).unwrap();
    source.signal().unwrap();
    assert!(!call.is_finished(), "Stop cannot replace an admitted native future with a cancellation receipt");
    release.send(()).unwrap();
    assert_eq!(call.join().unwrap().unwrap()["original_native_result"],true);
    fixture.ack.recv_timeout(Duration::from_secs(2)).unwrap(); fixture.worker.join().unwrap();
}
