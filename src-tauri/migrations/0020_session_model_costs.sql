-- What each of a session's models cost and the tokens it was priced for, as JSON. A cost
-- already settled is kept while its model's tokens are unchanged, so a later catalog prices
-- only what an earlier one could not. NULL on a row written before it was recorded; such a
-- row keeps its total while the session's tokens are unchanged.
ALTER TABLE session_costs ADD COLUMN model_costs TEXT;
