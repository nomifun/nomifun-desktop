-- The sales product returned to NomiFun's own Agent runtime.
-- Keep migration 060 in history so already-upgraded databases can move
-- forward safely, then remove the no-longer-used external bridge state.
DROP TABLE IF EXISTS sales_kylon_jobs;
DROP TABLE IF EXISTS sales_kylon_bindings;
