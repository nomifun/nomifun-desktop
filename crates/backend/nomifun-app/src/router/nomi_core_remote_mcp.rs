//! Streamable HTTP MCP transport for the current Nomi-core Remote owner.
//!
//! The public transport owns MCP session admission and installation-token
//! authentication. This module only adapts the existing Nomi-core Remote
//! handlers, so MCP and REST share one product state machine.

use std::sync::Arc;

use axum::Router;
use nomifun_api_types::{
    RemoteCancelRequestDto, RemoteObserveRequestDto, RemoteOpenRequestDto, RemoteTurnRequestDto,
};
use nomifun_auth::InstanceTokenValidator;
use nomifun_common::UserId;
use nomifun_public::{
    CanonicalRemoteOperationFuture, CanonicalRemoteOperations, canonical_remote_mcp_router_with_operations,
};

use super::nomi_core_session::{
    NomiCoreAgentApiState, run_nomi_core_remote_cancel, run_nomi_core_remote_observe,
    run_nomi_core_remote_open, run_nomi_core_remote_turn,
};

#[derive(Clone)]
struct NomiCoreRemoteOperations {
    state: NomiCoreAgentApiState,
}

impl CanonicalRemoteOperations for NomiCoreRemoteOperations {
    fn open<'a>(
        &'a self,
        owner: &'a UserId,
        request: RemoteOpenRequestDto,
    ) -> CanonicalRemoteOperationFuture<'a> {
        let state = self.state.clone();
        let owner = owner.clone();
        Box::pin(async move { run_nomi_core_remote_open(state, &owner, request).await })
    }

    fn turn<'a>(
        &'a self,
        owner: &'a UserId,
        request: RemoteTurnRequestDto,
    ) -> CanonicalRemoteOperationFuture<'a> {
        let state = self.state.clone();
        let owner = owner.clone();
        Box::pin(async move { run_nomi_core_remote_turn(state, &owner, request).await })
    }

    fn observe<'a>(
        &'a self,
        owner: &'a UserId,
        request: RemoteObserveRequestDto,
    ) -> CanonicalRemoteOperationFuture<'a> {
        let state = self.state.clone();
        let owner = owner.clone();
        Box::pin(async move { run_nomi_core_remote_observe(state, &owner, request).await })
    }

    fn cancel<'a>(
        &'a self,
        owner: &'a UserId,
        request: RemoteCancelRequestDto,
    ) -> CanonicalRemoteOperationFuture<'a> {
        let state = self.state.clone();
        let owner = owner.clone();
        Box::pin(async move { run_nomi_core_remote_cancel(state, &owner, request).await })
    }
}

pub(crate) fn build(
    state: NomiCoreAgentApiState,
    validator: Arc<InstanceTokenValidator>,
    owner_id: Arc<str>,
) -> Router {
    let owner = UserId::parse(owner_id.as_ref().to_owned())
        .unwrap_or_else(|error| panic!("Nomi-core Remote owner id is invalid: {error}"));
    canonical_remote_mcp_router_with_operations(
        Arc::new(NomiCoreRemoteOperations { state }),
        validator,
        owner,
    )
}
