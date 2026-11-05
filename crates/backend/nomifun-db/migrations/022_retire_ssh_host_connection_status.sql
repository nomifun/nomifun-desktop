-- Live SSH state belongs to the process pool's exact Session/host link.
-- Preserve saved profiles, credentials and negotiated host-key fingerprints.
ALTER TABLE ssh_hosts DROP COLUMN status;
ALTER TABLE ssh_hosts DROP COLUMN last_connected_at;
