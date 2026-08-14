ALTER TABLE limit_current ADD COLUMN source TEXT NOT NULL DEFAULT 'app_server';

UPDATE limit_current
SET source = 'session_log'
WHERE provider_instance_id IN (
  SELECT id FROM provider_instances WHERE provider = 'claude'
);
