-- Short ranges are read hour by hour: three columns say nothing about when a day's work
-- happened. The rows mirror daily_usage exactly, keyed by the local hour a bucket opened,
-- and retention keeps them only as long as such a range can reach back.
CREATE TABLE hourly_usage (
  provider_instance_id INTEGER NOT NULL,
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
  PRIMARY KEY (provider_instance_id, hour_start, model, service_tier),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_hourly_usage_provider_time
  ON hourly_usage(provider_instance_id, hour_start DESC);
