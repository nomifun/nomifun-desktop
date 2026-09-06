//! Typed AP-5 comparison between immutable Revision contribution locks and
//! the current formal contribution catalog.
//!
//! Matching is deliberately source-exact. A contribution with the same public
//! ID from another Mount, MiniApp, MCP binding, or built-in source is reported
//! only as an ignored alternative and is never selected as a fallback.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CanonicalErrorCode, ContributionId, ContributionLock, ContributionSourceKind, DigestHex,
    McpBindingId, MiniAppId, PluginMountId, StableSourceIdentity, digest_payload,
};

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CurrentContributionLifecycle {
    Active,
    /// The owning domain applied a replacement at the same stable source.
    Replaced {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_digest: Option<DigestHex>,
    },
    Disabled {
        reason: String,
    },
    Unavailable {
        code: CanonicalErrorCode,
        reason: String,
    },
    /// The current formal entry was published by a different Active Release
    /// of the same MiniApp. Compatibility is still decided exclusively from
    /// the frozen and current contract digests.
    MiniAppActiveReleaseChanged {
        release_id: String,
        release_digest: DigestHex,
    },
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct CurrentContribution {
    pub source_kind: ContributionSourceKind,
    pub source_identity: StableSourceIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_id: Option<PluginMountId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miniapp_id: Option<MiniAppId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_binding_id: Option<McpBindingId>,
    pub contribution_id: ContributionId,
    pub contract_digest: DigestHex,
    pub lifecycle: CurrentContributionLifecycle,
}

impl CurrentContribution {
    pub fn validate(&self) -> Result<(), RevisionImpactError> {
        ContributionLock {
            source_kind: self.source_kind,
            source_identity: self.source_identity.clone(),
            mount_id: self.mount_id.clone(),
            miniapp_id: self.miniapp_id.clone(),
            mcp_binding_id: self.mcp_binding_id.clone(),
            contribution_id: self.contribution_id.clone(),
            contract_digest: self.contract_digest.clone(),
        }
        .validate()
        .map_err(|violation| RevisionImpactError::InvalidCurrentContribution {
            contribution_id: self.contribution_id.clone(),
            reason: violation.message,
        })?;

        match &self.lifecycle {
            CurrentContributionLifecycle::Active => {}
            CurrentContributionLifecycle::Replaced { target_digest } => {
                if let Some(digest) = target_digest {
                    validate_digest(digest, &self.contribution_id, "target_digest")?;
                }
            }
            CurrentContributionLifecycle::Disabled { reason } => {
                validate_non_empty(reason, &self.contribution_id, "reason")?;
            }
            CurrentContributionLifecycle::Unavailable { code, reason } => {
                validate_non_empty(code.as_ref(), &self.contribution_id, "code")?;
                validate_non_empty(reason, &self.contribution_id, "reason")?;
            }
            CurrentContributionLifecycle::MiniAppActiveReleaseChanged {
                release_id,
                release_digest,
            } => {
                if self.source_kind != ContributionSourceKind::MiniAppActiveRelease {
                    return Err(RevisionImpactError::InvalidCurrentContribution {
                        contribution_id: self.contribution_id.clone(),
                        reason:
                            "miniapp_active_release_changed requires a miniapp_active_release source"
                                .into(),
                    });
                }
                validate_non_empty(release_id, &self.contribution_id, "release_id")?;
                validate_digest(
                    release_digest,
                    &self.contribution_id,
                    "release_digest",
                )?;
            }
        }
        Ok(())
    }

    fn source_key(&self) -> ContributionSourceKey {
        ContributionSourceKey {
            source_kind: self.source_kind,
            source_identity: self.source_identity.clone(),
            mount_id: self.mount_id.clone(),
            miniapp_id: self.miniapp_id.clone(),
            mcp_binding_id: self.mcp_binding_id.clone(),
            contribution_id: self.contribution_id.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContributionLifecycleImpact {
    Active,
    Replaced {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_digest: Option<DigestHex>,
    },
    Disabled {
        reason: String,
    },
    Unavailable {
        code: CanonicalErrorCode,
        reason: String,
    },
    Uninstalled,
    MiniAppActiveReleaseChanged {
        release_id: String,
        release_digest: DigestHex,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContributionContractImpact {
    Exact,
    Compatible,
    Breaking {
        expected_contract_digest: DigestHex,
        actual_contract_digest: DigestHex,
    },
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevisionUseReadiness {
    Ready,
    Blocked,
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
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ImpactRecoveryAction {
    Retry,
    SwitchSource,
    RestoreSource,
    ForkRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContributionImpact {
    pub lock: ContributionLock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentContribution>,
    pub lifecycle: ContributionLifecycleImpact,
    pub contract: ContributionContractImpact,
    pub new_use: RevisionUseReadiness,
    pub recovery_actions: BTreeSet<ImpactRecoveryAction>,
    /// Same public contribution ID at non-matching provenance. These records
    /// are diagnostic only and are never selected.
    pub ignored_alternate_source_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionImpactSummary {
    pub total: u32,
    pub ready: u32,
    pub blocked: u32,
    pub active_exact: u32,
    pub compatible_replace: u32,
    pub breaking_replace: u32,
    pub disabled: u32,
    pub uninstalled: u32,
    pub unavailable: u32,
    pub active_release_change_compatible: u32,
    pub active_release_change_breaking: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevisionImpactStatus {
    Ready,
    ActionRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionImpactDiff {
    pub catalog_digest: DigestHex,
    pub status: RevisionImpactStatus,
    pub summary: RevisionImpactSummary,
    pub contributions: Vec<ContributionImpact>,
}

/// Compare immutable server-owned locks with one current formal catalog
/// snapshot. The function never mutates a lock and never resolves another
/// source when the exact source key is missing.
pub fn compare_revision_contribution_locks(
    locks: &[ContributionLock],
    current_catalog: &[CurrentContribution],
) -> Result<RevisionImpactDiff, RevisionImpactError> {
    let mut normalized_locks = locks.to_vec();
    normalized_locks.sort();
    let mut lock_ids = BTreeSet::new();
    for lock in &normalized_locks {
        lock.validate()
            .map_err(|violation| RevisionImpactError::InvalidFrozenLock {
                contribution_id: lock.contribution_id.clone(),
                reason: violation.message,
            })?;
        if !lock_ids.insert(lock.contribution_id.clone()) {
            return Err(RevisionImpactError::DuplicateFrozenLock {
                contribution_id: lock.contribution_id.clone(),
            });
        }
    }

    let mut normalized_catalog = current_catalog.to_vec();
    normalized_catalog.sort();
    let catalog_digest =
        digest_payload(&normalized_catalog).map_err(RevisionImpactError::CatalogDigest)?;
    let mut current_by_key = BTreeMap::new();
    for current in &normalized_catalog {
        current.validate()?;
        let key = current.source_key();
        if current_by_key.insert(key.clone(), current).is_some() {
            return Err(RevisionImpactError::DuplicateCurrentContribution {
                contribution_id: key.contribution_id,
            });
        }
    }

    let mut contributions = Vec::with_capacity(normalized_locks.len());
    for lock in normalized_locks {
        let key = source_key_for_lock(&lock);
        let current = current_by_key.get(&key).copied();
        let ignored_alternate_source_count = normalized_catalog
            .iter()
            .filter(|candidate| {
                candidate.contribution_id == lock.contribution_id
                    && candidate.source_key() != key
            })
            .count() as u32;
        contributions.push(compare_one(
            lock,
            current,
            ignored_alternate_source_count,
        ));
    }

    let summary = summarize(&contributions);
    Ok(RevisionImpactDiff {
        catalog_digest,
        status: if summary.blocked == 0 {
            RevisionImpactStatus::Ready
        } else {
            RevisionImpactStatus::ActionRequired
        },
        summary,
        contributions,
    })
}

fn compare_one(
    lock: ContributionLock,
    current: Option<&CurrentContribution>,
    ignored_alternate_source_count: u32,
) -> ContributionImpact {
    let Some(current) = current else {
        return ContributionImpact {
            lock,
            current: None,
            lifecycle: ContributionLifecycleImpact::Uninstalled,
            contract: ContributionContractImpact::Unknown,
            new_use: RevisionUseReadiness::Blocked,
            recovery_actions: BTreeSet::from([
                ImpactRecoveryAction::SwitchSource,
                ImpactRecoveryAction::RestoreSource,
                ImpactRecoveryAction::ForkRevision,
            ]),
            ignored_alternate_source_count,
        };
    };

    let digest_matches = lock.contract_digest == current.contract_digest;
    let lifecycle = match &current.lifecycle {
        CurrentContributionLifecycle::Active => ContributionLifecycleImpact::Active,
        CurrentContributionLifecycle::Replaced { target_digest } => {
            ContributionLifecycleImpact::Replaced {
                target_digest: target_digest.clone(),
            }
        }
        CurrentContributionLifecycle::Disabled { reason } => {
            ContributionLifecycleImpact::Disabled {
                reason: reason.clone(),
            }
        }
        CurrentContributionLifecycle::Unavailable { code, reason } => {
            ContributionLifecycleImpact::Unavailable {
                code: code.clone(),
                reason: reason.clone(),
            }
        }
        CurrentContributionLifecycle::MiniAppActiveReleaseChanged {
            release_id,
            release_digest,
        } => ContributionLifecycleImpact::MiniAppActiveReleaseChanged {
            release_id: release_id.clone(),
            release_digest: release_digest.clone(),
        },
    };
    let contract = if digest_matches {
        match current.lifecycle {
            CurrentContributionLifecycle::Replaced { .. }
            | CurrentContributionLifecycle::MiniAppActiveReleaseChanged { .. } => {
                ContributionContractImpact::Compatible
            }
            _ => ContributionContractImpact::Exact,
        }
    } else {
        ContributionContractImpact::Breaking {
            expected_contract_digest: lock.contract_digest.clone(),
            actual_contract_digest: current.contract_digest.clone(),
        }
    };
    let lifecycle_ready = matches!(
        current.lifecycle,
        CurrentContributionLifecycle::Active
            | CurrentContributionLifecycle::Replaced { .. }
            | CurrentContributionLifecycle::MiniAppActiveReleaseChanged { .. }
    );
    let new_use = if lifecycle_ready && digest_matches {
        RevisionUseReadiness::Ready
    } else {
        RevisionUseReadiness::Blocked
    };
    let recovery_actions = if new_use == RevisionUseReadiness::Ready {
        BTreeSet::new()
    } else {
        match current.lifecycle {
            CurrentContributionLifecycle::Disabled { .. } => BTreeSet::from([
                ImpactRecoveryAction::Retry,
                ImpactRecoveryAction::SwitchSource,
                ImpactRecoveryAction::RestoreSource,
                ImpactRecoveryAction::ForkRevision,
            ]),
            CurrentContributionLifecycle::Unavailable { .. } => BTreeSet::from([
                ImpactRecoveryAction::Retry,
                ImpactRecoveryAction::SwitchSource,
                ImpactRecoveryAction::ForkRevision,
            ]),
            _ => BTreeSet::from([
                ImpactRecoveryAction::SwitchSource,
                ImpactRecoveryAction::RestoreSource,
                ImpactRecoveryAction::ForkRevision,
            ]),
        }
    };

    ContributionImpact {
        lock,
        current: Some(current.clone()),
        lifecycle,
        contract,
        new_use,
        recovery_actions,
        ignored_alternate_source_count,
    }
}

fn summarize(contributions: &[ContributionImpact]) -> RevisionImpactSummary {
    let mut summary = RevisionImpactSummary {
        total: contributions.len() as u32,
        ..RevisionImpactSummary::default()
    };
    for impact in contributions {
        match impact.new_use {
            RevisionUseReadiness::Ready => summary.ready += 1,
            RevisionUseReadiness::Blocked => summary.blocked += 1,
        }
        match impact.lifecycle {
            ContributionLifecycleImpact::Active => {
                if impact.contract == ContributionContractImpact::Exact {
                    summary.active_exact += 1;
                }
            }
            ContributionLifecycleImpact::Replaced { .. } => {
                if impact.contract == ContributionContractImpact::Compatible {
                    summary.compatible_replace += 1;
                }
            }
            ContributionLifecycleImpact::Disabled { .. } => summary.disabled += 1,
            ContributionLifecycleImpact::Unavailable { .. } => summary.unavailable += 1,
            ContributionLifecycleImpact::Uninstalled => summary.uninstalled += 1,
            ContributionLifecycleImpact::MiniAppActiveReleaseChanged { .. } => {
                if impact.contract == ContributionContractImpact::Compatible {
                    summary.active_release_change_compatible += 1;
                } else if matches!(
                    impact.contract,
                    ContributionContractImpact::Breaking { .. }
                ) {
                    summary.active_release_change_breaking += 1;
                }
            }
        }
        if matches!(
            impact.contract,
            ContributionContractImpact::Breaking { .. }
        ) {
            summary.breaking_replace += 1;
        }
    }
    summary
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ContributionSourceKey {
    source_kind: ContributionSourceKind,
    source_identity: StableSourceIdentity,
    mount_id: Option<PluginMountId>,
    miniapp_id: Option<MiniAppId>,
    mcp_binding_id: Option<McpBindingId>,
    contribution_id: ContributionId,
}

fn source_key_for_lock(lock: &ContributionLock) -> ContributionSourceKey {
    ContributionSourceKey {
        source_kind: lock.source_kind,
        source_identity: lock.source_identity.clone(),
        mount_id: lock.mount_id.clone(),
        miniapp_id: lock.miniapp_id.clone(),
        mcp_binding_id: lock.mcp_binding_id.clone(),
        contribution_id: lock.contribution_id.clone(),
    }
}

fn validate_non_empty(
    value: &str,
    contribution_id: &ContributionId,
    field: &'static str,
) -> Result<(), RevisionImpactError> {
    if value.trim().is_empty() || value != value.trim() {
        return Err(RevisionImpactError::InvalidCurrentContribution {
            contribution_id: contribution_id.clone(),
            reason: format!("{field} must be a non-empty canonical value"),
        });
    }
    Ok(())
}

fn validate_digest(
    value: &DigestHex,
    contribution_id: &ContributionId,
    field: &'static str,
) -> Result<(), RevisionImpactError> {
    if value.as_ref().len() != 64
        || !value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RevisionImpactError::InvalidCurrentContribution {
            contribution_id: contribution_id.clone(),
            reason: format!("{field} must be 64 lowercase hexadecimal characters"),
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum RevisionImpactError {
    #[error("frozen contribution lock {contribution_id:?} is invalid: {reason}")]
    InvalidFrozenLock {
        contribution_id: ContributionId,
        reason: String,
    },
    #[error("duplicate frozen contribution lock {contribution_id:?}")]
    DuplicateFrozenLock {
        contribution_id: ContributionId,
    },
    #[error("current contribution {contribution_id:?} is invalid: {reason}")]
    InvalidCurrentContribution {
        contribution_id: ContributionId,
        reason: String,
    },
    #[error("current catalog contains duplicate exact contribution {contribution_id:?}")]
    DuplicateCurrentContribution {
        contribution_id: ContributionId,
    },
    #[error("current catalog digest failed: {0}")]
    CatalogDigest(crate::CanonicalDigestError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: char) -> DigestHex {
        DigestHex::from(byte.to_string().repeat(64))
    }

    fn plugin_lock(mount: &str, contract: char) -> ContributionLock {
        ContributionLock {
            source_kind: ContributionSourceKind::PluginMount,
            source_identity: StableSourceIdentity::from("plugin.example"),
            mount_id: Some(PluginMountId::from(mount)),
            miniapp_id: None,
            mcp_binding_id: None,
            contribution_id: ContributionId::from("capability:example.run"),
            contract_digest: digest(contract),
        }
    }

    fn current_from_lock(
        lock: &ContributionLock,
        contract: char,
        lifecycle: CurrentContributionLifecycle,
    ) -> CurrentContribution {
        CurrentContribution {
            source_kind: lock.source_kind,
            source_identity: lock.source_identity.clone(),
            mount_id: lock.mount_id.clone(),
            miniapp_id: lock.miniapp_id.clone(),
            mcp_binding_id: lock.mcp_binding_id.clone(),
            contribution_id: lock.contribution_id.clone(),
            contract_digest: digest(contract),
            lifecycle,
        }
    }

    #[test]
    fn compatible_and_breaking_replace_are_decided_from_frozen_digest() {
        let lock = plugin_lock("mount-a", 'a');
        let compatible = compare_revision_contribution_locks(
            std::slice::from_ref(&lock),
            &[current_from_lock(
                &lock,
                'a',
                CurrentContributionLifecycle::Replaced {
                    target_digest: Some(digest('1')),
                },
            )],
        )
        .expect("compatible replace");
        assert_eq!(compatible.status, RevisionImpactStatus::Ready);
        assert_eq!(compatible.summary.compatible_replace, 1);
        assert_eq!(
            compatible.contributions[0].contract,
            ContributionContractImpact::Compatible
        );

        let breaking = compare_revision_contribution_locks(
            std::slice::from_ref(&lock),
            &[current_from_lock(
                &lock,
                'b',
                CurrentContributionLifecycle::Replaced {
                    target_digest: Some(digest('2')),
                },
            )],
        )
        .expect("breaking replace");
        assert_eq!(breaking.status, RevisionImpactStatus::ActionRequired);
        assert_eq!(breaking.summary.breaking_replace, 1);
        assert_eq!(
            breaking.contributions[0].new_use,
            RevisionUseReadiness::Blocked
        );
    }

    #[test]
    fn disabled_and_uninstalled_are_distinct_and_never_cross_source_fallback() {
        let lock = plugin_lock("mount-a", 'a');
        let disabled = compare_revision_contribution_locks(
            std::slice::from_ref(&lock),
            &[current_from_lock(
                &lock,
                'a',
                CurrentContributionLifecycle::Disabled {
                    reason: "disabled by owner".into(),
                },
            )],
        )
        .expect("disabled");
        assert_eq!(disabled.summary.disabled, 1);
        assert_eq!(
            disabled.contributions[0].new_use,
            RevisionUseReadiness::Blocked
        );

        let other_source = current_from_lock(
            &plugin_lock("mount-b", 'a'),
            'a',
            CurrentContributionLifecycle::Active,
        );
        let uninstalled = compare_revision_contribution_locks(
            std::slice::from_ref(&lock),
            &[other_source],
        )
        .expect("uninstalled");
        assert_eq!(uninstalled.summary.uninstalled, 1);
        assert_eq!(
            uninstalled.contributions[0].lifecycle,
            ContributionLifecycleImpact::Uninstalled
        );
        assert_eq!(
            uninstalled.contributions[0].ignored_alternate_source_count,
            1
        );
        assert!(uninstalled.contributions[0].current.is_none());
    }

    #[test]
    fn miniapp_active_release_change_is_compatible_or_breaking_without_mutating_lock() {
        let lock = ContributionLock {
            source_kind: ContributionSourceKind::MiniAppActiveRelease,
            source_identity: StableSourceIdentity::from("miniapp.example"),
            mount_id: None,
            miniapp_id: Some(MiniAppId::from("miniapp-1")),
            mcp_binding_id: None,
            contribution_id: ContributionId::from("capability:miniapp.run"),
            contract_digest: digest('a'),
        };
        let original = lock.clone();
        let lifecycle = CurrentContributionLifecycle::MiniAppActiveReleaseChanged {
            release_id: "release-2".into(),
            release_digest: digest('2'),
        };
        let compatible = compare_revision_contribution_locks(
            std::slice::from_ref(&lock),
            &[current_from_lock(&lock, 'a', lifecycle.clone())],
        )
        .expect("compatible active release");
        assert_eq!(compatible.summary.active_release_change_compatible, 1);
        assert_eq!(lock, original, "impact reads must not rewrite old Revision locks");

        let breaking = compare_revision_contribution_locks(
            std::slice::from_ref(&lock),
            &[current_from_lock(&lock, 'b', lifecycle)],
        )
        .expect("breaking active release");
        assert_eq!(breaking.summary.active_release_change_breaking, 1);
        assert_eq!(breaking.summary.breaking_replace, 1);
    }

    #[test]
    fn candidate_and_test_states_cannot_enter_the_formal_impact_catalog() {
        for state in ["ready_candidate", "test_host", "project_source"] {
            let value = serde_json::json!({ "state": state });
            assert!(
                serde_json::from_value::<CurrentContributionLifecycle>(value).is_err(),
                "{state} is not a current formal execution lifecycle"
            );
        }
    }
}
