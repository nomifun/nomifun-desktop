//! Canonical Gateway capability descriptor and typed dispatch adapter.
//!
//! C1 removes the legacy surface/effect execution matrix. `EffectClass` is
//! descriptive metadata only; it never changes dispatch. The only
//! central pre-dispatch boundary retained here is installation ownership.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::deps::{CallerCtx, CompatibilityCapabilityHost};

pub type BoxFut = Pin<Box<dyn Future<Output = Value> + Send>>;
pub type Handler = Arc<dyn Fn(Arc<CompatibilityCapabilityHost>, CallerCtx, Value) -> BoxFut + Send + Sync>;
pub type ProgressSink = tokio::sync::mpsc::Sender<Value>;
pub type StreamingHandler =
    Arc<dyn Fn(Arc<CompatibilityCapabilityHost>, CallerCtx, Value, ProgressSink) -> BoxFut + Send + Sync>;

fn invalid_arguments_error(error: serde_json::Error) -> Value {
    json!({
        "error": "invalid_tool_arguments",
        "message": error.to_string(),
    })
}

/// Descriptive effect metadata for diagnostics and later canonical mapping.
/// It is not an execution-policy input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectClass {
    Read,
    Write,
    Destructive,
    Sensitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipScope {
    User,
    InstanceOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Desktop,
    Channel,
}

impl CallerCtx {
    pub fn surface(&self) -> Surface {
        if self.channel_platform.is_some() {
            Surface::Channel
        } else {
            Surface::Desktop
        }
    }
}

pub struct CapabilityMeta {
    pub name: &'static str,
    pub domain: &'static str,
    pub summary: &'static str,
    pub effect_class: EffectClass,
    pub ownership_scope: OwnershipScope,
}

impl CapabilityMeta {
    pub const fn new(
        name: &'static str,
        domain: &'static str,
        summary: &'static str,
        effect_class: EffectClass,
    ) -> Self {
        Self {
            name,
            domain,
            summary,
            effect_class,
            ownership_scope: OwnershipScope::User,
        }
    }

    pub const fn instance_owner(mut self) -> Self {
        self.ownership_scope = OwnershipScope::InstanceOwner;
        self
    }
}

pub struct Capability {
    pub meta: CapabilityMeta,
    pub input_schema: Map<String, Value>,
    pub handler: Handler,
    argument_validator: Arc<dyn Fn(Value) -> Result<(), serde_json::Error> + Send + Sync>,
    pub stream: Option<StreamingHandler>,
}

impl Capability {
    pub fn new<P, F, Fut>(meta: CapabilityMeta, f: F) -> Self
    where
        P: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(Arc<CompatibilityCapabilityHost>, CallerCtx, P) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Value> + Send + 'static,
    {
        let input_schema = schema_for_params::<P>();
        let f = Arc::new(f);
        let argument_validator: Arc<
            dyn Fn(Value) -> Result<(), serde_json::Error> + Send + Sync,
        > = Arc::new(|args| serde_json::from_value::<P>(args).map(|_| ()));
        let handler: Handler = Arc::new(move |deps, ctx, args| {
            let f = f.clone();
            Box::pin(async move {
                match serde_json::from_value::<P>(args) {
                    Ok(params) => f(deps, ctx, params).await,
                    Err(error) => invalid_arguments_error(error),
                }
            })
        });
        Self {
            meta,
            input_schema,
            handler,
            argument_validator,
            stream: None,
        }
    }

    pub fn new_streaming<P, F, Fut>(meta: CapabilityMeta, f: F) -> Self
    where
        P: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(Arc<CompatibilityCapabilityHost>, CallerCtx, P, ProgressSink) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Value> + Send + 'static,
    {
        let input_schema = schema_for_params::<P>();
        let f = Arc::new(f);
        let argument_validator: Arc<
            dyn Fn(Value) -> Result<(), serde_json::Error> + Send + Sync,
        > = Arc::new(|args| serde_json::from_value::<P>(args).map(|_| ()));

        let streaming = f.clone();
        let stream: StreamingHandler = Arc::new(move |deps, ctx, args, sink| {
            let f = streaming.clone();
            Box::pin(async move {
                match serde_json::from_value::<P>(args) {
                    Ok(params) => f(deps, ctx, params, sink).await,
                    Err(error) => invalid_arguments_error(error),
                }
            })
        });

        let handler: Handler = Arc::new(move |deps, ctx, args| {
            let f = f.clone();
            Box::pin(async move {
                let params = match serde_json::from_value::<P>(args) {
                    Ok(params) => params,
                    Err(error) => return invalid_arguments_error(error),
                };
                let (tx, mut rx) = tokio::sync::mpsc::channel(64);
                let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
                let result = f(deps, ctx, params, tx).await;
                drain.abort();
                result
            })
        });

        Self {
            meta,
            input_schema,
            handler,
            argument_validator,
            stream: Some(stream),
        }
    }

    pub fn validate_arguments(&self, args: &Value) -> Result<(), Value> {
        (self.argument_validator)(args.clone()).map_err(invalid_arguments_error)
    }
}

fn schema_for_params<P: JsonSchema>() -> Map<String, Value> {
    let schema = schemars::schema_for!(P);
    let value = serde_json::to_value(schema).unwrap_or_else(|_| json!({ "type": "object" }));
    let mut map = match value {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.remove("$schema");
    map.remove("title");
    map.entry("type").or_insert_with(|| json!("object"));
    let composed = ["allOf", "anyOf", "oneOf"]
        .iter()
        .any(|keyword| map.contains_key(*keyword))
        || map.contains_key("$ref");
    if composed {
        project_composed_root_properties(&mut map);
    } else {
        map.entry("additionalProperties")
            .or_insert_with(|| json!(false));
    }
    map.entry("properties").or_insert_with(|| json!({}));
    map
}

fn project_composed_root_properties(schema: &mut Map<String, Value>) {
    let root = Value::Object(schema.clone());
    let mut object_paths = BTreeSet::new();
    let mut visited_paths = BTreeSet::new();
    collect_composed_object_paths(&root, &root, "", &mut object_paths, &mut visited_paths);

    let mut properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for path in object_paths {
        let Some(branch_properties) = root
            .pointer(&path)
            .and_then(Value::as_object)
            .and_then(|branch| branch.get("properties"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (name, property) in branch_properties {
            merge_projected_property(&mut properties, name, property);
        }
    }
    schema.insert("properties".to_owned(), Value::Object(properties));
}

fn merge_projected_property(properties: &mut Map<String, Value>, name: &str, incoming: &Value) {
    let Some(existing) = properties.get(name) else {
        properties.insert(name.to_owned(), incoming.clone());
        return;
    };
    if existing == incoming {
        return;
    }
    let mut alternatives = Vec::new();
    append_unique_schema_alternatives(&mut alternatives, existing);
    append_unique_schema_alternatives(&mut alternatives, incoming);
    properties.insert(name.to_owned(), json!({ "anyOf": alternatives }));
}

fn append_unique_schema_alternatives(alternatives: &mut Vec<Value>, schema: &Value) {
    let candidates = schema
        .get("anyOf")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(schema));
    for candidate in candidates {
        if !alternatives.contains(candidate) {
            alternatives.push(candidate.clone());
        }
    }
}

fn collect_composed_object_paths(
    root: &Value,
    node: &Value,
    path: &str,
    object_paths: &mut BTreeSet<String>,
    visited_paths: &mut BTreeSet<String>,
) {
    if !visited_paths.insert(path.to_owned()) {
        return;
    }
    let Some(object) = node.as_object() else {
        return;
    };
    if schema_describes_object(object) {
        object_paths.insert(path.to_owned());
    }
    if let Some(reference) = object.get("$ref").and_then(Value::as_str)
        && let Some(pointer) = reference.strip_prefix('#')
        && let Some(target) = (pointer.is_empty() || pointer.starts_with('/'))
            .then(|| root.pointer(pointer))
            .flatten()
    {
        collect_composed_object_paths(root, target, pointer, object_paths, visited_paths);
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        let Some(branches) = object.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        for (index, branch) in branches.iter().enumerate() {
            collect_composed_object_paths(
                root,
                branch,
                &format!("{path}/{keyword}/{index}"),
                object_paths,
                visited_paths,
            );
        }
    }
}

fn schema_describes_object(schema: &Map<String, Value>) -> bool {
    let object_type = match schema.get("type") {
        Some(Value::String(kind)) => kind == "object",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "object"),
        _ => false,
    };
    object_type
        || schema.contains_key("properties")
        || schema.contains_key("required")
        || schema.contains_key("additionalProperties")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct StrictParams {
        value: String,
    }

    #[test]
    fn generated_schema_and_runtime_use_the_same_strict_request() {
        let capability = Capability::new::<StrictParams, _, _>(
            CapabilityMeta::new("test", "test", "test", EffectClass::Write),
            |_deps, _ctx, params| async move { json!({"result": params.value}) },
        );
        assert!(capability.validate_arguments(&json!({"value": "ok"})).is_ok());
        assert!(
            capability
                .validate_arguments(&json!({"value": "ok", "unexpected": true}))
                .is_err()
        );
        assert!(
            !capability
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key("unexpected"))
        );
    }
}
