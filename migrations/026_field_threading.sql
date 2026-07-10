-- v026: Field threading — carry every fact the provider states, end to end.
--
-- Audit findings (2026-07-09, scripted against live /api/models + LM Studio):
--   * 251/281 cards HID their context_length behind a `size_gb ?` render guard
--   * 244 cloud display_names had "(Cloud · provider)" baked in at sync time —
--     redundant 3x with the CLOUD badge and the provider tag row
--   * LM Studio states quantization / arch / publisher per model; none were
--     threaded into the registry at all
--
-- Scalability contract: a field the provider states is stored named, typed,
-- and rendered from the same row every layer reads. No display strings
-- manufactured at sync time — presentation belongs to the presentation layer.

ALTER TABLE models ADD COLUMN IF NOT EXISTS quantization TEXT;
ALTER TABLE models ADD COLUMN IF NOT EXISTS arch TEXT;
ALTER TABLE models ADD COLUMN IF NOT EXISTS publisher TEXT;

-- Strip the sync-manufactured "(Cloud · provider)" suffix from existing
-- cloud rows; display composition is the frontend's job now.
UPDATE models
SET display_name = regexp_replace(display_name, '\s*\(Cloud · [^)]+\)$', '')
WHERE location = 'cloud'
  AND display_name ~ '\(Cloud · [^)]+\)$';
