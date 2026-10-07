//! Product-level Knowledge Module owner.
//!
//! Agent authoring exposes four actions and nothing about the retrieval
//! implementation behind them. Embedding, reranking, mount reconciliation,
//! source synchronization, and model selection remain private
//! [`KnowledgeService`] concerns. This adapter also keeps the resource half of
//! authorization at the final owner boundary: a Kernel action grant is not
//! sufficient unless the exact Knowledge resource grants the required
//! operation to the same principal.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use nomifun_common::{AppError, KnowledgeBaseId};
use serde::Serialize;

use crate::{
    BoundKnowledgeBase, BoundKnowledgeDocument, BoundKnowledgeReadService,
    BoundKnowledgeSearchHit, KnowledgeService, WriteMode, WriteOp, WriteOutcome, WritePolicy,
    WriteRequest, WriteSurface, WriteTargetSpec, decode_doc_handle, encode_doc_handle,
};

pub const KNOWLEDGE_MODULE_ID: &str = "knowledge";
pub const KNOWLEDGE_SEARCH_ACTION_ID: &str = "knowledge/search";
pub const KNOWLEDGE_READ_ACTION_ID: &str = "knowledge/read";
pub const KNOWLEDGE_WRITE_ACTION_ID: &str = "knowledge/write";
pub const KNOWLEDGE_AUTOGEN_ACTION_ID: &str = "knowledge/autogen";

pub const KNOWLEDGE_ACTION_IDS: [&str; 4] = [
    KNOWLEDGE_SEARCH_ACTION_ID,
    KNOWLEDGE_READ_ACTION_ID,
    KNOWLEDGE_WRITE_ACTION_ID,
    KNOWLEDGE_AUTOGEN_ACTION_ID,
];

/// Product actions an Agent may receive for the Knowledge Module.
///
/// Provider and lifecycle operations intentionally have no variant here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KnowledgeAction {
    Search,
    Read,
    Write,
    Autogen,
}

impl KnowledgeAction {
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::Search => KNOWLEDGE_SEARCH_ACTION_ID,
            Self::Read => KNOWLEDGE_READ_ACTION_ID,
            Self::Write => KNOWLEDGE_WRITE_ACTION_ID,
            Self::Autogen => KNOWLEDGE_AUTOGEN_ACTION_ID,
        }
    }

    pub const fn required_resource_operation(self) -> KnowledgeResourceOperation {
        match self {
            Self::Search => KnowledgeResourceOperation::Search,
            Self::Read => KnowledgeResourceOperation::Read,
            Self::Write | Self::Autogen => KnowledgeResourceOperation::Write,
        }
    }

    pub fn from_action_id(action_id: &str) -> Option<Self> {
        match action_id {
            KNOWLEDGE_SEARCH_ACTION_ID => Some(Self::Search),
            KNOWLEDGE_READ_ACTION_ID => Some(Self::Read),
            KNOWLEDGE_WRITE_ACTION_ID => Some(Self::Write),
            KNOWLEDGE_AUTOGEN_ACTION_ID => Some(Self::Autogen),
            _ => None,
        }
    }
}

/// Operations a concrete `knowledge_base` binding may grant.
///
/// Mounting and retrieval-provider mechanics are deliberately absent. They
/// cannot be smuggled back into an Agent grant through this owner API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KnowledgeResourceOperation {
    Search,
    Read,
    Write,
}

impl KnowledgeResourceOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Read => "read",
            Self::Write => "write",
        }
    }

    pub fn parse(operation: &str) -> Option<Self> {
        match operation {
            "search" => Some(Self::Search),
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            _ => None,
        }
    }
}

/// One exact, owner-scoped Knowledge resource selected for an AgentSession.
#[derive(Clone, Debug)]
pub struct AgentKnowledgeResource {
    binding_id: String,
    owner_id: String,
    knowledge_base_id: KnowledgeBaseId,
    name: String,
    root: PathBuf,
    operations: BTreeSet<KnowledgeResourceOperation>,
}

impl AgentKnowledgeResource {
    pub fn from_operation_names<I, S>(
        binding_id: impl Into<String>,
        owner_id: impl Into<String>,
        knowledge_base_id: KnowledgeBaseId,
        name: impl Into<String>,
        root: impl Into<PathBuf>,
        operations: I,
    ) -> Result<Self, AppError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let operations = operations
            .into_iter()
            .map(|operation| {
                KnowledgeResourceOperation::parse(operation.as_ref()).ok_or_else(|| {
                    AppError::BadRequest(
                        "knowledge resource binding contains a non-product operation".into(),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(
            binding_id,
            owner_id,
            knowledge_base_id,
            name,
            root,
            operations,
        )
    }

    pub fn new(
        binding_id: impl Into<String>,
        owner_id: impl Into<String>,
        knowledge_base_id: KnowledgeBaseId,
        name: impl Into<String>,
        root: impl Into<PathBuf>,
        operations: impl IntoIterator<Item = KnowledgeResourceOperation>,
    ) -> Result<Self, AppError> {
        let binding_id = binding_id.into();
        let owner_id = owner_id.into();
        let name = name.into();
        let root = root.into();
        if binding_id.trim().is_empty() || owner_id.trim().is_empty() {
            return Err(AppError::BadRequest(
                "knowledge resource binding and owner identities must not be blank".into(),
            ));
        }
        // Reuse the hardened root and display-name validation used by direct
        // binding-backed reads. The handle is reconstructed per operation so
        // root replacement is still checked at the point of use.
        BoundKnowledgeBase::new(knowledge_base_id.clone(), name.clone(), root.clone())?;
        let operations = operations.into_iter().collect::<BTreeSet<_>>();
        if operations.is_empty() {
            return Err(AppError::BadRequest(
                "knowledge resource binding must grant at least one product operation".into(),
            ));
        }
        Ok(Self {
            binding_id,
            owner_id,
            knowledge_base_id,
            name,
            root,
            operations,
        })
    }

    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub fn knowledge_base_id(&self) -> &KnowledgeBaseId {
        &self.knowledge_base_id
    }

    fn authorize(
        &self,
        principal_id: &str,
        action: KnowledgeAction,
    ) -> Result<(), AppError> {
        if self.owner_id != principal_id {
            return Err(AppError::Forbidden(
                "knowledge resource is owned by a different principal".into(),
            ));
        }
        let required = action.required_resource_operation();
        if !self.operations.contains(&required) {
            return Err(AppError::Forbidden(format!(
                "knowledge resource binding does not grant {} for {}",
                required.as_str(),
                action.action_id()
            )));
        }
        Ok(())
    }

    fn bound_base(&self) -> Result<BoundKnowledgeBase, AppError> {
        BoundKnowledgeBase::new(
            self.knowledge_base_id.clone(),
            self.name.clone(),
            self.root.clone(),
        )
    }
}

/// Immutable resource scope captured from one resolved Agent Snapshot.
#[derive(Clone, Debug)]
pub struct AgentKnowledgeAuthority {
    principal_id: String,
    resources: BTreeMap<KnowledgeBaseId, AgentKnowledgeResource>,
}

impl AgentKnowledgeAuthority {
    pub fn new(
        principal_id: impl Into<String>,
        resources: impl IntoIterator<Item = AgentKnowledgeResource>,
    ) -> Result<Self, AppError> {
        let principal_id = principal_id.into();
        if principal_id.trim().is_empty() {
            return Err(AppError::BadRequest(
                "knowledge authority principal must not be blank".into(),
            ));
        }
        let mut by_id = BTreeMap::new();
        let mut binding_ids = BTreeSet::new();
        for resource in resources {
            if resource.owner_id != principal_id {
                return Err(AppError::Forbidden(
                    "knowledge resource owner does not match the authority principal".into(),
                ));
            }
            if !binding_ids.insert(resource.binding_id.clone()) {
                return Err(AppError::BadRequest(
                    "knowledge authority contains a duplicate binding identity".into(),
                ));
            }
            let resource_id = resource.knowledge_base_id.clone();
            if by_id.insert(resource_id, resource).is_some() {
                // Never union duplicate bindings: doing so could combine two
                // individually narrow operation sets into wider authority.
                return Err(AppError::BadRequest(
                    "knowledge authority contains duplicate resource identities".into(),
                ));
            }
        }
        Ok(Self {
            principal_id,
            resources: by_id,
        })
    }

    pub fn principal_id(&self) -> &str {
        &self.principal_id
    }

    pub fn resource_ids_for(&self, action: KnowledgeAction) -> Vec<KnowledgeBaseId> {
        self.resources
            .values()
            .filter(|resource| resource.authorize(&self.principal_id, action).is_ok())
            .map(|resource| resource.knowledge_base_id.clone())
            .collect()
    }

    fn require(
        &self,
        action: KnowledgeAction,
        knowledge_base_id: &KnowledgeBaseId,
    ) -> Result<&AgentKnowledgeResource, AppError> {
        let resource = self.resources.get(knowledge_base_id).ok_or_else(|| {
            AppError::Forbidden(format!(
                "{} is not bound to this AgentSession",
                action.action_id()
            ))
        })?;
        resource.authorize(&self.principal_id, action)?;
        Ok(resource)
    }
}

#[derive(Clone, Debug)]
pub struct AgentKnowledgeWriteRequest {
    pub target: WriteTargetSpec,
    pub content: String,
}

/// Canonical Knowledge owner used by target Module actions.
///
/// It accepts product intent and exact resource authority only. Retrieval
/// provider/model configuration stays behind [`KnowledgeService`].
#[derive(Clone)]
pub struct AuthorizedKnowledgeService {
    service: Arc<KnowledgeService>,
    reads: BoundKnowledgeReadService,
    authority: AgentKnowledgeAuthority,
}

impl AuthorizedKnowledgeService {
    pub fn new(service: Arc<KnowledgeService>, authority: AgentKnowledgeAuthority) -> Self {
        Self {
            service,
            reads: BoundKnowledgeReadService::default(),
            authority,
        }
    }

    pub fn authority(&self) -> &AgentKnowledgeAuthority {
        &self.authority
    }

    async fn verify_registered_resource(
        &self,
        resource: &AgentKnowledgeResource,
    ) -> Result<(), AppError> {
        let (_name, registered_root) = self
            .service
            .agent_resource_identity(&resource.knowledge_base_id)
            .await?;
        if registered_root != resource.root {
            return Err(AppError::Conflict(
                "knowledge resource changed after this AgentSession was resolved".into(),
            ));
        }
        Ok(())
    }

    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<BoundKnowledgeSearchHit>, AppError> {
        if query.trim().is_empty() || query.chars().count() > 16_384 {
            return Err(AppError::BadRequest(
                "knowledge/search query must contain 1 to 16384 characters".into(),
            ));
        }
        if !(1..=20).contains(&limit) {
            return Err(AppError::BadRequest(
                "knowledge/search limit must be between 1 and 20".into(),
            ));
        }
        let resources = self
            .authority
            .resources
            .values()
            .filter(|resource| {
                resource
                    .authorize(&self.authority.principal_id, KnowledgeAction::Search)
                    .is_ok()
            })
            .collect::<Vec<_>>();
        if resources.is_empty() {
            return Err(AppError::Forbidden(
                "knowledge/search has no authorized knowledge_base resource".into(),
            ));
        }

        // Search the frozen roots, not a mutable registry lookup. Retrieval
        // providers remain KnowledgeService implementation details for its
        // configured product surfaces; they are not authoring actions and
        // cannot retarget this exact Agent resource scope.
        let mut hits = Vec::new();
        for resource in resources {
            hits.extend(
                self.reads
                    .search(&resource.bound_base()?, query, limit)
                    .await?,
            );
        }
        hits.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then(left.resource_id.as_str().cmp(right.resource_id.as_str()))
                .then(left.relative_path.cmp(&right.relative_path))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    pub async fn read(&self, handle: &str) -> Result<BoundKnowledgeDocument, AppError> {
        let (knowledge_base_id, _) = decode_doc_handle(handle)
            .ok_or_else(|| AppError::BadRequest("invalid knowledge document handle".into()))?;
        let resource = self
            .authority
            .require(KnowledgeAction::Read, &knowledge_base_id)?;
        self.verify_registered_resource(resource).await?;
        self.reads.read(&resource.bound_base()?, handle).await
    }

    pub async fn write(
        &self,
        request: AgentKnowledgeWriteRequest,
    ) -> Result<AgentKnowledgeWriteResult, AppError> {
        let knowledge_base_id = match &request.target {
            WriteTargetSpec::Handle(handle) => decode_doc_handle(handle)
                .map(|(knowledge_base_id, _)| knowledge_base_id)
                .ok_or_else(|| AppError::BadRequest("invalid knowledge document handle".into()))?,
            WriteTargetSpec::Path {
                kb_id,
                rel_path: _,
            } => kb_id.clone(),
        };
        let resource = self
            .authority
            .require(KnowledgeAction::Write, &knowledge_base_id)?;
        self.verify_registered_resource(resource).await?;
        self.service
            .write_document(WriteRequest {
                spec: request.target,
                content: request.content,
                policy: WritePolicy {
                    mode: WriteMode::Direct,
                    allow_create: true,
                    surface: WriteSurface::RegularChat,
                },
                // The request cannot choose or widen this list. It is derived
                // from the single resource authorized above.
                bound_kb_ids: vec![knowledge_base_id],
            })
            .await
            .map(AgentKnowledgeWriteResult::from)
    }

    pub async fn autogen(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        overwrite_readme: bool,
    ) -> Result<AgentKnowledgeAutogenResult, AppError> {
        let resource = self
            .authority
            .require(KnowledgeAction::Autogen, knowledge_base_id)?;
        self.verify_registered_resource(resource).await?;
        // Provider/model selection is an owner concern. Agent input cannot
        // name or override either one.
        self.service
            .generate_overview(knowledge_base_id.as_str(), overwrite_readme, None)
            .await
            .map(|outcome| AgentKnowledgeAutogenResult {
                resource_id: knowledge_base_id.clone(),
                description_updated: outcome.description_updated,
                readme_written: outcome.readme_written,
            })
    }
}

/// Stable, provider-neutral action result for a successful Knowledge write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentKnowledgeWriteResult {
    pub resource_id: KnowledgeBaseId,
    pub relative_path: String,
    pub created: bool,
}

/// Provider- and storage-neutral result of `knowledge/autogen`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentKnowledgeAutogenResult {
    pub resource_id: KnowledgeBaseId,
    pub description_updated: bool,
    pub readme_written: bool,
}

impl From<WriteOutcome> for AgentKnowledgeWriteResult {
    fn from(outcome: WriteOutcome) -> Self {
        Self {
            resource_id: outcome.kb_id,
            relative_path: outcome.final_rel_path,
            created: outcome.op == WriteOp::Create,
        }
    }
}

/// Provider-neutral document handle helper for host adapters.
pub fn knowledge_document_handle(
    knowledge_base_id: &KnowledgeBaseId,
    relative_path: &str,
) -> String {
    encode_doc_handle(knowledge_base_id, relative_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(
        owner: &str,
        operations: impl IntoIterator<Item = KnowledgeResourceOperation>,
    ) -> (tempfile::TempDir, AgentKnowledgeResource) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        std::fs::create_dir(&root).unwrap();
        let resource = AgentKnowledgeResource::new(
            "binding-1",
            owner,
            KnowledgeBaseId::new(),
            "Private base",
            root,
            operations,
        )
        .unwrap();
        (directory, resource)
    }

    #[test]
    fn authoring_surface_contains_only_product_actions() {
        assert_eq!(
            KNOWLEDGE_ACTION_IDS,
            [
                "knowledge/search",
                "knowledge/read",
                "knowledge/write",
                "knowledge/autogen"
            ]
        );
    }

    #[test]
    fn exact_resource_operations_preserve_sensitive_read_and_write_boundaries() {
        let (_directory, resource) = resource(
            "owner-a",
            [KnowledgeResourceOperation::Search, KnowledgeResourceOperation::Read],
        );
        let resource_id = resource.knowledge_base_id.clone();
        let authority = AgentKnowledgeAuthority::new("owner-a", [resource]).unwrap();
        assert!(authority.require(KnowledgeAction::Search, &resource_id).is_ok());
        assert!(authority.require(KnowledgeAction::Read, &resource_id).is_ok());
        assert!(authority.require(KnowledgeAction::Write, &resource_id).is_err());
        assert!(authority.require(KnowledgeAction::Autogen, &resource_id).is_err());
    }

    #[test]
    fn resource_binding_rejects_non_product_operations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        std::fs::create_dir(&root).unwrap();
        assert!(
            AgentKnowledgeResource::from_operation_names(
                "binding-1",
                "owner-a",
                KnowledgeBaseId::new(),
                "Private base",
                root,
                ["read", "provider-operation"],
            )
            .is_err()
        );
    }

    #[test]
    fn authority_rejects_owner_mismatch_and_duplicate_resource_widening() {
        let (_directory, wrong_owner) = resource("owner-b", [KnowledgeResourceOperation::Read]);
        assert!(AgentKnowledgeAuthority::new("owner-a", [wrong_owner]).is_err());

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        std::fs::create_dir(&root).unwrap();
        let id = KnowledgeBaseId::new();
        let read = AgentKnowledgeResource::new(
            "read-binding",
            "owner-a",
            id.clone(),
            "Private base",
            root.clone(),
            [KnowledgeResourceOperation::Read],
        )
        .unwrap();
        let write = AgentKnowledgeResource::new(
            "write-binding",
            "owner-a",
            id,
            "Private base",
            root,
            [KnowledgeResourceOperation::Write],
        )
        .unwrap();
        assert!(AgentKnowledgeAuthority::new("owner-a", [read, write]).is_err());
    }

    #[tokio::test]
    async fn product_owner_searches_reads_and_writes_only_the_bound_resource() {
        let directory = tempfile::tempdir().unwrap();
        let (service, knowledge_base_id, root) = crate::testutil::make_service_with_base(
            directory.path(),
            "Private base",
            &[("guide.md", "# Guide\nprivate deployment fact")],
        )
        .await;
        let resource = AgentKnowledgeResource::new(
            "binding-1",
            "owner-a",
            knowledge_base_id.clone(),
            "Private base",
            root,
            [
                KnowledgeResourceOperation::Search,
                KnowledgeResourceOperation::Read,
                KnowledgeResourceOperation::Write,
            ],
        )
        .unwrap();
        let owner = AuthorizedKnowledgeService::new(
            Arc::new(service),
            AgentKnowledgeAuthority::new("owner-a", [resource]).unwrap(),
        );

        let hits = owner.search("deployment", 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        let document = owner.read(&hits[0].handle).await.unwrap();
        assert!(document.content.contains("private deployment fact"));

        let written = owner
            .write(AgentKnowledgeWriteRequest {
                target: WriteTargetSpec::Path {
                    kb_id: knowledge_base_id.clone(),
                    rel_path: "new-fact.md".into(),
                },
                content: "# New fact\nAuthorized write".into(),
            })
            .await
            .unwrap();
        assert_eq!(written.resource_id, knowledge_base_id);
        assert!(written.created);
        let handle = encode_doc_handle(&written.resource_id, &written.relative_path);
        assert!(owner.read(&handle).await.unwrap().content.contains("Authorized write"));
    }

    #[tokio::test]
    async fn product_owner_checks_write_authority_before_touching_the_provider() {
        let directory = tempfile::tempdir().unwrap();
        let (service, knowledge_base_id, root) = crate::testutil::make_service_with_base(
            directory.path(),
            "Read only",
            &[("guide.md", "# Guide")],
        )
        .await;
        let resource = AgentKnowledgeResource::new(
            "binding-1",
            "owner-a",
            knowledge_base_id.clone(),
            "Read only",
            root,
            [KnowledgeResourceOperation::Read],
        )
        .unwrap();
        let owner = AuthorizedKnowledgeService::new(
            Arc::new(service),
            AgentKnowledgeAuthority::new("owner-a", [resource]).unwrap(),
        );
        assert!(
            matches!(
                owner
                    .write(AgentKnowledgeWriteRequest {
                        target: WriteTargetSpec::Path {
                            kb_id: knowledge_base_id.clone(),
                            rel_path: "denied.md".into(),
                        },
                        content: "must not land".into(),
                    })
                    .await,
                Err(AppError::Forbidden(_))
            ),
            "a read binding must never imply persistent write"
        );
        assert!(
            matches!(
                owner.autogen(&knowledge_base_id, true).await,
                Err(AppError::Forbidden(_))
            ),
            "autogen is a persistent write and requires write authority"
        );
    }
}
