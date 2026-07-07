-- v008: Add flagship models so the demographic sees themselves immediately.
-- Carey's directive: "flip and do the LLAMA and CLAW [Claude], just the most
-- important ones, so people can be kind of compatible."
--   - hermes-3-llama-3.1-8b: local, MLX, Nous's own Hermes-tuned Llama — the
--     bridge model for people coming from LM Studio's default catalog.
--   - anthropic/claude-sonnet-5: cloud via OpenRouter — the model most Hermes
--     Agent users have literally open in another tab right now.
INSERT INTO models (key, display_name, provider, location, context_length, size_gb, notes, tags, active)
VALUES
  ('hermes-3-llama-3.1-8b', 'Hermes-3-Llama-3.1-8B (Local)', 'lmstudio', 'local', 131072, 4.5,
   'NousResearch Hermes-tuned Llama 3.1, MLX 4-bit. The most recognizable local model name for anyone coming from Nous.',
   ARRAY['flagship','llama','hermes'], true),
  ('anthropic/claude-sonnet-5', 'Claude Sonnet 5 (Cloud · OpenRouter)', 'openrouter', 'cloud', 1000000, 0,
   'The model most Hermes Agent users already run as their main brain. Included so cloud results are directly comparable to what you see day to day.',
   ARRAY['flagship','claude','cloud'], true)
ON CONFLICT (key) DO NOTHING;
