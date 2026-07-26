//! Resolve the conversation workspace directory.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Priority: `--work-dir` CLI flag → pending atomic work-root reset → pending
/// immutable reset plan → persisted UI choice (`dir-config.json` in
/// `data_dir`) → strictly verified legacy-v1 repair → `NOMIFUN_WORK_DIR` env
/// (when non-empty) → `--data-dir` fallback.
///
/// The persisted choice sits **above** the env var on purpose: `relaunch()`
/// restarts the whole process and the child can inherit the `NOMIFUN_WORK_DIR`
/// the previous boot exported (see `environment.rs`), so a UI change must win
/// over that stale inherited value.
pub(crate) fn resolve_work_dir(
    cli_work_dir: Option<PathBuf>,
    data_dir: &Path,
) -> Result<PathBuf> {
    if let Some(cli) = cli_work_dir {
        return Ok(cli);
    }
    if let Some(requested) =
        nomifun_common::factory_reset::requested_v3_reset_work_dir(data_dir)?
    {
        return Ok(requested);
    }
    if let Some(work_dir) =
        nomifun_common::factory_reset::pending_v3_reset_work_dir(data_dir)?
    {
        return Ok(work_dir);
    }
    match nomifun_common::dir_config::checked_persisted_work_dir(data_dir) {
        Ok(Some(persisted)) => return Ok(persisted),
        Ok(None) => {}
        Err(config_error) => {
            if !nomifun_common::dir_config::repairable_malformed_work_dir_exists(
                data_dir,
            )? {
                return Err(config_error.into());
            }

            // Old releases wrote dir-config non-atomically. A truncated JSON
            // file is ignored only long enough to select a receipt candidate
            // or run the read-only database classifier. Owner/config writes
            // are deferred until SQLite itself proves the v3 lineage.
            if let Ok(Some(finalized_work)) =
                nomifun_common::factory_reset::finalized_v3_work_dir(data_dir)
            {
                tracing::warn!(
                    target: "boot",
                    work_dir = %finalized_work.display(),
                    "deferred truncated dir-config repair until the database proves its v3 lineage"
                );
                return Ok(finalized_work);
            }

            tracing::warn!(
                target: "boot",
                error = %config_error,
                "temporarily ignoring a truncated legacy dir-config until the read-only dataset probe proves a safe bootstrap/reset"
            );
            return Ok(data_dir.to_path_buf());
        }
    }
    // A failed in-process relaunch may have left NOMIFUN_WORK_DIR pointing at
    // the fallback data dir. Strong same-generation recovery evidence must
    // win over that stale inherited env value, and is atomically persisted so
    // subsequent boots no longer depend on the retired dataset.
    if let Some(repaired) =
        nomifun_common::factory_reset::repair_finalized_legacy_v1_work_dir(
            data_dir,
        )?
    {
        return Ok(repaired);
    }
    if let Some(from_env) = std::env::var("NOMIFUN_WORK_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    {
        return Ok(from_env);
    }
    Ok(data_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::{dir_config, now_ms};

    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nomifun-wdres-{tag}-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rewrite_active_plan_as_released_v1(
        data_dir: &Path,
        mut plan: nomifun_common::factory_reset::DatasetResetPlan,
    ) -> nomifun_common::factory_reset::DatasetResetPlan {
        use nomifun_common::dataset_roots::{
            AGENT_PROCESS_REGISTRY_FILE, WORK_ROOT_BINDING_FILE,
        };
        use nomifun_common::factory_reset::{
            ManagedRootBase, ManagedRootKind, ManagedRootPlan,
            V3_DATASET_RESET_DIR, V3_DATASET_RESET_PLAN_FILE,
        };

        plan.version = 1;
        plan.persist_work_dir = false;
        plan.automatic_legacy_retirement = false;
        plan.roots.retain(|root| {
            root.relative_path != WORK_ROOT_BINDING_FILE
                && root.relative_path != AGENT_PROCESS_REGISTRY_FILE
        });
        let insertion_index = plan
            .roots
            .iter()
            .position(|root| root.relative_path == "encryption_key")
            .expect("released registry contains encryption_key")
            + 1;
        plan.roots.insert(
            insertion_index,
            ManagedRootPlan {
                base: ManagedRootBase::DataDir,
                relative_path: dir_config::DIR_CONFIG_FILE.into(),
                retired_relative_path: format!(
                    "{}/{}",
                    plan.retired_dir,
                    dir_config::DIR_CONFIG_FILE
                ),
                kind: ManagedRootKind::File,
                initially_present: data_dir
                    .join(dir_config::DIR_CONFIG_FILE)
                    .is_file(),
            },
        );
        let mut released = serde_json::to_value(&plan).unwrap();
        let object = released.as_object_mut().unwrap();
        object.remove("persist_work_dir");
        object.remove("automatic_legacy_retirement");
        std::fs::write(
            data_dir
                .join(V3_DATASET_RESET_DIR)
                .join(V3_DATASET_RESET_PLAN_FILE),
            serde_json::to_vec_pretty(&released).unwrap(),
        )
        .unwrap();
        plan
    }

    #[test]
    fn persisted_work_dir_is_used_when_no_cli_flag() {
        let data_dir = temp_data_dir("persisted");
        let chosen = data_dir.join("chosen-ws");
        dir_config::set_work_dir(&data_dir, &chosen).unwrap();

        // Persisted value takes priority over the data_dir fallback even with no
        // CLI flag — this is what makes a UI-chosen work dir stick across boots.
        assert_eq!(resolve_work_dir(None, &data_dir).unwrap(), chosen);

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn cli_flag_wins_over_persisted_config() {
        let data_dir = temp_data_dir("cliwins");
        let persisted = data_dir.join("persisted-ws");
        dir_config::set_work_dir(&data_dir, &persisted).unwrap();
        let cli = data_dir.join("cli-ws");

        assert_eq!(
            resolve_work_dir(Some(cli.clone()), &data_dir).unwrap(),
            cli
        );

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn truncated_legacy_config_temporarily_falls_back_without_rewriting() {
        let data_dir = temp_data_dir("malformed");
        std::fs::write(
            data_dir.join(dir_config::DIR_CONFIG_FILE),
            b"not valid json",
        )
        .unwrap();

        assert_eq!(
            resolve_work_dir(None, &data_dir).unwrap(),
            data_dir,
        );
        assert!(
            dir_config::checked_persisted_work_dir(&data_dir).is_err(),
            "the resolver may defer repair but cannot rewrite without lifecycle proof"
        );

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn valid_but_unsafe_config_still_blocks_fallback() {
        let data_dir = temp_data_dir("unsafe");
        std::fs::write(
            data_dir.join(dir_config::DIR_CONFIG_FILE),
            br#"{"work_dir":"relative/path"}"#,
        )
        .unwrap();

        let error = resolve_work_dir(None, &data_dir).unwrap_err();
        assert!(error.to_string().contains("non-absolute"));

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn completed_old_request_replay_cannot_redirect_the_real_resolver() {
        let data = tempfile::tempdir().unwrap();
        let first_work = tempfile::tempdir().unwrap();
        let first_work_path =
            std::fs::canonicalize(first_work.path()).unwrap();
        let second_work = tempfile::tempdir().unwrap();
        let second_work_path =
            std::fs::canonicalize(second_work.path()).unwrap();
        std::fs::write(data.path().join("nomifun-backend.db"), b"old")
            .unwrap();

        nomifun_common::factory_reset::request_v3_dataset_reset_for_work_dir(
            data.path(),
            &first_work_path,
        )
        .unwrap();
        let first_request = std::fs::read(
            data.path().join(
                nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE,
            ),
        )
        .unwrap();
        nomifun_common::factory_reset::prepare_v3_dataset(
            data.path(),
            &first_work_path,
        )
        .unwrap();
        let first_plan =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                &first_work_path,
            )
            .unwrap()
            .unwrap();
        std::fs::write(data.path().join("nomifun-backend.db"), b"first")
            .unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
            data.path(),
            &first_work_path,
            &first_plan.generation,
        )
        .unwrap();
        nomifun_common::factory_reset::finalize_v3_dataset_reset(
            data.path(),
            &first_work_path,
        )
        .unwrap();

        nomifun_common::factory_reset::request_v3_dataset_reset_for_work_dir(
            data.path(),
            &second_work_path,
        )
        .unwrap();
        nomifun_common::factory_reset::prepare_v3_dataset(
            data.path(),
            &second_work_path,
        )
        .unwrap();
        let second_plan =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                &second_work_path,
            )
            .unwrap()
            .unwrap();
        std::fs::write(data.path().join("nomifun-backend.db"), b"second")
            .unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
            data.path(),
            &second_work_path,
            &second_plan.generation,
        )
        .unwrap();
        nomifun_common::factory_reset::finalize_v3_dataset_reset(
            data.path(),
            &second_work_path,
        )
        .unwrap();
        std::fs::create_dir_all(second_work_path.join("conversations"))
            .unwrap();
        let sentinel =
            second_work_path.join("conversations/current-v3");
        std::fs::write(&sentinel, b"current").unwrap();
        let generation =
            std::fs::read(data.path().join("storage-generation")).unwrap();

        std::fs::write(
            data.path().join(
                nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE,
            ),
            first_request,
        )
        .unwrap();
        first_work.close().unwrap();

        assert_eq!(
            resolve_work_dir(None, data.path()).unwrap(),
            second_work_path,
            "the resolver must ignore the permanently consumed old request before canonicalizing its missing target"
        );
        assert_eq!(
            nomifun_common::factory_reset::prepare_v3_dataset(
                data.path(),
                &second_work_path,
            )
            .unwrap(),
            nomifun_common::factory_reset::DatasetPreparation::Unchanged
        );
        assert!(sentinel.is_file());
        assert_eq!(
            std::fs::read(data.path().join("storage-generation")).unwrap(),
            generation
        );
    }

    #[test]
    fn ignored_v1_control_replay_cannot_redirect_or_clear_a_later_v3_root() {
        use nomifun_common::factory_reset::{
            DatasetPreparation, DatasetResetReason, RETIRED_DATASETS_DIR,
            V3_DATASET_RECEIPT_FILE, V3_DATASET_RESET_DIR,
            V3_DATASET_RESET_PLAN_FILE,
        };

        let data = tempfile::tempdir().unwrap();
        let first_work = tempfile::tempdir().unwrap();
        let first_work_path =
            std::fs::canonicalize(first_work.path()).unwrap();
        let second_work = tempfile::tempdir().unwrap();
        let second_work_path =
            std::fs::canonicalize(second_work.path()).unwrap();
        dir_config::set_work_dir(data.path(), &first_work_path).unwrap();
        std::fs::write(data.path().join("nomifun-backend.db"), b"old")
            .unwrap();

        let first_plan =
            nomifun_common::factory_reset::arm_v3_dataset_reset(
                data.path(),
                &first_work_path,
                DatasetResetReason::NonV3Dataset,
            )
            .unwrap();
        let first_plan = rewrite_active_plan_as_released_v1(
            data.path(),
            first_plan,
        );
        assert_eq!(
            nomifun_common::factory_reset::prepare_v3_dataset(
                data.path(),
                &first_work_path,
            )
            .unwrap(),
            DatasetPreparation::Unchanged
        );
        let archived_control = data
            .path()
            .join(RETIRED_DATASETS_DIR)
            .join("ignored-legacy-reset-plans")
            .join(&first_plan.operation_id);
        let replay = std::fs::read_dir(&archived_control)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<Vec<_>>();

        nomifun_common::factory_reset::request_v3_dataset_reset_for_work_dir(
            data.path(),
            &second_work_path,
        )
        .unwrap();
        assert_eq!(
            nomifun_common::factory_reset::prepare_v3_dataset(
                data.path(),
                &second_work_path,
            )
            .unwrap(),
            DatasetPreparation::ResetApplied
        );
        let current_plan =
            nomifun_common::factory_reset::read_pending_v3_reset(
                data.path(),
                &second_work_path,
            )
            .unwrap()
            .unwrap();
        std::fs::write(
            data.path().join("nomifun-backend.db"),
            b"current-v3",
        )
        .unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
            data.path(),
            &second_work_path,
            &current_plan.generation,
        )
        .unwrap();
        nomifun_common::factory_reset::finalize_v3_dataset_reset(
            data.path(),
            &second_work_path,
        )
        .unwrap();
        std::fs::create_dir_all(second_work_path.join("conversations"))
            .unwrap();
        let sentinel =
            second_work_path.join("conversations/current-v3");
        std::fs::write(&sentinel, b"current").unwrap();
        let generation =
            std::fs::read(data.path().join("storage-generation")).unwrap();
        let receipt =
            std::fs::read(data.path().join(V3_DATASET_RECEIPT_FILE))
                .unwrap();

        let active_control = data.path().join(V3_DATASET_RESET_DIR);
        std::fs::create_dir(&active_control).unwrap();
        for (name, bytes) in replay {
            std::fs::write(active_control.join(name), bytes).unwrap();
        }
        std::fs::write(
            active_control.join("phase-generation-installed"),
            b"v1\n",
        )
        .unwrap();
        assert!(
            active_control.join(V3_DATASET_RESET_PLAN_FILE).is_file()
        );
        first_work.close().unwrap();

        assert_eq!(
            resolve_work_dir(None, data.path()).unwrap(),
            second_work_path,
            "a permanently ignored v1 control must not win over the current dir-config"
        );
        assert_eq!(
            nomifun_common::factory_reset::prepare_v3_dataset(
                data.path(),
                &second_work_path,
            )
            .unwrap(),
            DatasetPreparation::Unchanged
        );
        assert_eq!(
            std::fs::read(data.path().join("nomifun-backend.db")).unwrap(),
            b"current-v3"
        );
        assert!(sentinel.is_file());
        assert_eq!(
            std::fs::read(data.path().join("storage-generation")).unwrap(),
            generation
        );
        assert_eq!(
            std::fs::read(data.path().join(V3_DATASET_RECEIPT_FILE))
                .unwrap(),
            receipt
        );
        assert!(!active_control.exists());
    }
}
