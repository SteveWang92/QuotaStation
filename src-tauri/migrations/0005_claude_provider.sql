-- Claude Code joins Codex as a monitored provider. Every table is already keyed on
-- provider_instance_id, so registering the instance is the whole change.
INSERT INTO provider_instances (provider, display_name, parser_revision)
VALUES ('claude', 'Claude Code', '033c1f7631f603fc939fdc85163e8203f0084f83')
ON CONFLICT(provider) DO NOTHING;
