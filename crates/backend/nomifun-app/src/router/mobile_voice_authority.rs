use super::{mobile_voice_registry::AppVoiceRegistry, nomi_core_session::NomiCoreSessionOwner};
use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentSessionId, AgentSessionLiveRecord, PrincipalRef, digest_payload,
};
use nomifun_voice::{SharedVoiceJournal, VoiceAuthorityPort, VoiceBindingPlan};
use nomifun_voice_contracts::{VoiceProfile, voice::*};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

pub(crate) struct AppVoiceAuthority {
    pub sessions: Arc<NomiCoreSessionOwner>,
    pub registry: Arc<AppVoiceRegistry>,
    pub journal: Arc<SharedVoiceJournal>,
    pool: sqlx::SqlitePool,
}
fn error(message: impl std::fmt::Display) -> VoiceError {
    VoiceError::new(VoiceErrorKind::StaleBinding, message.to_string())
}
/// Read the lifecycle owner's existing generation. Never create/repair a root
/// from the optional voice path, and never use the process-global environment
/// as evidence for a dataset that may have been restored since boot.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
struct VoiceStorageNamespace {
    data_root: String,
    storage_generation: String,
}
fn read_storage_namespace(data_dir: &Path) -> Result<VoiceStorageNamespace, VoiceError> {
    let root = std::fs::canonicalize(data_dir)
        .map_err(|_| error("voice storage namespace is unavailable"))?;
    let path = root.join("storage-generation");
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| error("voice storage generation is unavailable"))?;
    let linked = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        linked || metadata.file_attributes() & 0x400 != 0
    };
    if linked || !metadata.is_file() || metadata.len() > 128 {
        return Err(error(
            "voice storage generation is not a bounded regular lifecycle file",
        ));
    }
    let mut generation = String::new();
    std::fs::File::open(&path)
        .map_err(|_| error("voice storage generation could not be read"))?
        .take(129)
        .read_to_string(&mut generation)
        .map_err(|_| error("voice storage generation could not be read"))?;
    nomifun_common::validate_uuidv7(&generation).map_err(|_| {
        error("voice storage generation is not the current canonical lifecycle identity")
    })?;
    let data_root = nomifun_common::paths::simplified(&root)
        .to_str()
        .ok_or_else(|| error("voice storage namespace is not valid Unicode"))?
        .to_owned();
    Ok(VoiceStorageNamespace {
        data_root,
        storage_generation: generation,
    })
}
/// Queue recovery checks only this original dataset provenance. Accepted work
/// is not made invalid by later provider credential or VoiceProfile changes.
pub(crate) fn current_voice_namespace(data_dir: &Path) -> Result<String, VoiceError> {
    digest_payload(&read_storage_namespace(data_dir)?)
        .map(|digest| digest.0)
        .map_err(error)
}
impl AppVoiceAuthority {
    pub fn new(
        sessions: Arc<NomiCoreSessionOwner>,
        registry: Arc<AppVoiceRegistry>,
        journal: Arc<SharedVoiceJournal>,
        pool: sqlx::SqlitePool,
    ) -> Self {
        Self {
            sessions,
            registry,
            journal,
            pool,
        }
    }
    pub async fn live(
        &self,
        owner: &str,
        session: &str,
    ) -> Result<AgentSessionLiveRecord, VoiceError> {
        let record = self
            .sessions
            .canonical()
            .store()
            .get_live_session(&AgentSessionId::from(session))
            .await
            .map_err(error)?;
        if record.owner_ref
            != (PrincipalRef {
                principal_kind: "user".into(),
                principal_id: owner.into(),
            })
        {
            return Err(VoiceError::new(
                VoiceErrorKind::Authentication,
                "voice Session belongs to another owner",
            ));
        }
        Ok(record)
    }
    pub async fn saved_profile(
        &self,
        owner: &str,
        session: &str,
    ) -> Result<Option<VoiceProfile>, VoiceError> {
        self.live(owner, session).await?;
        if self.journal.opened().await.is_none()
            && !self.journal.data_dir().join("voice/voice.sqlite3").exists()
        {
            return Ok(None);
        }
        self.journal
            .get_or_open()
            .await?
            .session_profile(owner.into(), session.into())
            .await
    }
    async fn lease(
        &self,
        owner: &str,
        session: &str,
        profile: &VoiceProfile,
    ) -> Result<String, VoiceError> {
        let current = self.live(owner, session).await?;
        if current.agent_binding.binding_version != profile.binding_version
            || profile.agent_session_id.as_ref() != session
            || !profile.enabled
        {
            return Err(error("voice profile no longer matches the current binding"));
        }
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'").bind(session).fetch_one(&self.pool).await.map_err(error)?;
        let connection = self.registry.lease_revision(&profile.route).await?;
        let data_dir = self.journal.data_dir().to_owned();
        let namespace = tokio::task::spawn_blocking(move || current_voice_namespace(&data_dir))
            .await
            .map_err(|_| error("voice storage namespace read failed"))??;
        let full = digest_payload(&(
            profile.profile_id.as_str(),
            profile.revision,
            &current.agent_binding,
            floor,
            connection,
            &namespace,
        ))
        .map(|digest| digest.0)
        .map_err(error)?;
        Ok(format!("ns:{namespace}:{full}"))
    }
    pub async fn availability(
        &self,
        owner: &str,
        session: &str,
    ) -> Result<serde_json::Value, VoiceError> {
        let current = self.live(owner, session).await?;
        let profile = self.saved_profile(owner, session).await?;
        let enabled = profile.as_ref().is_some_and(|profile| {
            profile.enabled && profile.binding_version == current.agent_binding.binding_version
        });
        let descriptor = profile
            .as_ref()
            .and_then(|profile| self.registry.registry.validate_route(&profile.route).ok());
        Ok(
            serde_json::json!({"supported":true,"api_version":1,"agent_session_id":session,"binding_version":current.agent_binding.binding_version,
            "enabled":enabled,"profile_id":profile.as_ref().map(|p|p.profile_id.clone()),"profile_revision":profile.as_ref().map(|p|p.revision),
            "transport":profile.as_ref().map(|p|p.route.transport),"label":profile.as_ref().map(|p|p.label.clone()),"native_requirements":descriptor.as_ref().and_then(|d|d.native_requirements.clone()),
            "supported_transports":descriptor.map(|d|d.transports).unwrap_or_default(),"profile":profile}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_namespace_rotates_even_when_main_entity_and_connection_identities_are_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage-generation");
        let first = uuid::Uuid::now_v7().to_string();
        std::fs::write(&path, &first).unwrap();
        let namespace = read_storage_namespace(dir.path()).unwrap();
        let namespace_digest = current_voice_namespace(dir.path()).unwrap();
        assert_eq!(namespace.storage_generation, first);
        assert_eq!(namespace_digest, digest_payload(&namespace).unwrap().0);
        assert_eq!(namespace_digest.len(), 64);
        assert!(
            namespace_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("storage-generation"), &first).unwrap();
        assert_ne!(
            namespace_digest,
            current_voice_namespace(other.path()).unwrap(),
            "a copied data directory is a different voice namespace"
        );
        let next = uuid::Uuid::now_v7().to_string();
        std::fs::write(&path, &next).unwrap();
        let restored = read_storage_namespace(dir.path()).unwrap();
        assert_eq!(restored.storage_generation, next);
        assert_ne!(
            namespace_digest,
            current_voice_namespace(dir.path()).unwrap()
        );
        assert_ne!(
            digest_payload(&namespace).unwrap(),
            digest_payload(&restored).unwrap(),
            "restore generation must invalidate a prior voice lease despite identical Session/binding/floor/credentials"
        );
    }
    #[test]
    fn missing_or_invalid_storage_generation_fails_voice_without_creating_or_repairing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage-generation");
        assert!(current_voice_namespace(dir.path()).is_err());
        assert!(!path.exists());
        for invalid in [
            "",
            "uninitialized",
            "01900000-0000-7000-8000-000000000000\n",
        ] {
            std::fs::write(&path, invalid).unwrap();
            assert!(current_voice_namespace(dir.path()).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
        }
        assert!(!dir.path().join("nomifun-backend.db").exists());
    }
}
#[async_trait]
impl VoiceAuthorityPort for AppVoiceAuthority {
    async fn resolve_plan(
        &self,
        owner: &str,
        session: &str,
        binding_version: u64,
        profile_id: &str,
        profile_revision: u64,
    ) -> Result<VoiceBindingPlan, VoiceError> {
        self.validate_binding(owner, session, binding_version)
            .await?;
        let profile = self
            .journal
            .get_or_open()
            .await?
            .profile(owner.into(), profile_id.into())
            .await?
            .ok_or_else(|| error("voice profile was not found"))?;
        if profile.revision != profile_revision
            || profile.agent_session_id.as_ref() != session
            || profile.binding_version != binding_version
            || !profile.enabled
        {
            return Err(error("voice activation profile is stale or disabled"));
        }
        self.registry.registry.validate_route(&profile.route)?;
        let lease_revision = self.lease(owner, session, &profile).await?;
        let plan = ResolvedVoicePlan {
            identity: profile.route.identity().map_err(error)?,
            interaction: AgentVoiceInteraction {
                enabled: true,
                mode: VoiceInteractionMode::FullDuplex,
                route_key: profile_id.into(),
                work_policy: VoiceWorkPolicy::ExplicitIntent,
            },
        };
        Ok(VoiceBindingPlan {
            work_steering_policy:profile.work_steering_policy,
            record: profile.route,
            plan,
            profile_id: profile_id.into(),
            profile_revision,
            lease_revision,
        })
    }
    async fn validate_binding(
        &self,
        owner: &str,
        session: &str,
        binding_version: u64,
    ) -> Result<(), VoiceError> {
        if self
            .live(owner, session)
            .await?
            .agent_binding
            .binding_version
            != binding_version
        {
            return Err(error("voice Agent binding was changed"));
        }
        Ok(())
    }
    async fn validate_lease(
        &self,
        owner: &str,
        session: &str,
        binding_version: u64,
        record: &VoiceRouteRecord,
        revision: &str,
    ) -> Result<(), VoiceError> {
        self.validate_binding(owner, session, binding_version)
            .await?;
        let profile = self
            .saved_profile(owner, session)
            .await?
            .ok_or_else(|| error("voice profile was removed"))?;
        if profile.route.identity().map_err(error)? != record.identity().map_err(error)?
            || self.lease(owner, session, &profile).await? != revision
        {
            return Err(error(
                "voice authority, profile, context or credential lease was revoked",
            ));
        }
        Ok(())
    }
}
