-- Retire the manual-office history list's ordering and display metadata.
-- Request identities, pending targets and execution receipts remain in use.
DROP INDEX idx_office_operations_owner_history;
ALTER TABLE office_operations DROP COLUMN created_at;
ALTER TABLE office_operations DROP COLUMN updated_at;
