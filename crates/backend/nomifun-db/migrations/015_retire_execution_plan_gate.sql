-- Planning approval has no active API, DTO or scheduling consumer. Retire
-- only its unused column; preserve execution identities, facts and policies.
ALTER TABLE agent_executions DROP COLUMN plan_gate;
