-- A session's comparison is worth reading beside the work it describes: how long the
-- session ran, what it spent its tokens on, and how much code it changed. Claude Code
-- writes all of it into the same record the cost came from, so the row widens rather than
-- gaining a table.
--
-- The table is rebuilt rather than altered. No surface has ever displayed these rows, and
-- every session whose log still exists is written again by the next refresh, so nothing a
-- reader could have seen is lost.
DROP TABLE session_costs;

CREATE TABLE session_costs (
  provider_instance_id INTEGER NOT NULL,
  session_id TEXT NOT NULL,
  session_started_at TEXT NOT NULL,
  reported_cost_usd REAL NOT NULL,
  computed_cost_usd REAL NOT NULL,
  -- Nought once the computed side priced the session from the client's own per-message
  -- figures rather than from the catalog, which makes the two columns one number.
  independent INTEGER NOT NULL,
  -- Nought when the client itself could not price a model it used, so its total is short.
  reported_complete INTEGER NOT NULL,
  -- How long the session ran, and how much of that was spent waiting on the provider.
  total_duration_ms INTEGER NOT NULL,
  api_duration_ms INTEGER NOT NULL,
  lines_added INTEGER NOT NULL,
  lines_removed INTEGER NOT NULL,
  -- The four categories every other usage figure in the application is counted in, taken
  -- from the same parsed entries the computed cost was priced from.
  input_tokens INTEGER NOT NULL,
  cache_read_tokens INTEGER NOT NULL,
  output_tokens INTEGER NOT NULL,
  reasoning_tokens INTEGER NOT NULL,
  -- The parser's own total, which carries the tokens none of the four categories name.
  total_tokens INTEGER NOT NULL,
  -- The models the session used, most expensive first, comma separated. A session names a
  -- handful at most and nothing queries them, so they are read back as one string.
  models TEXT NOT NULL,
  parser_revision TEXT NOT NULL,
  pricing_catalog_revision TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (provider_instance_id, session_id),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_session_costs_provider_time
  ON session_costs(provider_instance_id, session_started_at DESC);
