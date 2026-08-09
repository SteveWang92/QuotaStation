PRAGMA foreign_keys = ON;

CREATE TABLE provider_instances (
  id INTEGER PRIMARY KEY,
  provider TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  parser_revision TEXT,
  plan_type TEXT,
  earned_reset_count INTEGER,
  last_live_success_at TEXT,
  last_history_success_at TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE limit_current (
  provider_instance_id INTEGER NOT NULL,
  window_kind TEXT NOT NULL,
  used_percent REAL,
  window_duration_mins INTEGER,
  resets_at INTEGER,
  observed_at TEXT NOT NULL,
  PRIMARY KEY (provider_instance_id, window_kind),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE TABLE limit_samples (
  id INTEGER PRIMARY KEY,
  provider_instance_id INTEGER NOT NULL,
  window_kind TEXT NOT NULL,
  used_percent REAL,
  window_duration_mins INTEGER,
  resets_at INTEGER,
  observed_at TEXT NOT NULL,
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_limit_samples_provider_time
  ON limit_samples(provider_instance_id, observed_at DESC);

CREATE TABLE usage_events (
  id INTEGER PRIMARY KEY,
  provider_instance_id INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  occurred_at TEXT NOT NULL,
  model TEXT,
  service_tier TEXT,
  input_tokens INTEGER NOT NULL,
  cache_read_tokens INTEGER NOT NULL,
  cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL,
  reasoning_tokens INTEGER NOT NULL,
  total_tokens INTEGER NOT NULL,
  parser_revision TEXT NOT NULL,
  UNIQUE (provider_instance_id, source_event_id),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_usage_events_provider_time
  ON usage_events(provider_instance_id, occurred_at DESC);

CREATE TABLE pricing_entries (
  id INTEGER PRIMARY KEY,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  service_tier TEXT NOT NULL,
  input_usd_per_token REAL NOT NULL,
  cache_read_usd_per_token REAL,
  cache_write_usd_per_token REAL,
  output_usd_per_token REAL NOT NULL,
  effective_at TEXT NOT NULL,
  source_url TEXT NOT NULL,
  UNIQUE(provider, model, service_tier, effective_at)
);

CREATE TABLE daily_usage (
  provider_instance_id INTEGER NOT NULL,
  usage_date TEXT NOT NULL,
  model TEXT NOT NULL,
  service_tier TEXT NOT NULL,
  input_tokens INTEGER NOT NULL,
  cache_read_tokens INTEGER NOT NULL,
  cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL,
  reasoning_tokens INTEGER NOT NULL,
  total_tokens INTEGER NOT NULL,
  estimated_cost_usd REAL,
  pricing_entry_id INTEGER,
  parser_revision TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (provider_instance_id, usage_date, model, service_tier),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE,
  FOREIGN KEY (pricing_entry_id) REFERENCES pricing_entries(id)
);

CREATE TABLE ingestion_cursors (
  provider_instance_id INTEGER NOT NULL,
  acquisition_path TEXT NOT NULL,
  cursor_value TEXT,
  source_revision TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (provider_instance_id, acquisition_path),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE TABLE refresh_runs (
  id INTEGER PRIMARY KEY,
  provider_instance_id INTEGER NOT NULL,
  acquisition_path TEXT NOT NULL,
  started_at TEXT NOT NULL,
  completed_at TEXT,
  status TEXT NOT NULL,
  error_code TEXT,
  error_message TEXT,
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

INSERT INTO provider_instances (provider, display_name, parser_revision)
VALUES ('codex', 'Codex', '033c1f7631f603fc939fdc85163e8203f0084f83');
