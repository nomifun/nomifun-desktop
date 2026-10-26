-- Remove the retired overview page's recent-resource cache.
-- Connection settings, submitted requests and execution receipts remain in use.
ALTER TABLE office_connections DROP COLUMN recent_resources_json;
