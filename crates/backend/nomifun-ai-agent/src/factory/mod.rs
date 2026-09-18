//! Shared provider configuration helpers.
//!
//! Runtime construction lives exclusively in the official Driver composition;
//! this module no longer owns an alternate Agent factory.

pub mod provider_config;

use std::sync::Arc;

use futures_util::FutureExt;
use nomifun_api_types::ModelTask;
use nomifun_common::AppError;
use nomifun_model_invoke::{ModelInvokeService, ModelRef};

use self::provider_config::resolve_runtime_model_selection;
use crate::runtime_sessions::{RuntimeModelConfigResolver, RuntimeModelConfigBinding};

pub fn build_agent_model_config_resolver(
    model_invoke: Arc<ModelInvokeService>,
) -> RuntimeModelConfigResolver {
    Arc::new(move |selection| {
        let model_invoke = Arc::clone(&model_invoke);
        async move {
            let selected = resolve_runtime_model_selection(&selection)?;
            let resolved = model_invoke
                .resolve_task_config(
                    &ModelRef {
                        provider_id: selected.provider_id,
                        model: selected.model,
                    },
                    ModelTask::Chat,
                )
                .await
                .map_err(|error| AppError::BadRequest(error.to_string()))?;
            Ok(RuntimeModelConfigBinding {
                provider_id: resolved.provider_id,
                model: resolved.model,
                config_revision: resolved.config_revision,
            })
        }
        .boxed()
    })
}
