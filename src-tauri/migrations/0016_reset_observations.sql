-- A restart is one account-wide event, but each of the user's computers detects it on its
-- own, a second or a few minutes apart. Every detection is kept as an observation of the
-- event it joined, so a surface can say which devices saw it and how far their timing
-- disagreed; `limit_resets` stays the merged event every surface reads, rewritten from
-- whichever observation measured the restart most precisely.
ALTER TABLE limit_resets ADD COLUMN anchor_spread_seconds INTEGER NOT NULL DEFAULT 0;

CREATE TABLE limit_reset_observations (
  id INTEGER PRIMARY KEY,
  reset_id INTEGER NOT NULL,
  provider_instance_id INTEGER NOT NULL,
  -- `local` for this machine, another device's shared identifier, or NULL for a restart
  -- recorded before detections were attributed.
  device TEXT,
  -- The name the device reported itself by, for a device whose own file this machine has
  -- never read because its detection arrived relayed through another device's file.
  device_name TEXT,
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
  -- When the two readings the detection compared were taken. The restart happened between
  -- them, so the narrower the pair, the more precisely it was measured. NULL where the
  -- detection predates the brackets being recorded.
  bracket_start INTEGER,
  bracket_end INTEGER,
  UNIQUE (provider_instance_id, device, window_duration_mins, new_resets_at),
  FOREIGN KEY (reset_id) REFERENCES limit_resets(id) ON DELETE CASCADE,
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_limit_reset_observations_reset ON limit_reset_observations(reset_id);

INSERT INTO limit_reset_observations
  (reset_id, provider_instance_id, device, window_kind, window_duration_mins, anchored_at,
   new_resets_at, previous_resets_at, used_percent_before, early_by_seconds, classification,
   source, detected_at)
SELECT id, provider_instance_id, NULL, window_kind, window_duration_mins, anchored_at,
       new_resets_at, previous_resets_at, used_percent_before, early_by_seconds, classification,
       source, detected_at
FROM limit_resets;
