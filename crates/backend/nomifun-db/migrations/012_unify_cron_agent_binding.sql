-- Normalize only current-generation canonical references. This does not compile
-- an Agent, import retired data, rewrite Session history, or broaden grants.
-- Malformed supported JSON is damage, not authority to silently reset data.
CREATE TEMP TABLE cron_binding_json_proof(valid INTEGER NOT NULL CHECK(valid=1));
INSERT INTO cron_binding_json_proof SELECT CASE WHEN
 (agent_config IS NULL OR (json_valid(agent_config) AND json_type(agent_config)='object')) AND
 (agent_snapshot IS NULL OR (json_valid(agent_snapshot) AND json_type(agent_snapshot)='object'))
 THEN 1 ELSE 0 END FROM cron_jobs;
DROP TABLE cron_binding_json_proof;
ALTER TABLE cron_jobs ADD COLUMN agent_selection TEXT;
ALTER TABLE cron_jobs ADD COLUMN model_selection TEXT;
ALTER TABLE cron_jobs ADD COLUMN agent_binding TEXT;
ALTER TABLE cron_jobs ADD COLUMN clear_context_each_run INTEGER NOT NULL DEFAULT 0 CHECK(clear_context_each_run IN(0,1));
ALTER TABLE cron_jobs ADD COLUMN configuration_error TEXT;
ALTER TABLE cron_jobs ADD COLUMN last_run_id TEXT;

UPDATE cron_jobs SET
    model_selection=CASE WHEN json_valid(agent_config) AND json_type(agent_config,'$.provider_id')='text' AND json_type(agent_config,'$.model')='text'
      THEN json_object('provider_id',json_extract(agent_config,'$.provider_id'),'model',json_extract(agent_config,'$.model')) END,
    agent_selection=CASE WHEN json_valid(agent_config) AND json_type(agent_config,'$.preset_id')='text'
      THEN json_object('kind','preset','preset_id',json_extract(agent_config,'$.preset_id')) END,
    clear_context_each_run=CASE WHEN json_valid(agent_config) AND json_extract(agent_config,'$.clear_context_each_run')=1 THEN 1 ELSE 0 END;

UPDATE cron_jobs AS job SET agent_binding=json_extract(job.agent_snapshot,'$.canonical_binding')
WHERE json_valid(job.agent_snapshot)
  AND json_type(job.agent_snapshot,'$.canonical_binding')='object'
  AND json_type(job.agent_snapshot,'$.canonical_binding.typed_resource_bindings')='array'
  AND (job.preset_id IS NULL OR job.preset_id=json_extract(job.agent_snapshot,'$.preset_id'))
  AND (job.preset_revision IS NULL OR job.preset_revision=json_extract(job.agent_snapshot,'$.preset_revision'))
  AND (job.agent_config IS NULL OR json_type(job.agent_config,'$.agent_snapshot') IS NULL
       OR json(json_extract(job.agent_config,'$.agent_snapshot'))=json(job.agent_snapshot))
  AND EXISTS(
    SELECT 1 FROM agent_preset_revisions revision JOIN agent_runtime_snapshots snapshot
      ON json_extract(snapshot.content_json,'$.preset_revision_ref.preset_id')=revision.preset_id
     AND json_extract(snapshot.content_json,'$.preset_revision_ref.revision')=revision.revision_no
     AND json_extract(snapshot.content_json,'$.preset_revision_ref.revision_digest')=revision.revision_digest
    WHERE revision.created_by=job.user_id AND json_extract(snapshot.envelope_json,'$.actor.principal_kind')='user' AND json_extract(snapshot.envelope_json,'$.actor.principal_id')=job.user_id
      AND revision.preset_id=json_extract(job.agent_snapshot,'$.canonical_binding.preset_revision_ref.preset_id')
      AND revision.revision_no=json_extract(job.agent_snapshot,'$.canonical_binding.preset_revision_ref.revision')
      AND revision.revision_digest=json_extract(job.agent_snapshot,'$.canonical_binding.preset_revision_ref.revision_digest')
      AND snapshot.snapshot_id=json_extract(job.agent_snapshot,'$.canonical_binding.resolved_snapshot_ref.snapshot_id')
      AND snapshot.snapshot_digest=json_extract(job.agent_snapshot,'$.canonical_binding.resolved_snapshot_ref.snapshot_digest')
      AND NOT EXISTS(SELECT 1 FROM json_each(snapshot.content_json,'$.required_resource_kinds') required
        WHERE required.value NOT IN('knowledge_base','channel','robot','canvas','ssh_host','browser','computer')
          AND NOT EXISTS(SELECT 1 FROM json_each(job.agent_snapshot,'$.canonical_binding.typed_resource_bindings') resource
            WHERE json_extract(resource.value,'$.resource_kind')=required.value
              AND json_extract(resource.value,'$.owner_id')=job.user_id))
  )
  AND NOT EXISTS(SELECT 1 FROM json_each(job.agent_snapshot,'$.canonical_binding.typed_resource_bindings') resource
    WHERE json_extract(resource.value,'$.owner_id') IS NOT job.user_id);

-- A per-occurrence task never owns an anchor Session. Retire only this
-- redundant job link; its Session, original reservations and history survive.
UPDATE cron_jobs SET conversation_id=NULL,conversation_title=NULL WHERE execution_mode='new_conversation';
-- An existing live canonical Session remains its own sole binding authority.
-- Unproven old launch definitions remain visible as disabled authoring inputs.
UPDATE cron_jobs SET enabled=0,next_run_at=NULL,schedule_revision=schedule_revision+1,
 configuration_error='CRON_CONFIGURATION_RESELECTION_REQUIRED'
WHERE agent_binding IS NULL AND NOT(execution_mode='existing' AND conversation_id IS NOT NULL
 AND EXISTS(SELECT 1 FROM agent_sessions session WHERE session.agent_session_id=cron_jobs.conversation_id
   AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_id')=cron_jobs.user_id));
UPDATE cron_jobs SET agent_binding=NULL,agent_selection=NULL,model_selection=NULL
 WHERE execution_mode='existing' AND conversation_id IS NOT NULL AND configuration_error IS NULL;

-- Final authorization proof uses the saved typed contracts at this database
-- opening. The marker remains resumable if the process exits after SQL commit.
UPDATE cron_jobs SET configuration_error='CRON_BINDING_VALIDATION_PENDING' WHERE agent_binding IS NOT NULL;
CREATE TEMP TABLE cron_binding_sequence AS SELECT seq FROM sqlite_sequence WHERE name='cron_jobs';
CREATE TABLE cron_jobs_binding_next (
 id INTEGER PRIMARY KEY AUTOINCREMENT,cron_job_id TEXT NOT NULL UNIQUE CHECK(length(cron_job_id) = 36 AND lower(cron_job_id) = cron_job_id AND cron_job_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(cron_job_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
 user_id TEXT NOT NULL CHECK(length(user_id) = 36 AND lower(user_id) = user_id AND user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
 name TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN(0,1)),schedule_revision INTEGER NOT NULL DEFAULT 1 CHECK(schedule_revision>0),
 schedule_kind TEXT NOT NULL CHECK(schedule_kind IN('at','every','cron')),schedule_value TEXT NOT NULL,schedule_tz TEXT,schedule_description TEXT,payload_message TEXT NOT NULL,
 execution_mode TEXT NOT NULL DEFAULT 'existing' CHECK(execution_mode IN('existing','new_conversation')),
 agent_selection TEXT CHECK(agent_selection IS NULL OR (json_valid(agent_selection) AND json_type(agent_selection)='object')),
 model_selection TEXT CHECK(model_selection IS NULL OR (json_valid(model_selection) AND json_type(model_selection)='object')),
 agent_binding TEXT CHECK(agent_binding IS NULL OR (json_valid(agent_binding) AND json_type(agent_binding)='object')),
 clear_context_each_run INTEGER NOT NULL DEFAULT 0 CHECK(clear_context_each_run IN(0,1)),configuration_error TEXT,conversation_id TEXT CHECK(conversation_id IS NULL OR(length(conversation_id) = 36 AND lower(conversation_id) = conversation_id AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
 conversation_title TEXT,created_by TEXT NOT NULL CHECK(created_by IN('user','agent')),skill_content TEXT,description TEXT,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,next_run_at INTEGER,last_run_at INTEGER,
 last_run_id TEXT CHECK(last_run_id IS NULL OR(length(last_run_id) = 36 AND lower(last_run_id) = last_run_id AND last_run_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(last_run_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
 last_status TEXT CHECK(last_status IN('ok','error','skipped','missed','input_required','cancelled')),last_error TEXT,run_count INTEGER NOT NULL DEFAULT 0
);
INSERT INTO cron_jobs_binding_next (id,cron_job_id,user_id,name,enabled,schedule_revision,schedule_kind,schedule_value,schedule_tz,schedule_description,payload_message,execution_mode,agent_selection,model_selection,agent_binding,clear_context_each_run,configuration_error,conversation_id,conversation_title,created_by,skill_content,description,created_at,updated_at,next_run_at,last_run_at,last_run_id,last_status,last_error,run_count) SELECT id,cron_job_id,user_id,name,enabled,schedule_revision,schedule_kind,schedule_value,schedule_tz,schedule_description,payload_message,execution_mode,agent_selection,model_selection,agent_binding,clear_context_each_run,configuration_error,conversation_id,conversation_title,created_by,skill_content,description,created_at,updated_at,next_run_at,last_run_at,last_run_id,last_status,last_error,run_count FROM cron_jobs;
DROP TABLE cron_jobs;
ALTER TABLE cron_jobs_binding_next RENAME TO cron_jobs;
UPDATE sqlite_sequence SET seq=MAX(seq,COALESCE((SELECT seq FROM cron_binding_sequence),0)) WHERE name='cron_jobs';
DROP TABLE cron_binding_sequence;
CREATE INDEX idx_cron_jobs_next_run ON cron_jobs(enabled,next_run_at);
CREATE INDEX idx_cron_jobs_owner_conversation ON cron_jobs(user_id,conversation_id,created_at);
ALTER TABLE cron_run_reservations ADD COLUMN domain_failure_json TEXT CHECK(domain_failure_json IS NULL OR (json_valid(domain_failure_json) AND json_type(domain_failure_json)='object'));
-- Terminal reservations already contain every historical presentation value.
-- They retain the original identity and are never pruned with the seven-row UI.
DROP TABLE cron_job_runs;
DROP INDEX idx_cron_run_reservations_cron_job_id;
CREATE INDEX idx_cron_run_reservations_cron_job_id ON cron_run_reservations(cron_job_id,settled_at_ms DESC,id DESC,status);
