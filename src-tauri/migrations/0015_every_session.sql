-- A session is worth listing whether or not its client priced it. The parser knows every
-- session it has entries for; only recent Claude Code sessions carry a cost of the
-- client's own, and Codex records none at all. The client's side of a row therefore
-- becomes optional, and the parser's side — which every session has — carries the rest.
--
-- The table is rebuilt rather than altered, as it was when it gained its detail: no
-- released build has ever displayed these rows, and every session whose log still exists
-- is written again by the next refresh.
DROP TABLE session_costs;

CREATE TABLE session_costs (
  provider_instance_id INTEGER NOT NULL,
  session_id TEXT NOT NULL,
  -- The first and the last entry the parser read for this session, so a session is placed
  -- and measured the same way whatever its client recorded.
  session_started_at TEXT NOT NULL,
  duration_ms INTEGER NOT NULL,
  computed_cost_usd REAL NOT NULL,
  -- Nought once the computed side priced the session from the client's own per-message
  -- figures rather than from the catalog, which makes the two costs one number.
  independent INTEGER NOT NULL,
  -- What the client accounted the session to, and everything else only it can say. All
  -- null for a session whose client recorded nothing, which is every Codex session and
  -- every Claude Code session older than the record.
  reported_cost_usd REAL,
  -- Nought when the client itself could not price a model it used, so its total is short.
  reported_complete INTEGER,
  api_duration_ms INTEGER,
  lines_added INTEGER,
  lines_removed INTEGER,
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
