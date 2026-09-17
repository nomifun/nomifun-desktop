//! Canonical Nomi composition for the Creative Studio Wave 3 capabilities.
//!
//! This is the only application seam that connects the four Wave 3 product
//! Modules to their real domain owners.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CanonicalSchemaRef, CapabilityId, ResolvedCapability, StrictJsonValue,
};
use nomifun_agent_domain_wave3::{
    CREATION_MEDIA_MODULE_ID, CREATIVE_WORKSHOP_MODULE_ID, OFFICE_MODULE_ID,
    PLUGIN_DEVELOPMENT_MODULE_ID, Wave3CapabilityOperation, Wave3HostPort,
    Wave3HostPortError, Wave3HostRequest, Wave3OwnerBindings, composed_host_port,
};
use nomifun_agent_kernel::PluginRegistration;
use nomifun_ai_agent::NomiPlatformBuiltinToolSchemaResolver;
use nomifun_creation::CreationService;
use nomifun_office::{
    OfficeAgentError, OfficeAssetFormat, OfficeDocumentEditRequest, OfficePreviewRequest,
    OfficeRevisionDraft, OfficeSheetEditRequest, OfficeSlidesEditRequest, bounded_preview,
    build_document_revision, build_sheet_revision, build_slides_revision,
};
use nomifun_plugin_platform::runtime::PluginRuntimeApplicationService;
use nomifun_workshop::{WorkshopService, service::NewTextAsset};

use super::agent_wave3_creation_host::Wave3CreationHost;
use super::agent_wave3_plugin_host::NomiWave3PluginHost;
use super::agent_wave3_workshop_host::NomiWave3WorkshopHost;

pub(crate) const NOMI_WAVE3_MODULE_IDS: [&str; 4] = [
    CREATION_MEDIA_MODULE_ID,
    CREATIVE_WORKSHOP_MODULE_ID,
    OFFICE_MODULE_ID,
    PLUGIN_DEVELOPMENT_MODULE_ID,
];

pub(crate) fn approved_capability_ids() -> BTreeSet<CapabilityId> {
    NOMI_WAVE3_MODULE_IDS
        .into_iter()
        .map(CapabilityId::from)
        .collect()
}

/// Build host-backed registrations from the exact application service
/// singletons already used by the Creative Studio and Plugin HTTP products.
pub(crate) fn registrations(
    creation: Arc<CreationService>,
    workshop: Arc<WorkshopService>,
    plugin_runtime: Arc<PluginRuntimeApplicationService>,
    model_invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    pool: nomifun_db::SqlitePool,
    owner_id: Arc<str>,
) -> Result<Vec<PluginRegistration>, String> {
    let workshop_host = NomiWave3WorkshopHost::new(
        Arc::clone(&workshop),
        Arc::clone(&creation),
    );
    let office_host = NomiWave3OfficeHost::new(Arc::clone(&workshop));
    let host = composed_host_port(
        Wave3OwnerBindings::default()
            .with_creation(
                Wave3CreationHost::new(creation, model_invoke, pool, owner_id).into_host_port(),
            )
            .with_workshop(workshop_host.into_port())
            .with_office(office_host.into_port())
            .with_plugin(NomiWave3PluginHost::new(plugin_runtime).into_port()),
    );
    nomifun_agent_domain_wave3::registrations_with_host_port(host)
}

/// Resolve only the schema bytes belonging to the exact Wave 3 target locked
/// into a Snapshot. Source/package admission is enforced by the Nomi bundled
/// Tool bridge before calling this function.
pub(crate) fn resolve_schema(
    capability: &ResolvedCapability,
    reference: &CanonicalSchemaRef,
) -> Result<StrictJsonValue, String> {
    if !approved_capability_ids().contains(&capability.capability.id) {
        return Err(format!(
            "{} is not an approved Nomi Wave 3 Module",
            capability.capability.id.as_ref()
        ));
    }
    nomifun_agent_domain_wave3::resolve_action_schema(
        capability.capability.id.as_ref(),
        reference,
    )
}

pub(crate) struct NomiWave3SchemaResolver;

#[async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for NomiWave3SchemaResolver {
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        resolve_schema(capability, reference)
    }
}

pub(crate) fn schema_resolver() -> Arc<dyn NomiPlatformBuiltinToolSchemaResolver> {
    Arc::new(NomiWave3SchemaResolver)
}

#[derive(Clone)]
struct NomiWave3OfficeHost {
    workshop: Arc<WorkshopService>,
}

impl NomiWave3OfficeHost {
    fn new(workshop: Arc<WorkshopService>) -> Self {
        Self { workshop }
    }

    fn into_port(self) -> Arc<dyn Wave3HostPort> {
        Arc::new(self)
    }

    async fn create_revision(
        &self,
        draft: OfficeRevisionDraft,
    ) -> Result<StrictJsonValue, Wave3HostPortError> {
        if let Some(source_asset_id) = draft.source_asset_id.as_deref() {
            let source = self
                .workshop
                .get_asset(source_asset_id)
                .await
                .map_err(map_office_storage_error)?;
            if source.deleted_at.is_some() {
                return Err(Wave3HostPortError::new(
                    "OFFICE_SOURCE_UNAVAILABLE",
                    "The source asset was deleted. Choose an available asset or create a new Office item.",
                ));
            }
        }
        let format = draft.format;
        let source_asset_id = draft.source_asset_id.clone();
        let asset = self
            .workshop
            .create_text_asset(NewTextAsset {
                title: draft.title,
                text_content: draft.content,
                collection: draft.collection,
                tags: Some(format.asset_tags()),
                in_library: Some(true),
                origin: None,
            })
            .await
            .map_err(map_office_storage_error)?;
        Ok(StrictJsonValue(serde_json::json!({
            "asset_id": asset.asset_id,
            "source_asset_id": source_asset_id,
            "title": asset.title,
            "format": format.as_str(),
            "created_at": asset.created_at,
        })))
    }
}

impl Wave3HostPort for NomiWave3OfficeHost {
    fn invoke<'a>(
        &'a self,
        request: Wave3HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            request.validate()?;
            if request.context.principal.principal_kind != "user" {
                return Err(Wave3HostPortError::invalid_request(
                    "Office actions require an authenticated user owner",
                ));
            }
            match request.operation {
                Wave3CapabilityOperation::OfficePreview { input } => {
                    let input: OfficePreviewRequest = parse_office_input(input)?;
                    let asset = self
                        .workshop
                        .get_asset(&input.asset_id)
                        .await
                        .map_err(map_office_storage_error)?;
                    if asset.deleted_at.is_some() {
                        return Err(Wave3HostPortError::new(
                            "OFFICE_ASSET_UNAVAILABLE",
                            "The selected Office asset was deleted.",
                        ));
                    }
                    let content = if let Some(content) = asset.text_content.as_deref() {
                        content.to_owned()
                    } else {
                        let (bytes, mime) = self
                            .workshop
                            .read_asset_bytes(&input.asset_id)
                            .await
                            .map_err(map_office_storage_error)?;
                        if !(mime.starts_with("text/")
                            || mime == "application/json"
                            || mime == "application/csv")
                        {
                            return Err(Wave3HostPortError::new(
                                "OFFICE_PREVIEW_UNAVAILABLE",
                                "This asset has no bounded text preview. Open it in Office Preview or choose a text-based Office asset.",
                            ));
                        }
                        String::from_utf8(bytes).map_err(|_| {
                            Wave3HostPortError::new(
                                "OFFICE_PREVIEW_UNAVAILABLE",
                                "The selected asset is not valid UTF-8 text.",
                            )
                        })?
                    };
                    let preview = bounded_preview(
                        input,
                        asset.title,
                        OfficeAssetFormat::from_tags(&asset.tags),
                        &content,
                    )
                    .map_err(map_office_agent_error)?;
                    serde_json::to_value(preview)
                        .map(StrictJsonValue)
                        .map_err(|error| {
                            Wave3HostPortError::invalid_response(format!(
                                "Office preview serialization failed: {error}"
                            ))
                        })
                }
                Wave3CapabilityOperation::OfficeDocumentEdit { input } => {
                    let input = parse_office_input::<OfficeDocumentEditRequest>(input)?;
                    self.create_revision(
                        build_document_revision(input).map_err(map_office_agent_error)?,
                    )
                    .await
                }
                Wave3CapabilityOperation::OfficeSheetEdit { input } => {
                    let input = parse_office_input::<OfficeSheetEditRequest>(input)?;
                    self.create_revision(
                        build_sheet_revision(input).map_err(map_office_agent_error)?,
                    )
                    .await
                }
                Wave3CapabilityOperation::OfficeSlidesEdit { input } => {
                    let input = parse_office_input::<OfficeSlidesEditRequest>(input)?;
                    self.create_revision(
                        build_slides_revision(input).map_err(map_office_agent_error)?,
                    )
                    .await
                }
                operation => Err(Wave3HostPortError::action_operation_mismatch(format!(
                    "Office host cannot execute {}",
                    operation.action_id().as_ref()
                ))),
            }
        })
    }
}

fn parse_office_input<T: for<'de> serde::Deserialize<'de>>(
    input: StrictJsonValue,
) -> Result<T, Wave3HostPortError> {
    serde_json::from_value(input.0).map_err(|error| {
        Wave3HostPortError::invalid_request(format!("Office action input is invalid: {error}"))
    })
}

fn map_office_agent_error(error: OfficeAgentError) -> Wave3HostPortError {
    Wave3HostPortError::invalid_request(error.to_string())
}

fn map_office_storage_error(error: nomifun_common::AppError) -> Wave3HostPortError {
    let code = match error.status_code() {
        axum::http::StatusCode::NOT_FOUND => "OFFICE_ASSET_NOT_FOUND",
        axum::http::StatusCode::CONFLICT => "OFFICE_ASSET_CONFLICT",
        status if status.is_client_error() => return Wave3HostPortError::invalid_request(error.to_string()),
        _ => "OFFICE_ASSET_FAILED",
    };
    Wave3HostPortError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_set_is_exact_and_exposes_only_product_modules() {
        let approved = approved_capability_ids();
        assert_eq!(approved.len(), NOMI_WAVE3_MODULE_IDS.len());
        assert!(!approved.contains(&CapabilityId::from("workshop.director")));
        assert_eq!(
            approved,
            NOMI_WAVE3_MODULE_IDS.into_iter().map(CapabilityId::from).collect()
        );
    }
}
