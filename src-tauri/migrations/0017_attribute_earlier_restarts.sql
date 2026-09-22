-- Restarts recorded before detections were attributed carry no device. A restart this
-- machine saw leaves both of its readings in `limit_samples` — one still publishing the
-- expiry the restart replaced, one already publishing the new one — so those are this
-- machine's own, bracketed by the last reading before and the first after. Only readings
-- with a percentage count: a window recovered from session logs has none and never evidences
-- a restart.
UPDATE limit_reset_observations
SET device = 'local',
    bracket_start = (
      SELECT MAX(unixepoch(observed_at)) FROM limit_samples AS s
      WHERE s.provider_instance_id = limit_reset_observations.provider_instance_id
        AND s.window_duration_mins = limit_reset_observations.window_duration_mins
        AND s.resets_at = limit_reset_observations.previous_resets_at
        AND s.used_percent IS NOT NULL),
    bracket_end = (
      SELECT MIN(unixepoch(observed_at)) FROM limit_samples AS s
      WHERE s.provider_instance_id = limit_reset_observations.provider_instance_id
        AND s.window_duration_mins = limit_reset_observations.window_duration_mins
        AND s.resets_at = limit_reset_observations.new_resets_at
        AND s.used_percent IS NOT NULL)
WHERE device IS NULL
  AND EXISTS (
    SELECT 1 FROM limit_samples AS before
    JOIN limit_samples AS after
      ON after.provider_instance_id = before.provider_instance_id
     AND after.window_duration_mins = before.window_duration_mins
     AND after.resets_at = limit_reset_observations.new_resets_at
     AND after.used_percent IS NOT NULL
     AND unixepoch(after.observed_at) >= unixepoch(before.observed_at)
    WHERE before.provider_instance_id = limit_reset_observations.provider_instance_id
      AND before.window_duration_mins = limit_reset_observations.window_duration_mins
      AND before.resets_at = limit_reset_observations.previous_resets_at
      AND before.used_percent IS NOT NULL);

-- Older Codex restarts were recovered from its rollout logs, which are still on disk.
-- Forgetting how far the rollout scan got makes the next start replay every rollout, and
-- each restart it finds is filed as this machine's with its own bracket. What is left
-- without a device was detected by another machine, and is attributed when that machine's
-- attributed detections arrive through the shared folder.
DELETE FROM retention_state WHERE job_name = 'codex_reset_backfill';
