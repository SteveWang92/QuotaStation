-- The first schema reserved room for event-level ingestion and a database-resident
-- pricing catalogue. Neither arrived: the Codex parser returns daily aggregates and
-- carries its own embedded pricing map, so nothing has ever written these tables.
-- Removing them keeps the schema an honest description of what QuotaStation stores.

-- Rebuilt without cache_write_tokens (never populated) and without the pricing_entries
-- foreign key, which cannot outlive its parent table.
CREATE TABLE daily_usage_rebuilt (
  provider_instance_id INTEGER NOT NULL,
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
  PRIMARY KEY (provider_instance_id, usage_date, model, service_tier),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

INSERT INTO daily_usage_rebuilt
  (provider_instance_id, usage_date, model, service_tier, input_tokens, cache_read_tokens,
   output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at)
SELECT provider_instance_id, usage_date, model, service_tier, input_tokens, cache_read_tokens,
       output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at
FROM daily_usage;

DROP TABLE daily_usage;
ALTER TABLE daily_usage_rebuilt RENAME TO daily_usage;

DROP TABLE usage_events;
DROP TABLE pricing_entries;
DROP TABLE ingestion_cursors;
