# Speculative Decoding Pair Inventory

## Verified Working Pairs
- `google/gemma-4-31b` + `google/gemma-4-12b` — draft stats confirmed
- `google/gemma-4-31b-qat` + `google/gemma-4-12b-qat` — loaded successfully
- `ibm/granite-3.2-8b` + `ibm/granite-3.1-8b` — loaded successfully

## Confirmed Blockers
- Step Fun pair binding: memory guardrail blocks load (~104 GB estimate)
- Qwen load-time draft binding: engine protocol mismatch
- MTP path: Step Fun MTP files not loadable as standalone models

## LM Studio Backup
- Path: `~/Downloads/lmstudio-backup-20260713/`
- Contains: draft-model-compatibility-cache.json, settings.json, user-concrete-model-default-config/

## Tested At
2026-07-13
