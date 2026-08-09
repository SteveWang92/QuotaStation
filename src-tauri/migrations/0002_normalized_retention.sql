CREATE TABLE limit_rollups (
  id INTEGER PRIMARY KEY,
  provider_instance_id INTEGER NOT NULL,
  granularity TEXT NOT NULL CHECK (granularity IN ('hourly', 'daily')),
  bucket_start TEXT NOT NULL,
  bucket_end TEXT NOT NULL,
  window_kind TEXT NOT NULL,
  window_duration_mins INTEGER,
  resets_at INTEGER,
  reset_segment TEXT NOT NULL,
  start_used_percent REAL,
  end_used_percent REAL,
  min_used_percent REAL,
  max_used_percent REAL,
  average_used_percent REAL,
  sample_count INTEGER NOT NULL,
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE,
  UNIQUE (provider_instance_id, granularity, bucket_start, window_kind, reset_segment)
);

CREATE INDEX idx_limit_rollups_provider_time
  ON limit_rollups(provider_instance_id, granularity, bucket_start DESC);

CREATE TABLE retention_state (
  job_name TEXT PRIMARY KEY,
  last_completed_at TEXT,
  last_status TEXT NOT NULL,
  last_error TEXT
);
