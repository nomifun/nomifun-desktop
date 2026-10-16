-- Uploads use the file owner's temporary staging directory. The removed
-- setting had no file-ingress consumer in any supported shell or WebUI.
-- Preserve published receipts and every other setting and canonical fact.
ALTER TABLE system_settings DROP COLUMN save_upload_to_workspace;
DELETE FROM client_preferences WHERE key IN ('upload.saveToWorkspace', 'saveUploadToWorkspace');
