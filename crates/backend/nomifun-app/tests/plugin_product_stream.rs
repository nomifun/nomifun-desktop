//! Real publication -> existing application authorization -> committed Runtime
//! -> Node event delivery. No Nomi model adapter is claimed by this fixture.
use super::*;
use nomifun_plugin_platform::runtime::{
    PluginRuntimeAgentCapabilityInvocation, PluginRuntimeAgentCapabilityPort,
    PluginRuntimeApplicationService,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

const ACTION: &str = "fixture.stream.values";
const SOURCE: &str = r#"
export async function start() {
  let started = 0, acknowledged = 0, aborted = 0;
  return {
    async invoke({method, payload, signal, emit}) {
      if (method !== 'fixture.stream.values') throw new Error('unexpected action');
      if (payload.mode === 'stats') return {started, acknowledged, aborted, unary: emit === undefined};
      started++;
      try {
        for (let index = 0; index < payload.count; index++) {
          await emit({index});
          acknowledged++;
        }
        if (payload.mode === 'fail') throw new Error('partial stream failure');
        return {completed: payload.count};
      } catch (error) {
        if (signal.aborted) aborted++;
        throw error;
      }
    },
    async dispose() {},
  };
}
"#;

fn schema_ref() -> CanonicalSchemaRef {
    format!("schema://fixture.stream/value@1#{}", digest_payload(&json!({"type":"object"})).unwrap().as_ref()).into()
}

fn capability(id: &str) -> CapabilityManifest {
    let mut capability = discovery_capability(id);
    capability.id = format!("plugin.{id}.service-stream").into();
    capability.contribution_id = format!("capability:{}", capability.id.as_ref()).into();
    capability.display.name = "Service stream fixture".into();
    capability.contributions.actions = vec![CapabilityActionDescriptor {
        action_id: ACTION.into(), input_schema: schema_ref(), output_schema: schema_ref(),
        effect_class: EffectClass::ReadLocal, presentation: ToolPresentationKind::FunctionTool,
    }];
    capability
}

async fn next(receiver: &mut mpsc::Receiver<StrictJsonValue>) -> Value {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv()).await
        .expect("event timeout").expect("event channel closed early").0
}

fn start_call(
    application: Arc<PluginRuntimeApplicationService>,
    request: PluginRuntimeAgentCapabilityInvocation,
) -> (
    mpsc::Receiver<StrictJsonValue>,
    tokio::task::JoinHandle<Result<StrictJsonValue, nomifun_plugin_platform::runtime::PluginRuntimeApplicationError>>,
) {
    let (events, receiver) = mpsc::channel(1);
    (receiver, tokio::spawn(async move {
        application.invoke_agent_capability_with_events(request, Some(events)).await
    }))
}

async fn stats(application: &PluginRuntimeApplicationService, base: &PluginRuntimeAgentCapabilityInvocation) -> Value {
    let mut request = base.clone();
    request.call_id = uuid::Uuid::now_v7().to_string().into();
    request.payload = StrictJsonValue(json!({"mode":"stats"}));
    application.invoke_agent_capability(request).await.unwrap().0
}

#[tokio::test]
async fn published_service_stream_preserves_authority_cancellation_and_disable_fences() {
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
        owner_user_id: services.authoritative_user_id.to_string(),
        plugin_product_id: id.clone().into(),
        capability: CapabilityRef { id: capability(&id).id, version: "1.0.0".into() },
        action_id: ACTION.into(), action_allowlist: [ACTION.into()].into(),
        active_release: release,
        active_release_epoch: product["releases"]["active_release_epoch"].as_u64().unwrap(),
        catalog_digest, operation_id: "product-stream-operation".into(),
        call_id: "product-stream".into(), payload: StrictJsonValue(json!({"count":5})),
    };
    let application = services.plugin_runtime.clone();
    let (mut receiver, call) = start_call(application.clone(), request.clone());
    assert_eq!(next(&mut receiver).await, json!({"index":0}));
    assert!(!call.is_finished(), "unconsumed stream completed without backpressure");
    let observed = stats(&application, &request).await;
    assert_eq!(observed["started"], 1);
    assert_eq!(observed["unary"], true, "unary invocation opted into events");
    assert!(observed["acknowledged"].as_u64().unwrap() <= 2);
    for index in 1..5 { assert_eq!(next(&mut receiver).await, json!({"index": index})); }
    assert_eq!(tokio::time::timeout(Duration::from_secs(5), call).await.unwrap().unwrap().unwrap().0,
        json!({"completed":5}));
    assert!(receiver.recv().await.is_none());

    // These are application authority checks, not a fake process accepting a
    // fabricated fence. No rejected call may enter the actual JS handler.
    for field in ["owner", "capability", "action", "allowlist", "epoch", "catalog", "release"] {
        let mut invalid = request.clone();
        invalid.call_id = format!("denied-{field}").into();
        match field {
            "owner" => invalid.owner_user_id = uuid::Uuid::now_v7().to_string(),
            "capability" => invalid.capability.id = "plugin.missing".into(),
            "action" => invalid.action_id = "fixture.missing".into(),
            "allowlist" => invalid.action_allowlist = ["fixture.other".into()].into(),
            "epoch" => invalid.active_release_epoch += 1,
            "catalog" => invalid.catalog_digest = digest_bytes(b"stale-stream-catalog"),
            "release" => invalid.active_release.release_id = uuid::Uuid::now_v7().to_string().into(),
            _ => unreachable!(),
        }
        let (events, mut denied_events) = mpsc::channel(1);
        assert!(application.invoke_agent_capability_with_events(invalid, Some(events)).await.is_err(), "{field}");
        assert!(denied_events.recv().await.is_none(), "rejected {field} emitted a value");
    }
    assert_eq!(stats(&application, &request).await["started"], 1);

    let mut canceled = request.clone();
    canceled.call_id = "receiver-dropped".into();
    canceled.payload = StrictJsonValue(json!({"count":100}));
    let (mut receiver, call) = start_call(application.clone(), canceled);
    assert_eq!(next(&mut receiver).await, json!({"index":0}));
    drop(receiver);
    assert!(tokio::time::timeout(Duration::from_secs(5), call).await.unwrap().unwrap().is_err());
    tokio::time::timeout(Duration::from_secs(5), async {
        while stats(&application, &request).await["aborted"] != 1 {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();

    let mut failing = request.clone();
    failing.call_id = "partial-failure".into();
    failing.payload = StrictJsonValue(json!({"mode":"fail", "count":1}));
    let (mut receiver, call) = start_call(application.clone(), failing);
    assert_eq!(next(&mut receiver).await, json!({"index":0}));
    let error = tokio::time::timeout(Duration::from_secs(5), call).await.unwrap().unwrap().unwrap_err();
    assert!(error.to_string().contains("partial stream failure"), "{error}");
    assert!(receiver.recv().await.is_none());
    assert_eq!(stats(&application, &request).await["started"], 3, "partial failure was replayed");

    let mut withdrawn = request.clone();
    withdrawn.call_id = "disabled-in-flight".into();
    withdrawn.payload = StrictJsonValue(json!({"count":100}));
    let (mut receiver, call) = start_call(application.clone(), withdrawn);
    assert_eq!(next(&mut receiver).await, json!({"index":0}));
    let workshop = data(&router, "GET", &format!("{base_path}/workshop"), Value::Null).await;
    let product = &workshop["plugin"];
    data(&router, "POST", &format!("{base_path}/enabled"), json!({
        "plugin_id":id, "expected_product_revision":product["product_revision"],
        "expected_pointer_revision":product["releases"]["pointer_revision"],
        "expected_active_release_digest":product["releases"]["active"]["release_digest"], "enabled":false,
    })).await;
    assert!(tokio::time::timeout(Duration::from_secs(5), call).await.unwrap().unwrap().is_err());
    tokio::time::timeout(Duration::from_secs(5), async { while receiver.recv().await.is_some() {} }).await.unwrap();
    let (events, mut receiver) = mpsc::channel(1);
    assert!(application.invoke_agent_capability_with_events(request, Some(events)).await.is_err());
    assert!(receiver.recv().await.is_none());
    application.shutdown_service_runtime(services.authoritative_user_id.as_ref()).await.unwrap();
}
