-- Cancellation belongs to the original immutable occurrence. A reserved run
-- remains nonterminal until its exact Session/native owner has been observed;
-- cancelling it never disables or rewrites the recurring job.
ALTER TABLE cron_run_reservations ADD COLUMN cancel_requested_at_ms INTEGER;
