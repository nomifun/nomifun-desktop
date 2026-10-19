// Opt-in T36 native evidence. This file is included only inside services::tests
// with browser-conformance. Its exact fixture verifier proves the original
// provider/AppServices teardown seam, not canonical end-to-end Agent admission.

async fn t36_document(
    connection: &nomi_browser_engine::transport::Connection,
    session: &str,
) -> serde_json::Value {
    let mut params = chromiumoxide::cdp::js_protocol::runtime::EvaluateParams::new(
        "({epoch:window.fixtureEpoch,draft:document.getElementById('draft')?.value,focus:document.hasFocus()})",
    );
    params.return_by_value = Some(true);
    let response = connection.send(session, &params).await.unwrap();
    assert!(
        response.get("exceptionDetails").is_none(),
        "native fixture document must remain readable"
    );
    response["result"]["value"].clone()
}

async fn t36_focus(
    connection: &nomi_browser_engine::transport::Connection,
    session: &str,
    expected: bool,
) {
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            if t36_document(connection, session).await["focus"] == expected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("original task tab focus emulation did not reach its measured state");
}

async fn t36_page(
    connection: &nomi_browser_engine::transport::Connection,
    url: &str,
) -> (String, String) {
    use chromiumoxide::cdp::browser_protocol::target::{AttachToTargetParams, CreateTargetParams};
    use nomi_browser_engine::transport::ROOT_SESSION;
    let target = connection
        .send(ROOT_SESSION, &CreateTargetParams::new(url))
        .await
        .unwrap()["targetId"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut params = AttachToTargetParams::new(target.clone());
    params.flatten = Some(true);
    let session = connection.send(ROOT_SESSION, &params).await.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    connection
        .registry()
        .register_session(session.clone(), "page");
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let document = t36_document(connection, &session).await;
            if document["epoch"].as_str().is_some() && document["draft"].as_str().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("separately owned fixture tab did not load its unsaved DOM state");
    (target, session)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T36 explicit NOMIFUN_CHROME_BINARY, fresh visible external Chrome/profile, real AppServices shutdown"]
async fn real_app_quit_detaches_attached_chrome_without_closing_or_reloading_original_pages() {
    use chromiumoxide::cdp::browser_protocol::target::{ActivateTargetParams, GetTargetsParams};
    use futures_util::FutureExt;
    use nomi_browser_engine::launch::{LaunchConfig, launch_chrome};
    use nomi_browser_engine::transport::ROOT_SESSION;
    use nomifun_browser_platform::attached_browser::{
        AttachedBrowserCommand, AttachedBrowserProviderHost, AttachedBrowserRuntimeError,
    };

    struct ExactFixtureOwner {
        owner: String,
        session: String,
    }
    #[async_trait::async_trait]
    impl crate::browser_workspace_provider::attached_provider::AttachedBrowserSessionVerifier
        for ExactFixtureOwner
    {
        async fn verify(
            &self,
            owner: &str,
            session: &str,
        ) -> Result<(), AttachedBrowserRuntimeError> {
            if owner == self.owner && session == self.session {
                Ok(())
            } else {
                Err(AttachedBrowserRuntimeError::ActionDenied)
            }
        }
    }

    let chrome = std::path::PathBuf::from(
        std::env::var_os("NOMIFUN_CHROME_BINARY")
            .expect("explicit isolated fixture Chrome binary required"),
    );
    assert!(
        chrome.is_absolute() && chrome.is_file(),
        "Chrome must be supplied explicitly; no discovery/install is permitted"
    );
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("external-chrome-profile");
    let launched = launch_chrome(
        &LaunchConfig {
            chrome_path: chrome,
            user_data_dir: profile.clone(),
            headful: true,
        },
        false,
    )
    .await
    .unwrap();
    let (mut external_owner, diagnostics) = launched.connect().await.unwrap();
    let external_pid = external_owner.child_mut().id().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let html = |label: &str| {
        format!(
            "<!doctype html><meta charset=utf-8><title>{label}</title><h1>{label}</h1><textarea id=draft aria-label='Unsaved draft'></textarea><button>Keep original page</button><script>window.fixtureEpoch=crypto.randomUUID();draft.value='UNSAVED_{label}_'+fixtureEpoch;</script>"
        )
    };
    let user = html("T36 original user page");
    let task = html("T36 original task page");
    let router = axum::Router::new()
        .route(
            "/user",
            axum::routing::get(move || {
                let html = user.clone();
                async move { axum::response::Html(html) }
            }),
        )
        .route(
            "/task",
            axum::routing::get(move || {
                let html = task.clone();
                async move { axum::response::Html(html) }
            }),
        );
    let server_stop = CancellationToken::new();
    let server_end = server_stop.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_end.cancelled_owned())
            .await
            .unwrap();
    });
    let mut app_services: Option<AppServices> = None;
    let verification=std::panic::AssertUnwindSafe(async {
        let (user_target,user_session)=t36_page(&diagnostics,&format!("http://127.0.0.1:{port}/user")).await;
        let (task_target,task_session)=t36_page(&diagnostics,&format!("http://127.0.0.1:{port}/task")).await;
        diagnostics.send(ROOT_SESSION,&ActivateTargetParams::new(user_target.clone())).await.unwrap();
        t36_focus(&diagnostics,&task_session,false).await;
        let user_before=t36_document(&diagnostics,&user_session).await;
        let task_before=t36_document(&diagnostics,&task_session).await;

        let data=directory.path().join("app-data");std::fs::create_dir_all(&data).unwrap();
        let sentinel=data.join("persistent-data-sentinel");std::fs::write(&sentinel,b"preserve fixture data after App Quit").unwrap();
        let config=test_config(&data);let database=nomifun_db::init_database(&config.database_path()).await.unwrap();
        app_services=Some(AppServices::from_config(database,&config).await.unwrap());
        let services=app_services.as_mut().unwrap();let owner=services.authoritative_user_id.to_string();
        let session=uuid::Uuid::now_v7().to_string();
        let attached=crate::AttachedChromeProviderService::with_conformance_port_file(profile.join("DevToolsActivePort"));
        attached.install_session_verifier(Arc::new(ExactFixtureOwner {owner:owner.clone(),session:session.clone()})).unwrap();
        services.attached_chrome=Some(attached.clone());
        let connected=attached.connect(&owner).await.unwrap();
        let resource=attached.resource(&owner,&session).await.unwrap();let turn=resource.begin_run().await.unwrap();
        let tabs=turn.invoke(AttachedBrowserCommand::Tabs {}).await.unwrap();
        let tab=tabs["tabs"].as_array().unwrap().iter().find(|tab|tab["title"]=="T36 original task page").unwrap()["tab_id"].as_str().unwrap().to_owned();
        let observation=turn.invoke(AttachedBrowserCommand::Observe {tab_id:tab.clone()}).await.unwrap();
        assert!(observation["observation_id"].as_str().is_some(),"a real native observation must acquire the original tab claim");
        // Observe intentionally does not arm focus emulation. One benign native
        // Escape on our fixture tab exercises the real active override owner.
        turn.invoke(AttachedBrowserCommand::Press {tab_id:tab.clone(),observation_id:observation["observation_id"].as_str().unwrap().into(),keys:"Escape".into()}).await.unwrap();
        assert_eq!(attached.conformance_claim_counts(),(1,1));
        // Keep the other page foreground: a true result on the background task
        // now measures this provider's emulation, not accidental OS activation.
        diagnostics.send(ROOT_SESSION,&ActivateTargetParams::new(user_target.clone())).await.unwrap();
        t36_focus(&diagnostics,&task_session,true).await;
        assert_eq!(attached.snapshot(&owner).unwrap().unwrap().incarnation,connected.incarnation);

        tokio::time::timeout(std::time::Duration::from_secs(15),services.shutdown_nomi_core_host()).await.unwrap().unwrap();
        assert!(services.database.pool().is_closed(),"actual AppServices storage shutdown must complete");
        assert!(attached.snapshot(&owner).unwrap().is_none(),"the application releases its provider socket");
        assert_eq!(attached.conformance_claim_counts(),(0,0),"original runs/claims are drained during App Quit");
        assert!(attached.connect(&owner).await.is_err(),"application Quit seals reconnect admission");
        assert!(turn.invoke(AttachedBrowserCommand::Observe {tab_id:tab}).await.is_err(),"a retained old Run cannot reuse the disconnected provider");
        assert!(external_owner.child_mut().try_wait().unwrap().is_none(),"App Quit must preserve the externally owned Chrome process");
        assert!(profile.is_dir()&&profile.join("DevToolsActivePort").is_file(),"App Quit cannot remove the external browser Profile/connection marker");
        assert_eq!(std::fs::read(&sentinel).unwrap(),b"preserve fixture data after App Quit");
        let targets=diagnostics.send(ROOT_SESSION,&GetTargetsParams::default()).await.unwrap();
        for original in [&user_target,&task_target] {assert!(targets["targetInfos"].as_array().unwrap().iter().any(|target|target["targetId"]==original.as_str()),"App Quit closed an original browser tab");}
        t36_focus(&diagnostics,&task_session,false).await;
        let user_after=t36_document(&diagnostics,&user_session).await;let task_after=t36_document(&diagnostics,&task_session).await;
        for (before,after) in [(&user_before,&user_after),(&task_before,&task_after)] {
            assert_eq!(before["epoch"],after["epoch"],"original page must not reload");
            assert_eq!(before["draft"],after["draft"],"unsaved DOM state must survive App Quit");
        }
        println!("T36_ATTACHED_CHROME_APP_QUIT external_pid={external_pid} external_browser_alive=true original_pages=2 original_dom_preserved=true claims_drained=true focus_override_reset=true app_storage_closed=true verifier=exact_fixture_owner_not_canonical_end_to_end");
    }).catch_unwind().await;

    // The external launch guard remains the sole process/profile owner until
    // all preservation observations finish, including when a fixture fails.
    if let Some(services) = app_services {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            services.shutdown_nomi_core_host(),
        )
        .await;
    }
    diagnostics.shutdown().await;
    let external_cleanup = external_owner.shutdown().await;
    server_stop.cancel();
    if tokio::time::timeout(std::time::Duration::from_secs(5), &mut server)
        .await
        .is_err()
    {
        server.abort();
        let _ = server.await;
    }
    external_cleanup.unwrap();
    if let Err(reason) = verification {
        std::panic::resume_unwind(reason);
    }
}
