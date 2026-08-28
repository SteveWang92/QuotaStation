-- Token totals are parsed from one machine's session logs, so a second machine's work is
-- missing from every figure built out of them. Usage rows gain the device that produced
-- them: this machine writes its own under `local` and reads every other device's out of
-- the exported aggregates a shared folder carries.
--
-- `local` rather than the device's own identifier because that identifier lives in the
-- settings file, which a migration cannot read — and because a row this machine wrote is
-- this machine's whichever identifier it later reports itself by.
CREATE TABLE devices (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  -- When this device's aggregates were last read in, and the modification time of the file
  -- they were read from, which is what decides whether the next refresh has to read it
  -- again. Both are NULL for the local device: it has nothing to import.
  last_import_at TEXT,
  source_modified_at INTEGER
);

INSERT INTO devices (id, display_name) VALUES ('local', 'This machine');

CREATE TABLE daily_usage_rebuilt (
  provider_instance_id INTEGER NOT NULL,
  device TEXT NOT NULL,
  usage_date TEXT NOT NULL,
  model TEXT NOT NULL,
  service_tier TEXT NOT NULL,
  input_tokens INTEGER NOT NULL,
  cache_read_tokens INTEGER NOT NULL,
  output_tokens INTEGER NOT NULL,
  reasoning_tokens INTEGER NOT NULL,
  total_tokens INTEGER NOT NULL,
  estimated_cost_usd REAL,
  parser_revision TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (provider_instance_id, device, usage_date, model, service_tier),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE,
  FOREIGN KEY (device) REFERENCES devices(id) ON DELETE CASCADE
);

INSERT INTO daily_usage_rebuilt
  (provider_instance_id, device, usage_date, model, service_tier, input_tokens, cache_read_tokens,
   output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at)
SELECT provider_instance_id, 'local', usage_date, model, service_tier, input_tokens, cache_read_tokens,
       output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at
FROM daily_usage;

DROP TABLE daily_usage;
ALTER TABLE daily_usage_rebuilt RENAME TO daily_usage;

CREATE TABLE hourly_usage_rebuilt (
  provider_instance_id INTEGER NOT NULL,
  device TEXT NOT NULL,
  hour_start TEXT NOT NULL,
  model TEXT NOT NULL,
  service_tier TEXT NOT NULL,
  input_tokens INTEGER NOT NULL,
  cache_read_tokens INTEGER NOT NULL,
  output_tokens INTEGER NOT NULL,
  reasoning_tokens INTEGER NOT NULL,
  total_tokens INTEGER NOT NULL,
  estimated_cost_usd REAL,
  parser_revision TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (provider_instance_id, device, hour_start, model, service_tier),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE,
  FOREIGN KEY (device) REFERENCES devices(id) ON DELETE CASCADE
);

INSERT INTO hourly_usage_rebuilt
  (provider_instance_id, device, hour_start, model, service_tier, input_tokens, cache_read_tokens,
   output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at)
SELECT provider_instance_id, 'local', hour_start, model, service_tier, input_tokens, cache_read_tokens,
       output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at
FROM hourly_usage;

DROP TABLE hourly_usage;
ALTER TABLE hourly_usage_rebuilt RENAME TO hourly_usage;

CREATE INDEX idx_hourly_usage_provider_time
  ON hourly_usage(provider_instance_id, hour_start DESC);
