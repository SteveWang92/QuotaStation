-- A quota sample is one reading of one window, identified by when it was observed. A cached
-- status-line reading is offered again on every refresh until a newer one replaces it, and a
-- copy of it says nothing the first row did not, so each reading keeps its earliest row and
-- the index refuses the rest.
DELETE FROM limit_samples
WHERE id NOT IN (
  SELECT MIN(id) FROM limit_samples GROUP BY provider_instance_id, window_kind, observed_at
);

CREATE UNIQUE INDEX idx_limit_samples_reading
  ON limit_samples(provider_instance_id, window_kind, observed_at);
