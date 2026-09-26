-- A device the user forgot: its usage rows are gone, and its file is not read again while
-- its modification time still equals `source_modified_at`, which forgetting sets to the
-- file's time at that moment. A file that changes afterwards is read again as usual.
ALTER TABLE devices ADD COLUMN forgotten INTEGER NOT NULL DEFAULT 0;

-- The identifier this machine publishes its shared file under, kept on the `local` row so
-- that losing the settings file alone does not give the machine a new identity. NULL on
-- every other row, whose `id` already is that device's identifier.
ALTER TABLE devices ADD COLUMN shared_id TEXT;
