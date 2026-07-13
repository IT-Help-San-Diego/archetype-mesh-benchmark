# Speculative Decoding Pair Inventory

## Verified Working Pairs
- `google/gemma-4-31b` + `google/gemma-4-12b` — draft stats confirmed
- `google/gemma-4-31b-qat` + `google/gemma-4-12b-qat` — loaded successfully
- `ibm/granite-3.2-8b` + `ibm/granite-3.1-8b` — loaded successfully
- `nvidia/nemotron-3-nano-omni` + `nvidia/nemotron-3-nano-4b` — loaded successfully, draft stats confirmed

## Verified Standalone Models (not yet paired)
- `stepfun-ai_step-3.5-flash` (Bartoski GGUF) — loads successfully, 85.97 GB resident, inference 200 OK

## Confirmed Blockers / Not Verified
- `hermes-4.3-36b` + `harmonic-hermes-9b@q8_0` — insufficient system resources guardrail (~52.93 GB estimate)
- `ibm/granite-3.3-8b-instruct` + `ibm/granite-3.2-8b` — model_not_found in LM Studio registry
- Step Fun pair binding: memory guardrail blocks load for some Step Fun combinations
- Qwen load-time draft binding: engine protocol mismatch
- MTP path: Step Fun MTP files not loadable as standalone models

## Experimental State
- LM Studio Backup: `~/Downloads/lmstudio-backup-20260713/`
- Contains: draft-model-compatibility-cache.json, settings.json, user-concrete-model-default-config/
- Current LM Studio residents: `stepfun-ai_step-3.5-flash`, `ibm/granite-3.2-8b`

## Notes
- LM Studio supports multiple simultaneous loaded instances on this host.
- **LM Studio unload API is unreliable.** Use `lms unload -a` as the authoritative cleanup path, then verify with `lms ps`.
- Exhaustive same-family GGUF pair sweep completed on downloaded inventory.
- Step Fun Bartoski 3.5 Flash is the first verified standalone Step Fun model on this host.

## Tested At
2026-07-13
