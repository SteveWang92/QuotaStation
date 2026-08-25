-- When the soonest available Codex reset credit stops being redeemable. Null where the
-- provider grants no credits, or grants them without publishing an expiry.
ALTER TABLE provider_instances ADD COLUMN earned_reset_expires_at INTEGER;
