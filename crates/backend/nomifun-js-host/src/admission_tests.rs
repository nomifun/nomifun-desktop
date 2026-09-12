use super::*;

#[test]
fn inflight_limits_must_be_nonzero() {
    let valid = JavaScriptHostLimits::default();
    assert!(valid.validate().is_ok());
    for limits in [
        JavaScriptHostLimits {
            max_pending_requests: 0,
            ..valid.clone()
        },
        JavaScriptHostLimits {
            max_service_requests: 0,
            ..valid
        },
    ] {
        assert!(matches!(
            limits.validate(),
            Err(JavaScriptHostError::InvalidConfiguration(_))
        ));
    }
}

#[tokio::test]
async fn command_queue_admission_timeout_does_not_enqueue_later() {
    let (handle, mut receiver) = running_handle();
    let (reply, _response) = oneshot::channel();
    handle
        .commands
        .try_send(ActorCommand::Stop {
            expected_generation: 1,
            reply,
        })
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        handle.request(
            &PluginN1ContractManifest::canonical(),
            PluginHostRequest::HostShutdown,
            None,
            Duration::from_millis(30),
        ),
    )
    .await
    .expect("admission must be bounded");
    assert!(matches!(result, Err(JavaScriptHostError::AdmissionTimeout)));
    assert!(matches!(receiver.try_recv(), Ok(ActorCommand::Stop { .. })));
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

fn running_handle() -> (GenerationHandle, mpsc::Receiver<ActorCommand>) {
    let (commands, receiver) = mpsc::channel(1);
    let (_state, state) = watch::channel(JavaScriptHostState::Running {
        generation: 1,
        process_id: 1,
    });
    let handle = GenerationHandle {
        host_kind: JavaScriptHostKind::SharedExtension,
        generation: 1,
        commands,
        state,
        mounts: Arc::new(RwLock::new(BTreeMap::new())),
        admission: Arc::new(RwLock::new(())),
    };
    (handle, receiver)
}

#[tokio::test]
async fn mount_fence_waits_for_prior_submitters_before_enqueuing() {
    let (handle, mut receiver) = running_handle();
    let submitting = handle.admission.read().await;
    let mut fence = Box::pin(
        handle.commit_fence_for_mount(PluginMountId::from("mount-a"), Duration::from_secs(1)),
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut fence)
            .await
            .is_err()
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    drop(submitting);
    let responder = async {
        let Some(ActorCommand::MountFence { mount_id, reply }) = receiver.recv().await else {
            panic!("expected Mount fence after admission drained");
        };
        assert_eq!(mount_id.as_ref(), "mount-a");
        reply.send(Ok(PluginHostCommitFence::NotResident)).unwrap();
    };
    let (result, ()) = tokio::join!(fence, responder);
    assert_eq!(result.unwrap(), PluginHostCommitFence::NotResident);
}

#[tokio::test]
async fn mount_fence_queue_and_response_waits_are_bounded() {
    for full_queue in [true, false] {
        let (handle, mut receiver) = running_handle();
        if full_queue {
            let (reply, _) = oneshot::channel();
            handle
                .commands
                .try_send(ActorCommand::Stop {
                    expected_generation: 1,
                    reply,
                })
                .unwrap();
        }
        let result = handle
            .commit_fence_for_mount(PluginMountId::from("mount-a"), Duration::from_millis(30))
            .await;
        assert!(matches!(result, Err(JavaScriptHostError::AdmissionTimeout)));
        let command = receiver.try_recv().unwrap();
        if full_queue {
            assert!(matches!(command, ActorCommand::Stop { .. }));
        } else {
            let ActorCommand::MountFence { reply, .. } = command else {
                panic!("expected fence");
            };
            assert!(
                reply.is_closed(),
                "timed-out observer is no longer retained"
            );
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(handle.admission.try_read().is_ok());
    }
}
