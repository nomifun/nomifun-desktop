use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;
use nomifun_agent_contracts::{
    AgentSessionId, DigestHex, ResolvedSnapshotRef, RuntimeBindingId,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::error::CodingEngineError;
use crate::events::{CodingEventSink, NoopCodingEventSink, SharedCodingEventSink};
use crate::model::CodingModelPort;
use crate::tool::CodingToolInvoker;
use crate::turn::{CodingTurnRequest, CodingTurnResult};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EngineFamilyId(pub String);

impl From<&str> for EngineFamilyId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for EngineFamilyId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for EngineFamilyId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EngineBuildId(pub String);

impl From<&str> for EngineBuildId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for EngineBuildId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for EngineBuildId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingEngineChannel {
    Stable,
    Canary,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CodingRuntimeProfile {
    Coding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingEngineBuild {
    pub family_id: EngineFamilyId,
    pub build_id: EngineBuildId,
    pub build_digest: DigestHex,
    pub display_name: String,
    pub supported_profiles: Vec<CodingRuntimeProfile>,
}

impl CodingEngineBuild {
    pub fn validate(&self) -> Result<(), CodingEngineError> {
        if !is_trimmed_non_empty(self.family_id.as_ref())
            || !is_trimmed_non_empty(self.build_id.as_ref())
            || !is_trimmed_non_empty(self.build_digest.as_ref())
            || !is_trimmed_non_empty(&self.display_name)
        {
            return Err(CodingEngineError::InvalidContract(
                "engine family, build, digest and display name are required".to_owned(),
            ));
        }
        if self.build_digest.as_ref().len() != 64
            || !self
                .build_digest
                .as_ref()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CodingEngineError::InvalidContract(
                "engine build digest must be 64 hexadecimal characters".to_owned(),
            ));
        }
        let mut profiles = std::collections::BTreeSet::new();
        if self
            .supported_profiles
            .iter()
            .any(|profile| !profiles.insert(*profile))
        {
            return Err(CodingEngineError::InvalidContract(
                "engine build contains duplicate runtime profiles".to_owned(),
            ));
        }
        if !self
            .supported_profiles
            .contains(&CodingRuntimeProfile::Coding)
        {
            return Err(CodingEngineError::UnsupportedProfile(
                "coding profile is not advertised by the engine build".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "selection", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodingEngineSelector {
    Exact {
        family_id: EngineFamilyId,
        build_id: EngineBuildId,
        build_digest: DigestHex,
    },
    Channel {
        family_id: EngineFamilyId,
        channel: CodingEngineChannel,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineBinding {
    pub agent_session_id: AgentSessionId,
    pub runtime_binding_id: RuntimeBindingId,
    pub family_id: EngineFamilyId,
    pub build_id: EngineBuildId,
    pub build_digest: DigestHex,
    pub profile: CodingRuntimeProfile,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
}

impl EngineBinding {
    pub fn validate(&self) -> Result<(), CodingEngineError> {
        if !is_trimmed_non_empty(self.agent_session_id.as_ref())
            || !is_trimmed_non_empty(self.runtime_binding_id.as_ref())
            || !is_trimmed_non_empty(self.family_id.as_ref())
            || !is_trimmed_non_empty(self.build_id.as_ref())
            || !is_valid_digest(&self.build_digest)
            || !is_trimmed_non_empty(self.resolved_snapshot_ref.snapshot_id.as_ref())
            || !is_valid_digest(&self.resolved_snapshot_ref.snapshot_digest)
        {
            return Err(CodingEngineError::InvalidContract(
                "engine binding contains an empty or malformed identity".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct CodingEngineCatalog {
    builds: BTreeMap<(EngineFamilyId, EngineBuildId), CodingEngineBuild>,
    channel_defaults: BTreeMap<(EngineFamilyId, CodingEngineChannel), EngineBuildId>,
}

impl CodingEngineCatalog {
    pub fn register(&mut self, build: CodingEngineBuild) -> Result<(), CodingEngineError> {
        build.validate()?;
        let key = (build.family_id.clone(), build.build_id.clone());
        match self.builds.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(build);
                Ok(())
            }
            Entry::Occupied(_) => Err(CodingEngineError::InvalidContract(
                "engine build id is already registered for this family".to_owned(),
            )),
        }
    }

    pub fn set_channel_default(
        &mut self,
        family_id: EngineFamilyId,
        channel: CodingEngineChannel,
        build_id: EngineBuildId,
    ) -> Result<(), CodingEngineError> {
        let Some(build) = self.builds.get(&(family_id.clone(), build_id.clone())) else {
            return Err(CodingEngineError::EngineBuildNotFound(build_id.0));
        };
        debug_assert_eq!(build.family_id, family_id);
        self.channel_defaults.insert((family_id, channel), build_id);
        Ok(())
    }

    pub fn resolve(
        &self,
        selector: &CodingEngineSelector,
    ) -> Result<CodingEngineBuild, CodingEngineError> {
        let build = match selector {
            CodingEngineSelector::Exact {
                family_id,
                build_id,
                build_digest,
            } => {
                let build = self
                    .builds
                    .get(&(family_id.clone(), build_id.clone()))
                    .ok_or_else(|| CodingEngineError::EngineBuildNotFound(build_id.0.clone()))?;
                if &build.build_digest != build_digest {
                    return Err(CodingEngineError::EngineBuildDigestMismatch {
                        engine_build_id: build_id.0.clone(),
                        expected: build_digest.0.clone(),
                        actual: build.build_digest.0.clone(),
                    });
                }
                build
            }
            CodingEngineSelector::Channel { family_id, channel } => {
                let build_id = self
                    .channel_defaults
                    .get(&(family_id.clone(), *channel))
                    .ok_or_else(|| {
                        CodingEngineError::EngineBuildNotFound(format!(
                            "{}:{channel:?}",
                            family_id.as_ref()
                        ))
                    })?;
                self.builds
                    .get(&(family_id.clone(), build_id.clone()))
                    .ok_or_else(|| CodingEngineError::EngineBuildNotFound(build_id.0.clone()))?
            }
        };
        Ok(build.clone())
    }

    pub fn builds(&self) -> impl Iterator<Item = &CodingEngineBuild> {
        self.builds.values()
    }

    pub fn instantiate(
        &self,
        selector: &CodingEngineSelector,
    ) -> Result<CodingEngine, CodingEngineError> {
        CodingEngine::new(self.resolve(selector)?)
    }
}

pub trait AgentEngine: Send + Sync {
    fn build(&self) -> &CodingEngineBuild;
}

pub struct CodingEngine {
    build: CodingEngineBuild,
}

impl CodingEngine {
    pub fn new(build: CodingEngineBuild) -> Result<Self, CodingEngineError> {
        build.validate()?;
        Ok(Self { build })
    }

    pub fn bind(
        &self,
        agent_session_id: AgentSessionId,
        runtime_binding_id: RuntimeBindingId,
        profile: CodingRuntimeProfile,
        resolved_snapshot_ref: ResolvedSnapshotRef,
    ) -> Result<EngineBinding, CodingEngineError> {
        if !self.build.supported_profiles.contains(&profile) {
            return Err(CodingEngineError::UnsupportedProfile(format!(
                "{profile:?}"
            )));
        }
        let binding = EngineBinding {
            agent_session_id,
            runtime_binding_id,
            family_id: self.build.family_id.clone(),
            build_id: self.build.build_id.clone(),
            build_digest: self.build.build_digest.clone(),
            profile,
            resolved_snapshot_ref,
        };
        binding.validate()?;
        Ok(binding)
    }

    pub fn open_session(
        &self,
        binding: EngineBinding,
        model: Arc<dyn CodingModelPort>,
        tools: Arc<dyn CodingToolInvoker>,
        event_sink: Option<Arc<dyn CodingEventSink>>,
    ) -> Result<CodingEngineSession, CodingEngineError> {
        binding.validate()?;
        if binding.family_id != self.build.family_id
            || binding.build_id != self.build.build_id
            || binding.build_digest != self.build.build_digest
        {
            return Err(CodingEngineError::InvalidContract(
                "runtime binding does not match the selected engine build".to_owned(),
            ));
        }
        if !self.build.supported_profiles.contains(&binding.profile) {
            return Err(CodingEngineError::UnsupportedProfile(format!(
                "{:?}",
                binding.profile
            )));
        }
        Ok(CodingEngineSession {
            binding,
            model,
            tools,
            event_sink: event_sink.unwrap_or_else(|| Arc::new(NoopCodingEventSink)),
            state: Mutex::new(CodingEngineSessionState::default()),
        })
    }

    pub fn build(&self) -> &CodingEngineBuild {
        &self.build
    }
}

impl AgentEngine for CodingEngine {
    fn build(&self) -> &CodingEngineBuild {
        &self.build
    }
}

pub struct CodingEngineSession {
    binding: EngineBinding,
    model: Arc<dyn CodingModelPort>,
    tools: Arc<dyn CodingToolInvoker>,
    event_sink: SharedCodingEventSink,
    state: Mutex<CodingEngineSessionState>,
}

#[derive(Default)]
struct CodingEngineSessionState {
    disposed: bool,
    active_turn: Option<CancellationToken>,
}

impl CodingEngineSession {
    pub fn binding(&self) -> &EngineBinding {
        &self.binding
    }

    pub async fn run_turn(
        &self,
        request: CodingTurnRequest,
    ) -> Result<CodingTurnResult, CodingEngineError> {
        let cancellation = {
            let mut state = self.state.lock().await;
            if state.disposed {
                return Err(CodingEngineError::SessionDisposed);
            }
            if state.active_turn.is_some() {
                return Err(CodingEngineError::TurnAlreadyRunning);
            }
            let cancellation = CancellationToken::new();
            state.active_turn = Some(cancellation.clone());
            cancellation
        };
        let run_cancellation = cancellation.clone();
        let result = AssertUnwindSafe(crate::turn::run_turn(
            self.binding.clone(),
            Arc::clone(&self.model),
            Arc::clone(&self.tools),
            Arc::clone(&self.event_sink),
            request,
            run_cancellation,
        ))
        .catch_unwind()
        .await
        .unwrap_or(Err(CodingEngineError::TurnPanicked));

        cancellation.cancel();
        self.state.lock().await.active_turn = None;
        result
    }

    pub async fn cancel(&self) -> bool {
        let cancellation = self.state.lock().await.active_turn.clone();
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
            true
        } else {
            false
        }
    }

    pub async fn dispose(&self) {
        let cancellation = {
            let mut state = self.state.lock().await;
            state.disposed = true;
            state.active_turn.clone()
        };
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
        }
    }

    pub async fn is_turn_active(&self) -> bool {
        self.state.lock().await.active_turn.is_some()
    }

    pub async fn is_disposed(&self) -> bool {
        self.state.lock().await.disposed
    }
}

fn is_trimmed_non_empty(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

fn is_valid_digest(value: &DigestHex) -> bool {
    value.as_ref().len() == 64 && value.as_ref().bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(
        build_id: &str,
        digest_byte: char,
    ) -> CodingEngineBuild {
        CodingEngineBuild {
            family_id: EngineFamilyId::from("nomifun.coding"),
            build_id: EngineBuildId::from(build_id),
            build_digest: DigestHex::from(digest_byte.to_string().repeat(64)),
            display_name: format!("Coding {build_id}"),
            supported_profiles: vec![CodingRuntimeProfile::Coding],
        }
    }

    #[test]
    fn catalog_keeps_stable_and_canary_builds_resolvable() {
        let mut catalog = CodingEngineCatalog::default();
        catalog
            .register(build("stable-1", 'a'))
            .unwrap();
        catalog
            .register(build("canary-1", 'b'))
            .unwrap();
        catalog
            .set_channel_default(
                EngineFamilyId::from("nomifun.coding"),
                CodingEngineChannel::Stable,
                EngineBuildId::from("stable-1"),
            )
            .unwrap();
        catalog
            .set_channel_default(
                EngineFamilyId::from("nomifun.coding"),
                CodingEngineChannel::Canary,
                EngineBuildId::from("canary-1"),
            )
            .unwrap();

        assert_eq!(
            catalog
                .resolve(&CodingEngineSelector::Channel {
                    family_id: EngineFamilyId::from("nomifun.coding"),
                    channel: CodingEngineChannel::Stable,
                })
                .unwrap()
                .build_id,
            EngineBuildId::from("stable-1")
        );
        assert_eq!(
            catalog
                .resolve(&CodingEngineSelector::Channel {
                    family_id: EngineFamilyId::from("nomifun.coding"),
                    channel: CodingEngineChannel::Canary,
                })
                .unwrap()
                .build_id,
            EngineBuildId::from("canary-1")
        );
        assert_eq!(catalog.builds().count(), 2);
    }

    #[test]
    fn the_same_immutable_build_can_be_promoted_between_channels() {
        let mut catalog = CodingEngineCatalog::default();
        catalog.register(build("release-1", 'a')).unwrap();
        catalog
            .set_channel_default(
                EngineFamilyId::from("nomifun.coding"),
                CodingEngineChannel::Canary,
                EngineBuildId::from("release-1"),
            )
            .unwrap();
        catalog
            .set_channel_default(
                EngineFamilyId::from("nomifun.coding"),
                CodingEngineChannel::Stable,
                EngineBuildId::from("release-1"),
            )
            .unwrap();

        let stable = catalog
            .resolve(&CodingEngineSelector::Channel {
                family_id: EngineFamilyId::from("nomifun.coding"),
                channel: CodingEngineChannel::Stable,
            })
            .unwrap();
        let canary = catalog
            .resolve(&CodingEngineSelector::Channel {
                family_id: EngineFamilyId::from("nomifun.coding"),
                channel: CodingEngineChannel::Canary,
            })
            .unwrap();
        assert_eq!(stable, canary);
    }

    #[test]
    fn duplicate_registration_and_exact_digest_mismatch_fail_closed() {
        let mut catalog = CodingEngineCatalog::default();
        catalog.register(build("coding-1", 'a')).unwrap();
        assert!(matches!(
            catalog.register(build("coding-1", 'b')),
            Err(CodingEngineError::InvalidContract(_))
        ));
        assert!(matches!(
            catalog.resolve(&CodingEngineSelector::Exact {
                family_id: EngineFamilyId::from("nomifun.coding"),
                build_id: EngineBuildId::from("coding-1"),
                build_digest: DigestHex::from("b".repeat(64)),
            }),
            Err(CodingEngineError::EngineBuildDigestMismatch { .. })
        ));
    }

    #[test]
    fn different_sessions_can_keep_different_exact_build_bindings() {
        let mut catalog = CodingEngineCatalog::default();
        catalog.register(build("stable-1", 'a')).unwrap();
        catalog.register(build("canary-1", 'b')).unwrap();
        catalog
            .set_channel_default(
                EngineFamilyId::from("nomifun.coding"),
                CodingEngineChannel::Stable,
                EngineBuildId::from("stable-1"),
            )
            .unwrap();
        catalog
            .set_channel_default(
                EngineFamilyId::from("nomifun.coding"),
                CodingEngineChannel::Canary,
                EngineBuildId::from("canary-1"),
            )
            .unwrap();

        let snapshot = ResolvedSnapshotRef {
            snapshot_id: nomifun_agent_contracts::ResolvedSnapshotId::from("snapshot"),
            snapshot_digest: DigestHex::from("c".repeat(64)),
        };
        let stable = catalog
            .instantiate(&CodingEngineSelector::Channel {
                family_id: EngineFamilyId::from("nomifun.coding"),
                channel: CodingEngineChannel::Stable,
            })
            .unwrap()
            .bind(
                AgentSessionId::from("stable-session"),
                RuntimeBindingId::from("stable-binding"),
                CodingRuntimeProfile::Coding,
                snapshot.clone(),
            )
            .unwrap();
        let canary = catalog
            .instantiate(&CodingEngineSelector::Channel {
                family_id: EngineFamilyId::from("nomifun.coding"),
                channel: CodingEngineChannel::Canary,
            })
            .unwrap()
            .bind(
                AgentSessionId::from("canary-session"),
                RuntimeBindingId::from("canary-binding"),
                CodingRuntimeProfile::Coding,
                snapshot,
            )
            .unwrap();

        assert_eq!(stable.build_id, EngineBuildId::from("stable-1"));
        assert_eq!(canary.build_id, EngineBuildId::from("canary-1"));
        assert_ne!(stable.build_digest, canary.build_digest);
    }
}
