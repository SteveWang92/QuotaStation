-- Codex restarts a quota window whenever the server decides to, not only when the window
-- it published was due to expire. The samples that record such an event age out under the
-- normal retention rules, so the event itself is stored separately and kept indefinitely:
-- a handful of rows a month, and the only lasting record of why a reset time moved.
CREATE TABLE limit_resets (
  id INTEGER PRIMARY KEY,
  provider_instance_id INTEGER NOT NULL,
  window_kind TEXT NOT NULL,
  window_duration_mins INTEGER NOT NULL,
  -- The first request after the reset anchors the new window, so the reset instant is
  -- recovered from the new expiry rather than from when this machine noticed it.
  anchored_at INTEGER NOT NULL,
  new_resets_at INTEGER NOT NULL,
  previous_resets_at INTEGER NOT NULL,
  used_percent_before REAL NOT NULL,
  early_by_seconds INTEGER NOT NULL,
  classification TEXT NOT NULL CHECK (classification IN ('scheduled', 'unplanned')),
  source TEXT NOT NULL CHECK (source IN ('live', 'backfill')),
  detected_at TEXT NOT NULL,
  -- Live reads and the rollout backfill observe the same server state, so one event must
  -- not be stored twice because two sources saw it.
  UNIQUE (provider_instance_id, window_kind, window_duration_mins, new_resets_at),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_limit_resets_provider_time
  ON limit_resets(provider_instance_id, anchored_at DESC);
