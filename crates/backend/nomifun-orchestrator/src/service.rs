//! Fleet (编队) CRUD service. No axum imports — pure business logic over the
//! [`IFleetRepository`]. Handles Row↔DTO mapping and JSON (de)serialization of
//! the per-member `capability_profile` / `constraints` fields, decoding
//! fail-soft (a malformed JSON column logs a warning and surfaces as `None`
//! rather than failing the whole request — mirrors the team's `decode_tags`).

use std::sync::Arc;

use nomifun_api_types::{
    CapabilityProfile, CreateFleetRequest, Fleet, FleetMember, FleetMemberInput, MemberConstraints,
    UpdateFleetRequest,
};
use nomifun_common::AppError;
use nomifun_db::models::{FleetMemberRow, FleetRow};
use nomifun_db::{CreateFleetParams, IFleetRepository, NewFleetMember, UpdateFleetParams};

use crate::error::OrchestratorError;

/// Decode an optional JSON column into a typed struct, fail-soft: a malformed
/// JSON string logs a warning and yields `None` rather than failing the request
/// (mirrors the team's `decode_tags` philosophy). `field`/`member_id` are only
/// used for the warning context.
///
/// `serde_json::from_str`'s `DeserializeOwned` bound is expressed via the value
/// type at each call site, so this crate does not need a direct `serde`
/// dependency.
fn decode_capability_profile(raw: Option<&str>, member_id: &str) -> Option<CapabilityProfile> {
    let raw = raw?;
    match serde_json::from_str::<CapabilityProfile>(raw) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(
                member_id,
                field = "capability_profile",
                error = %e,
                "failed to decode fleet member JSON field; treating as absent"
            );
            None
        }
    }
}

fn decode_constraints(raw: Option<&str>, member_id: &str) -> Option<MemberConstraints> {
    let raw = raw?;
    match serde_json::from_str::<MemberConstraints>(raw) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(
                member_id,
                field = "constraints",
                error = %e,
                "failed to decode fleet member JSON field; treating as absent"
            );
            None
        }
    }
}

/// Map a fleet member DB row to its DTO, decoding the JSON columns fail-soft.
fn member_row_to_dto(row: FleetMemberRow) -> FleetMember {
    let capability_profile = decode_capability_profile(row.capability_profile.as_deref(), &row.id);
    let constraints = decode_constraints(row.constraints.as_deref(), &row.id);
    FleetMember {
        id: row.id,
        agent_id: row.agent_id,
        provider_id: row.provider_id,
        model: row.model,
        role_hint: row.role_hint,
        capability_profile,
        constraints,
        sort_order: row.sort_order,
    }
}

/// Assemble a [`Fleet`] DTO from its row + member rows.
fn fleet_row_to_dto(row: FleetRow, members: Vec<FleetMemberRow>) -> Fleet {
    Fleet {
        id: row.id,
        name: row.name,
        description: row.description,
        max_parallel: row.max_parallel,
        members: members.into_iter().map(member_row_to_dto).collect(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Map a member input (DTO) to a repository insert struct, JSON-encoding the
/// structured fields. `index` supplies the default `sort_order` when the input
/// leaves it unset.
fn member_input_to_new(input: FleetMemberInput, index: usize) -> NewFleetMember {
    NewFleetMember {
        agent_id: input.agent_id,
        provider_id: input.provider_id,
        model: input.model,
        role_hint: input.role_hint,
        // Encoding a well-formed struct cannot fail; if it ever did we drop the
        // field rather than reject the write.
        capability_profile: input
            .capability_profile
            .and_then(|p| serde_json::to_string(&p).ok()),
        constraints: input.constraints.and_then(|c| serde_json::to_string(&c).ok()),
        sort_order: input.sort_order.unwrap_or(index as i64),
    }
}

#[derive(Clone)]
pub struct FleetService {
    fleet_repo: Arc<dyn IFleetRepository>,
}

impl FleetService {
    pub fn new(fleet_repo: Arc<dyn IFleetRepository>) -> Self {
        Self { fleet_repo }
    }

    pub async fn list(&self, user_id: &str) -> Result<Vec<Fleet>, AppError> {
        let rows = self
            .fleet_repo
            .list_fleets(user_id)
            .await
            .map_err(OrchestratorError::from)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let members = self
                .fleet_repo
                .list_members(&row.id)
                .await
                .map_err(OrchestratorError::from)?;
            out.push(fleet_row_to_dto(row, members));
        }
        Ok(out)
    }

    pub async fn get(&self, id: &str) -> Result<Fleet, AppError> {
        let row = self
            .fleet_repo
            .get_fleet(id)
            .await
            .map_err(OrchestratorError::from)?
            .ok_or_else(|| OrchestratorError::NotFound(format!("fleet {id}")))?;
        let members = self
            .fleet_repo
            .list_members(&row.id)
            .await
            .map_err(OrchestratorError::from)?;
        Ok(fleet_row_to_dto(row, members))
    }

    pub async fn create(&self, user_id: &str, req: CreateFleetRequest) -> Result<Fleet, AppError> {
        if req.name.trim().is_empty() {
            return Err(OrchestratorError::BadRequest("name must not be empty".into()).into());
        }
        if req.members.is_empty() {
            return Err(
                OrchestratorError::BadRequest("a fleet must have at least one member".into()).into(),
            );
        }
        let row = self
            .fleet_repo
            .create_fleet(CreateFleetParams {
                user_id: user_id.to_string(),
                name: req.name,
                description: req.description,
                max_parallel: req.max_parallel,
            })
            .await
            .map_err(OrchestratorError::from)?;
        let new_members: Vec<NewFleetMember> = req
            .members
            .into_iter()
            .enumerate()
            .map(|(i, m)| member_input_to_new(m, i))
            .collect();
        self.fleet_repo
            .replace_members(&row.id, new_members)
            .await
            .map_err(OrchestratorError::from)?;
        // Re-read members so the returned DTO reflects the minted ids + ordering.
        self.get(&row.id).await
    }

    pub async fn update(&self, id: &str, req: UpdateFleetRequest) -> Result<Fleet, AppError> {
        // Confirm the fleet exists first so an unknown id is a clean 404.
        if self
            .fleet_repo
            .get_fleet(id)
            .await
            .map_err(OrchestratorError::from)?
            .is_none()
        {
            return Err(OrchestratorError::NotFound(format!("fleet {id}")).into());
        }
        if let Some(name) = &req.name
            && name.trim().is_empty()
        {
            return Err(OrchestratorError::BadRequest("name must not be empty".into()).into());
        }
        self.fleet_repo
            .update_fleet(
                id,
                UpdateFleetParams {
                    name: req.name,
                    description: req.description,
                    max_parallel: req.max_parallel,
                },
            )
            .await
            .map_err(OrchestratorError::from)?;
        if let Some(members) = req.members {
            let new_members: Vec<NewFleetMember> = members
                .into_iter()
                .enumerate()
                .map(|(i, m)| member_input_to_new(m, i))
                .collect();
            self.fleet_repo
                .replace_members(id, new_members)
                .await
                .map_err(OrchestratorError::from)?;
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        self.fleet_repo
            .delete_fleet(id)
            .await
            .map_err(OrchestratorError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::init_database_memory;
    use nomifun_db::SqliteFleetRepository;

    fn sample_member(agent_id: &str) -> FleetMemberInput {
        FleetMemberInput {
            agent_id: agent_id.to_string(),
            provider_id: Some("prov_x".to_string()),
            model: Some("claude-opus-4-8".to_string()),
            role_hint: Some("后端".to_string()),
            capability_profile: Some(CapabilityProfile {
                strengths: vec!["coding".to_string()],
                modalities: vec!["text".to_string()],
                tools: true,
                reasoning: "high".to_string(),
                cost_tier: "premium".to_string(),
                speed_tier: "medium".to_string(),
            }),
            constraints: Some(MemberConstraints {
                max_concurrency: Some(2),
                cost_tier: Some("premium".to_string()),
                allowed_task_kinds: Some(vec!["research".to_string()]),
            }),
            sort_order: None,
        }
    }

    async fn service() -> FleetService {
        let db = init_database_memory().await.expect("db init");
        let repo = SqliteFleetRepository::new(db.pool().clone());
        FleetService::new(Arc::new(repo))
    }

    #[tokio::test]
    async fn fleet_service_create_get_update_delete() {
        let svc = service().await;

        // create with one member + capability_profile
        let created = svc
            .create(
                "u1",
                CreateFleetRequest {
                    name: "研究编队".to_string(),
                    description: Some("multi-agent".to_string()),
                    max_parallel: Some(3),
                    members: vec![sample_member("agent_builtin_claude")],
                },
            )
            .await
            .expect("create succeeds");
        assert!(created.id.starts_with("fleet_"));
        assert_eq!(created.name, "研究编队");
        assert_eq!(created.max_parallel, Some(3));
        assert_eq!(created.members.len(), 1);
        let m = &created.members[0];
        assert_eq!(m.agent_id, "agent_builtin_claude");
        assert_eq!(m.sort_order, 0, "sort_order defaults to member index");
        let profile = m.capability_profile.as_ref().expect("profile decoded");
        assert_eq!(profile.strengths, vec!["coding"]);
        assert!(profile.tools);
        let constraints = m.constraints.as_ref().expect("constraints decoded");
        assert_eq!(constraints.max_concurrency, Some(2));

        // get returns the same fleet
        let fetched = svc.get(&created.id).await.expect("get succeeds");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.members.len(), 1);

        // get unknown id → NotFound
        let err = svc.get("fleet_nope").await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");

        // create empty name → BadRequest
        let err = svc
            .create(
                "u1",
                CreateFleetRequest {
                    name: "  ".to_string(),
                    description: None,
                    max_parallel: None,
                    members: vec![sample_member("agent_a")],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "empty name got {err:?}");

        // create no members → BadRequest
        let err = svc
            .create(
                "u1",
                CreateFleetRequest {
                    name: "空编队".to_string(),
                    description: None,
                    max_parallel: None,
                    members: vec![],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "empty members got {err:?}");

        // update: rename + replace members (two members)
        let updated = svc
            .update(
                &created.id,
                UpdateFleetRequest {
                    name: Some("改名编队".to_string()),
                    description: Some(None), // clear description
                    max_parallel: Some(Some(5)),
                    members: Some(vec![sample_member("agent_one"), sample_member("agent_two")]),
                },
            )
            .await
            .expect("update succeeds");
        assert_eq!(updated.name, "改名编队");
        assert_eq!(updated.description, None);
        assert_eq!(updated.max_parallel, Some(5));
        assert_eq!(updated.members.len(), 2);
        assert_eq!(updated.members[0].agent_id, "agent_one");
        assert_eq!(updated.members[0].sort_order, 0);
        assert_eq!(updated.members[1].agent_id, "agent_two");
        assert_eq!(updated.members[1].sort_order, 1);

        // update unknown id → NotFound
        let err = svc
            .update("fleet_nope", UpdateFleetRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "update unknown got {err:?}");

        // list returns the fleet
        let listed = svc.list("u1").await.expect("list succeeds");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        // delete → list empty
        svc.delete(&created.id).await.expect("delete succeeds");
        let listed = svc.list("u1").await.expect("list after delete");
        assert!(listed.is_empty(), "fleet list should be empty after delete");
    }
}
