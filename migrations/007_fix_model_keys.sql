-- v007: Registry keys must be the EXACT provider-native selectable id.
-- The executor addresses LM Studio by these keys; a synthetic 'local-' prefix
-- would break load/eject/chat calls. Verified against /api/v0/models 2026-07-07.
UPDATE models SET key = 'qwen2.5-vl-7b-instruct'        WHERE key = 'local-qwen2.5-vl-7b-instruct';
UPDATE models SET key = 'qwen2.5-coder-7b-instruct-mlx' WHERE key = 'local-qwen2.5-coder-7b-instruct-mlx';
UPDATE models SET key = 'llama-3.2-3b-instruct'         WHERE key = 'local-llama-3.2-3b-instruct';
UPDATE models SET key = 'ibm/granite-3.2-8b'            WHERE key = 'local-ibm-granite-3.2-8b';
