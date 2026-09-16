//! Real publication -> application authorization -> committed Runtime -> unary Node invocation.
use super::*;
use nomifun_plugin_platform::runtime::{
    PluginRuntimeAgentCapabilityInvocation, PluginRuntimeAgentCapabilityPort,
    PluginRuntimeApplicationService,
};
use std::time::Duration;

const ACTION: &str = "fixture.service.values";
const SOURCE: &str = r#"
export async function start() {
  let started = 0, aborted = 0;
  return {
    async invoke({method, payload, signal, ...rest}) {
      if (method !== 'fixture.service.values') throw new Error('unexpected action');
      if (payload.mode === 'stats') return {started, aborted, unary: !('emit' in rest)};
      started++;
      if (payload.mode === 'wait') {
        return await new Promise((resolve, reject) => {
          signal.addEventListener('abort', () => {
            aborted++;
            reject(Object.assign(new Error('canceled'), {name: 'AbortError'}));
          }, {once: true});
        });
      }
      if (payload.mode === 'fail') throw new Error('unary service failure');
      return {completed: payload.count};
    },
    async dispose() {},
  };
}
"#;

fn schema_ref() -> CanonicalSchemaRef {
    format!("schema://fixture.service/value@1#{}", digest_payload(&json!({"type":"object"})).unwrap().as_ref()).into()
}

fn capability(id: &str) -> CapabilityManifest {
    let mut capability = discovery_capability(id);
    capability.id = format!("plugin.{id}.service-invocation").into();
    capability.contribution_id = format!("capability:{}", capability.id.as_ref()).into();
    capability.display.name = "Service invocation fixture".into();
    capability.contributions.actions = vec![CapabilityActionDescriptor {
        action_id: ACTION.into(), input_schema: schema_ref(), output_schema: schema_ref(),
        effect_class: EffectClass::ReadLocal, presentation: ToolPresentationKind::FunctionTool,
    }];
    capability
}

async fn stats(application: &PluginRuntimeApplicationService, base: &PluginRuntimeAgentCapabilityInvocation) -> Value {
    let mut request = base.clone();
    request.cancellation = Default::default();
    request.call_id = uuid::Uuid::now_v7().to_string().into();
    request.payload = StrictJsonValue(json!({"mode":"stats"}));
    application.invoke_agent_capability(request).await.unwrap().0
}

async fn wait_stats(application: &PluginRuntimeApplicationService, base: &PluginRuntimeAgentCapabilityInvocation, started: u64, aborted: u64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if stats(application, base).await == json!({"started":started, "aborted":aborted, "unary":true}) {
                break;
            }
            tokio::task::yield_now().await;
        }
    }).await.expect("Service counters did not settle");
}

#[tokio::test]
async fn published_service_preserves_authority_cancellation_and_disable_fences() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (id, base_path) = publish_source(&router, SOURCE, capability,
        BTreeMap::from([(schema_ref(), StrictJsonValue(json!({"type":"object"})))]),
    ).await;
    let workshop = data(&router, "GET", &format!("{base_path}/workshop"), Value::Null).await;
    let product = &workshop["plugin"];
    let active = &product["releases"]["active"];
    let release = PluginReleaseRef {
        release_id: active["release_id"].as_str().unwrap().into(),
        artifact_id: active["artifact_id"].as_str().unwrap().into(),
        release_digest: active["release_digest"].as_str().unwrap().into(),
        manifest_digest: active["manifest_digest"].as_str().unwrap().into(),
    };
    let catalog_digest = nomifun_plugin_platform::runtime::plugin_catalog_digest(
        &id, &release, &PackageContributions { capabilities: vec![capability(&id)], ..Default::default() },
    ).unwrap();
    let request = PluginRuntimeAgentCapabilityInvocation {
        cancellation: Default::default(),
        owner_user_id: services.authoritative_user_id.to_string(),
        plugin_product_id: id.clone().into(),
        capability: CapabilityRef { id: capability(&id).id, version: "1.0.0".into() },
        action_id: ACTION.into(), action_allowlist: [ACTION.into()].into(),
        active_release: release,
        active_release_epoch: product["releases"]["active_release_epoch"].as_u64().unwrap(),
        catalog_digest, operation_id: "product-service-operation".into(),
        call_id: "product-service".into(), payload: StrictJsonValue(json!({"count":5})),
    };
    let application = services.plugin_runtime.clone();
    assert_eq!(application.invoke_agent_capability(request.clone()).await.unwrap().0, json!({"completed":5}));
    wait_stats(&application, &request, 1, 0).await;

    // No rejected call may enter the actual JS handler.
    for field in ["owner", "capability", "action", "allowlist", "epoch", "catalog", "release"] {
        let mut invalid = request.clone();
        invalid.cancellation = Default::default();
        invalid.call_id = format!("denied-{field}").into();
        match field {
            "owner" => invalid.owner_user_id = uuid::Uuid::now_v7().to_string(),
            "capability" => invalid.capability.id = "plugin.missing".into(),
            "action" => invalid.action_id = "fixture.missing".into(),
            "allowlist" => invalid.action_allowlist = ["fixture.other".into()].into(),
            "epoch" => invalid.active_release_epoch += 1,
            "catalog" => invalid.catalog_digest = digest_bytes(b"stale-service-catalog"),
            "release" => invalid.active_release.release_id = uuid::Uuid::now_v7().to_string().into(),
            _ => unreachable!(),
        }
        assert!(application.invoke_agent_capability(invalid).await.is_err(), "{field}");
    }
    wait_stats(&application, &request, 1, 0).await;

    let mut canceled = request.clone();
    // This is a separate invocation, not another waiter for the base call.
    // Cloning the same cancellation handle would cancel later stats probes too.
    canceled.cancellation = Default::default();
    canceled.call_id = "future-dropped".into();
    canceled.payload = StrictJsonValue(json!({"mode":"wait"}));
    let call = {
        let application = application.clone();
        tokio::spawn(async move { application.invoke_agent_capability(canceled).await })
    };
    wait_stats(&application, &request, 2, 0).await;
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    wait_stats(&application, &request, 2, 1).await;

    let mut failing = request.clone();
    failing.cancellation = Default::default();
    failing.call_id = "unary-failure".into();
    failing.payload = StrictJsonValue(json!({"mode":"fail"}));
    let error = application.invoke_agent_capability(failing).await.unwrap_err();
    assert!(error.to_string().contains("unary service failure"), "{error}");
    wait_stats(&application, &request, 3, 1).await;

    let mut withdrawn = request.clone();
    withdrawn.cancellation = Default::default();
    withdrawn.call_id = "disabled-in-flight".into();
    withdrawn.payload = StrictJsonValue(json!({"mode":"wait"}));
    let call = {
        let application = application.clone();
        tokio::spawn(async move { application.invoke_agent_capability(withdrawn).await })
    };
    wait_stats(&application, &request, 4, 1).await;
    let workshop = data(&router, "GET", &format!("{base_path}/workshop"), Value::Null).await;
    let product = &workshop["plugin"];
    data(&router, "POST", &format!("{base_path}/enabled"), json!({
        "plugin_id":id, "expected_product_revision":product["product_revision"],
        "expected_pointer_revision":product["releases"]["pointer_revision"],
        "expected_active_release_digest":product["releases"]["active"]["release_digest"], "enabled":false,
    })).await;
    assert!(tokio::time::timeout(Duration::from_secs(5), call).await.unwrap().unwrap().is_err());
    assert!(!request.cancellation.is_canceled(), "an independent call must test the disable fence, not inherit cancellation");
    assert!(application.invoke_agent_capability(request).await.is_err());
    application.shutdown_service_runtime(services.authoritative_user_id.as_ref()).await.unwrap();
}
