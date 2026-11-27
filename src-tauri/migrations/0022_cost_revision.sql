-- The revision of the parsers' cost calculation a row was priced under. A build that
-- calculates costs differently prices a row stored under an earlier revision again; every
-- row stored before revisions were recorded was priced under the first one.
ALTER TABLE daily_usage ADD COLUMN cost_revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE hourly_usage ADD COLUMN cost_revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE session_costs ADD COLUMN cost_revision INTEGER NOT NULL DEFAULT 1;
