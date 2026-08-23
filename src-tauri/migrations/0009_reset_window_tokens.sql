-- The tokens a window carried are summed from the hourly rows it spans, and those are kept
-- for a fortnight while the reset events themselves are kept indefinitely. Summing on every
-- read therefore forgot the total the moment its hours aged out. It is stored on the event
-- instead, recomputed on every write for as long as the hours behind it are still there and
-- frozen once they are gone. Existing rows are filled by the first refresh after this
-- migration rather than here, because the retention window the fill is bounded by is a Rust
-- constant and a second copy of it in SQL would drift.
ALTER TABLE limit_resets ADD COLUMN tokens_in_window INTEGER;
