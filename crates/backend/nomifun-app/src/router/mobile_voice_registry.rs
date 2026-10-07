//! Optional Mobile voice discovery/factories. This never edits Main model
//! capabilities, defaults, Agent Presets, Snapshots or provider settings.
use nomifun_model_invoke::voice::{self, VoiceConnectionLease};
use nomifun_model_invoke::{AuthScheme, ModelInvokeService, ModelRef};
use nomifun_voice::{VoiceAdapterRegistration, VoiceAdapterRegistry};
use nomifun_voice_contracts::{
    DigestHex, VoiceProfileUpdate, digest_bytes, digest_payload, voice::*,
};
use nomifun_voice_core::VoiceModelPort;
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::BTreeMap;
use std::sync::Arc;

type Builder = fn(
    nomifun_model_invoke::ResolvedConnection,
    Value,
    String,
) -> Result<Arc<dyn VoiceModelPort>, VoiceError>;
pub(crate) struct AppVoiceRegistry {
    pub(crate) registry: Arc<VoiceAdapterRegistry>,
    invoke: Arc<ModelInvokeService>,
    pool: sqlx::SqlitePool,
    owner: Arc<str>,
}
fn config_error(message: impl std::fmt::Display) -> VoiceError {
    VoiceError::new(VoiceErrorKind::Configuration, message.to_string())
}
fn stale(message: &str) -> VoiceError {
    VoiceError::new(VoiceErrorKind::StaleBinding, message)
}
fn auth_scheme(scheme: &AuthScheme) -> Value {
    match scheme {
        AuthScheme::Bearer => json!("bearer"),
        AuthScheme::TokenHeader => json!("token"),
        AuthScheme::Bedrock => json!("bedrock"),
        AuthScheme::HeaderKey(name) => json!({"header_key":name.to_ascii_lowercase()}),
        AuthScheme::QueryKey(name) => json!({"query_key":name}),
        AuthScheme::MultiHeader(fields) => json!({"multi_header":fields}),
    }
}
fn connection_digest(
    lease: &VoiceConnectionLease,
    adapter_id: &str,
    config: &Value,
    transport: VoiceTransportPreference,
) -> Result<DigestHex, VoiceError> {
    digest_payload(&json!({"provider_id":lease.provider_id,"model":lease.model,"role":lease.connection.role,
        "base_url":lease.connection.base_url,"auth_scheme":auth_scheme(&lease.connection.auth.scheme),"extra":lease.connection.extra,
        "bedrock_config":lease.bedrock_config,"adapter_id":adapter_id,"adapter_config":config,"transport":transport})).map_err(config_error)
}
fn connection_role(record: &VoiceRouteRecord) -> Result<&str, VoiceError> {
    let prefix = format!("provider:{}:", record.provider_id);
    let role = record
        .connection_config_ref
        .strip_prefix(&prefix)
        .filter(|role| !role.is_empty())
        .ok_or_else(|| stale("voice connection reference is not bound to its provider"))?;
    if record.credential_ref != record.connection_config_ref {
        return Err(stale(
            "voice credential reference differs from its frozen connection",
        ));
    }
    Ok(role)
}
impl AppVoiceRegistry {
    /// Builds registration closures only; no database access or network probe.
    pub(crate) fn new(
        invoke: Arc<ModelInvokeService>,
        pool: sqlx::SqlitePool,
        owner: Arc<str>,
    ) -> Result<Arc<Self>, VoiceError> {
        let mut entries = Vec::new();
        for (id, describe, schema, projection, validate, builder) in [
            (
                voice::STEPFUN_VOICE_ADAPTER_ID,
                voice::stepfun_descriptor as fn(&str) -> VoiceAdapterDescriptor,
                voice::stepfun_config_schema(),
                voice::stepfun_protocol_descriptor(),
                voice::validate_stepfun_config as fn(&Value) -> Result<(), String>,
                voice::create_stepfun_model_port as Builder,
            ),
            (
                voice::OPENAI_LIVE_ADAPTER_ID,
                voice::openai_live_descriptor as fn(&str) -> VoiceAdapterDescriptor,
                voice::openai_live_config_schema(),
                voice::openai_live_protocol_descriptor(),
                voice::validate_openai_live_config as fn(&Value) -> Result<(), String>,
                voice::create_openai_live_model_port as Builder,
            ),
        ] {
            let factory_invoke = invoke.clone();
            entries.push(VoiceAdapterRegistration{adapter_id:id.into(),contract_version:VOICE_CONTRACT_VERSION,config_schema:schema,
                catalog_projection:serde_json::to_value(projection).map_err(config_error)?,describe:Arc::new(describe),
                validate:Arc::new(move|value|validate(value).map_err(config_error)),factory:Arc::new(move|record|{
                    let invoke=factory_invoke.clone();Box::pin(async move{
                        record.validate().map_err(config_error)?;
                        if record.adapter_id!=id{return Err(stale("voice factory identity differs from frozen adapter"));}
                        validate(&record.adapter_config).map_err(config_error)?;
                        let descriptor=describe(&record.model);validate_native_duplex(&descriptor.capabilities).map_err(config_error)?;
                        if !descriptor.transports.contains(&record.transport){return Err(stale("voice transport is not supported by the selected adapter"));}
                        let role=connection_role(&record)?;
                        let lease=invoke.resolve_voice_connection_lease(&ModelRef{provider_id:record.provider_id.clone(),model:record.model.clone()},role,None).await.map_err(config_error)?;
                        if connection_digest(&lease,id,&record.adapter_config,record.transport)?!=record.connection_config_digest{
                            return Err(stale("frozen voice connection/model/configuration changed; update only its VoiceProfile"));
                        }
                        builder(lease.connection,record.adapter_config,record.model)
                    })
                })});
        }
        Ok(Arc::new(Self {
            registry: Arc::new(VoiceAdapterRegistry::new(entries)?),
            invoke,
            pool,
            owner,
        }))
    }
    pub(crate) async fn resolve_record(
        &self,
        selection: &VoiceProfileUpdate,
        revision: u64,
    ) -> Result<VoiceRouteRecord, VoiceError> {
        if revision == 0 || selection.binding_version == 0 {
            return Err(config_error(
                "voice profile and binding revisions must be positive",
            ));
        }
        let registration = if let Some(id) = &selection.adapter_id {
            self.registry.registration(id)?
        } else {
            let mut candidates = self.registry.entries().filter(|entry| {
                validate_native_duplex(&(entry.describe)(&selection.model).capabilities).is_ok()
            });
            let entry = candidates.next().ok_or_else(|| {
                VoiceError::new(
                    VoiceErrorKind::Unsupported,
                    "no registered adapter has verified native duplex support for this model",
                )
            })?;
            if candidates.next().is_some() {
                return Err(config_error(
                    "multiple verified adapters support this model; select one explicitly",
                ));
            }
            entry
        };
        (registration.validate)(&selection.adapter_config)?;
        let descriptor = (registration.describe)(&selection.model);
        validate_native_duplex(&descriptor.capabilities)
            .map_err(|message| VoiceError::new(VoiceErrorKind::Unsupported, message))?;
        if !descriptor.transports.contains(&selection.transport) {
            return Err(VoiceError::new(
                VoiceErrorKind::Unsupported,
                "selected voice transport is not supported",
            ));
        }
        let mut required = native_duplex_requirements();
        let work = if descriptor
            .capabilities
            .get(&VoiceFeature::TypedTools)
            .is_some_and(|e| e.support == CapabilitySupport::Supported)
        {
            VoiceFeature::TypedTools
        } else {
            VoiceFeature::Delegation
        };
        required.insert(work);
        let lease = self
            .invoke
            .resolve_voice_connection_lease(
                &ModelRef {
                    provider_id: selection.provider_id.clone(),
                    model: selection.model.clone(),
                },
                &selection.connection_role,
                None,
            )
            .await
            .map_err(config_error)?;
        let model_revision = u64::try_from(lease.config_revision)
            .map_err(|_| config_error("voice provider configuration revision is negative"))?;
        let reference = format!("provider:{}:{}", lease.provider_id, lease.connection.role);
        let record = VoiceRouteRecord {
            schema: VOICE_ROUTE_SCHEMA.into(),
            route_id: format!(
                "mobile.voice:{}:{}",
                self.owner,
                selection.agent_session_id.as_ref()
            ),
            revision,
            provider_id: lease.provider_id.clone(),
            model: lease.model.clone(),
            model_revision,
            connection_config_ref: reference.clone(),
            credential_ref: reference,
            adapter_id: registration.adapter_id.clone(),
            adapter_contract_version: registration.contract_version,
            adapter_config: selection.adapter_config.clone(),
            adapter_config_digest: digest_payload(&selection.adapter_config)
                .map_err(config_error)?,
            connection_config_digest: connection_digest(
                &lease,
                &registration.adapter_id,
                &selection.adapter_config,
                selection.transport,
            )?,
            required_features: required,
            transport: selection.transport,
        };
        self.registry.validate_route(&record)?;
        // Constructor validation verifies scheme, complete credentials and
        // credential origin without opening a socket or charging a model.
        self.registry.create(&record).await?;
        Ok(record)
    }
    pub(crate) async fn lease_revision(
        &self,
        record: &VoiceRouteRecord,
    ) -> Result<String, VoiceError> {
        self.registry.validate_route(record)?;
        let role = connection_role(record)?;
        for _ in 0..3 {
            let lease = self
                .invoke
                .resolve_voice_connection_lease(
                    &ModelRef {
                        provider_id: record.provider_id.clone(),
                        model: record.model.clone(),
                    },
                    role,
                    None,
                )
                .await
                .map_err(config_error)?;
            if connection_digest(
                &lease,
                &record.adapter_id,
                &record.adapter_config,
                record.transport,
            )? != record.connection_config_digest
            {
                return Err(stale(
                    "frozen voice nonsecret connection configuration changed",
                ));
            }
            let row=if role=="default"{
                sqlx::query("SELECT config_revision,credentials_encrypted FROM providers WHERE provider_id=? AND enabled=1").bind(&record.provider_id).fetch_optional(&self.pool).await
            }else{
                sqlx::query("SELECT p.config_revision,c.credentials_encrypted FROM provider_connections c JOIN providers p ON p.provider_id=c.provider_id WHERE p.provider_id=? AND p.enabled=1 AND c.role=?")
                    .bind(&record.provider_id).bind(role).fetch_optional(&self.pool).await
            }.map_err(|_|config_error("voice credential reference could not be read"))?.ok_or_else(||VoiceError::new(VoiceErrorKind::Authentication,"voice credential reference was revoked"))?;
            let revision: i64 = row
                .try_get("config_revision")
                .map_err(|_| config_error("voice credential revision is invalid"))?;
            if revision != lease.config_revision {
                continue;
            }
            let encrypted: String = row
                .try_get("credentials_encrypted")
                .map_err(|_| config_error("voice credential reference is invalid"))?;
            return Ok(digest_bytes(encrypted.as_bytes()).0);
        }
        Err(stale(
            "voice credential lease changed repeatedly during read",
        ))
    }
    pub(crate) async fn catalog(&self) -> Result<Value, VoiceError> {
        let rows=sqlx::query("SELECT m.provider_id,m.model,m.display_name,p.name AS provider_label,p.base_url,p.auth_scheme FROM provider_models m JOIN providers p ON p.provider_id=m.provider_id WHERE m.enabled=1 AND p.enabled=1 ORDER BY p.sort_order,m.sort_order,m.model")
            .fetch_all(&self.pool).await.map_err(|_|config_error("voice model catalog could not be read"))?;
        let mut roles_by_provider = BTreeMap::<String, Vec<Value>>::new();
        let mut models = Vec::new();
        for row in rows {
            let provider_id: String = row.try_get("provider_id").map_err(config_error)?;
            let model: String = row.try_get("model").map_err(config_error)?;
            let roles = if let Some(roles) = roles_by_provider.get(&provider_id) {
                roles.clone()
            } else {
                let base: String = row.try_get("base_url").map_err(config_error)?;
                let scheme: String = row.try_get("auth_scheme").map_err(config_error)?;
                let mut roles = vec![
                    json!({"role":"default","label":"Default","base_url":base,"auth_scheme":scheme}),
                ];
                for role in sqlx::query("SELECT role,label,base_url,auth_scheme FROM provider_connections WHERE provider_id=? ORDER BY role").bind(&provider_id).fetch_all(&self.pool).await.map_err(|_|config_error("voice connection roles could not be read"))?{
                    roles.push(json!({"role":role.try_get::<String,_>("role").map_err(config_error)?,"label":role.try_get::<Option<String>,_>("label").map_err(config_error)?,
                        "base_url":role.try_get::<String,_>("base_url").map_err(config_error)?,"auth_scheme":role.try_get::<String,_>("auth_scheme").map_err(config_error)?}));
                }
                roles_by_provider.insert(provider_id.clone(), roles.clone());
                roles
            };
            let adapters=self.registry.entries().map(|entry|{
                let descriptor=(entry.describe)(&model);let support=if validate_native_duplex(&descriptor.capabilities).is_ok(){CapabilitySupport::Supported}else if descriptor.capabilities.values().any(|e|e.support==CapabilitySupport::Unknown){CapabilitySupport::Unknown}else{CapabilitySupport::Unsupported};
                json!({"adapter_id":entry.adapter_id,"label":descriptor.label,"support":support,"descriptor":descriptor,"config_schema":entry.config_schema})
            }).collect::<Vec<_>>();
            // Product selection consumes role identities. Keep optional
            // display metadata separate rather than returning objects where
            // the actual Mobile contract requires string candidates.
            let role_ids = roles
                .iter()
                .map(|role| {
                    role.get("role")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or_else(|| config_error("voice connection role identity is invalid"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            models.push(json!({"provider_id":provider_id,"provider_label":row.try_get::<String,_>("provider_label").map_err(config_error)?,"model":model,
                "label":row.try_get::<Option<String>,_>("display_name").map_err(config_error)?.unwrap_or_else(||model.clone()),"connection_roles":role_ids,"connection_role_details":roles,"adapters":adapters}));
        }
        let adapters=self.registry.entries().map(|entry|json!({"adapter_id":entry.adapter_id,"contract_version":entry.contract_version,"config_schema":entry.config_schema,"catalog_projection":entry.catalog_projection})).collect::<Vec<_>>();
        Ok(json!({"contract_version":VOICE_CONTRACT_VERSION,"models":models,"adapters":adapters}))
    }
}

#[cfg(test)]
#[path = "mobile_voice_registry/tests.rs"]
mod tests;
