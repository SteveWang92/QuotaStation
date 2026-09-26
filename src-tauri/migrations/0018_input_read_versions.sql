-- A marker that an input was read records which build's reading it was. A file or log an
-- earlier build consumed may carry content that build could not understand, so a build
-- that reads a later version than the one stored reads the input again, however unchanged
-- it looks. NULL is a reading made before versions were recorded, older than any version.

-- The shared-file format version this machine read the device's file at.
ALTER TABLE devices ADD COLUMN import_format_version INTEGER;

-- The reader version a job's watermark was reached at. Only the restart backfill sets it.
ALTER TABLE retention_state ADD COLUMN reader_version INTEGER;
