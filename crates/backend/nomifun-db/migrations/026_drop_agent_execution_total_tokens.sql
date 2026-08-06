-- agent_executions.total_tokens has been write-only since the v3 baseline:
-- the scheduler summed per-attempt tokens into it at terminal states, but no
-- Rust logic, SQL clause, or UI code ever read the value back (the DAG canvas
-- renders per-attempt tokens instead). Drop the storage. The column-level
-- CHECK travels with the column; no index or table-level constraint
-- references it, so SQLite DROP COLUMN is legal here.
ALTER TABLE agent_executions DROP COLUMN total_tokens;
