-- Reset events are account-wide. A window can move between primary and secondary slots,
-- so the slot must not make two devices' observation of one reset look different.
CREATE TABLE limit_resets_rebuilt (
  id INTEGER PRIMARY KEY,
  provider_instance_id INTEGER NOT NULL,
  window_kind TEXT NOT NULL,
  window_duration_mins INTEGER NOT NULL,
  anchored_at INTEGER NOT NULL,
  new_resets_at INTEGER NOT NULL,
  previous_resets_at INTEGER NOT NULL,
  used_percent_before REAL NOT NULL,
  early_by_seconds INTEGER NOT NULL,
  classification TEXT NOT NULL CHECK (classification IN ('scheduled', 'unplanned')),
  source TEXT NOT NULL CHECK (source IN ('live', 'backfill')),
  detected_at TEXT NOT NULL,
  tokens_in_window INTEGER,
  UNIQUE (provider_instance_id, window_duration_mins, new_resets_at),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO limit_resets_rebuilt
  (id, provider_instance_id, window_kind, window_duration_mins, anchored_at, new_resets_at,
   previous_resets_at, used_percent_before, early_by_seconds, classification, source,
   detected_at, tokens_in_window)
SELECT id, provider_instance_id, window_kind, window_duration_mins, anchored_at, new_resets_at,
       previous_resets_at, used_percent_before, early_by_seconds, classification, source,
       detected_at, tokens_in_window
FROM limit_resets
ORDER BY id;

DROP TABLE limit_resets;
ALTER TABLE limit_resets_rebuilt RENAME TO limit_resets;
CREATE INDEX idx_limit_resets_provider_time ON limit_resets(provider_instance_id, anchored_at DESC);
