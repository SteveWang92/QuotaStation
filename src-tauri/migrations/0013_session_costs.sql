-- What a provider's own client said a session cost, beside what this machine's pricing
-- catalog makes of the same session. Neither figure is a bill: on a subscription nothing is
-- charged per token, so the pair measures whether the catalog still agrees with the vendor's
-- own accounting. A session is the unit because the client reports per session and its
-- record carries no time of its own.
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
  parser_revision TEXT NOT NULL,
  pricing_catalog_revision TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (provider_instance_id, session_id),
  FOREIGN KEY (provider_instance_id) REFERENCES provider_instances(id) ON DELETE CASCADE
);

CREATE INDEX idx_session_costs_provider_time
  ON session_costs(provider_instance_id, session_started_at DESC);
