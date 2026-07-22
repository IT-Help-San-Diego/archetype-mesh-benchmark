# LM Studio GUI — "Loaded Models" Load Tab Shows DEFAULTS, Not the Active Config

**Finding (verified 2026-07-21):** the Local Server "Loaded Models" window's
right-side "Load" tab shows the model's **persistent per-model default config**
(from `~/.lmstudio/.internal/user-concrete-model-default-config/<publisher>/<model>.json`),
NOT the API-load-time config that a program (like this benchmark) actually applied.

## What this means in practice

When Calibration Scope runs a model with the **performance** preset
(`context_length: 131072`, `eval_batch_size: 4096`, `parallel: 4`), the
benchmark's `POST /api/v1/models/load` sends those values, and they ARE what the
run uses (recorded in `lmstudio_runtime_config`). But LM Studio's GUI "Load" tab
still shows the model's SAVED default — e.g. nemotron-3-nano-omni's default is
`contextLength: 65536` (65k). So a user watching the GUI sees "65k context"
while the benchmark is actually running at 131072.

**The GUI shows the default, not the run's config.** The run's real config is
only in the benchmark's own provenance (`lmstudio_runtime_config` in the DB).

## Why (the mechanism)

LM Studio has TWO config sources:
1. **Persistent per-model defaults** — the JSON config files + the "My Models"
   gear settings. The GUI "Load" tab reads THESE. Verified: nemotron-3-nano-omni's
   default file contains `contextLength: 65536`.
2. **API-load-time config** — what `POST /api/v1/models/load` accepts. This
   OVERRIDES the default for the loaded instance, but the GUI does NOT reflect
   the override.

The official docs confirm per-model defaults "will be used when the model is
loaded anywhere in the app (including through lms load)" — but the API load
endpoint's parameters take precedence for that instance, and the GUI does not
show the precedence.

## Community reports (same class of issue)

- r/LocalLLM: "LMStudio context length setting keeps resetting to 2048… if you
  change the context length per model, does it not persist?" — users confused
  about which context value is actually active.
- lmstudio-ai/lms#111: "does not load a model if context size is too big" —
  the load-time context vs. the default context confusion.
- LM Studio feature request #156 (open since Dec 2024): no API field reporting
  the model's on-disk size / active config — the registry query
  (`/api/v1/models`) does not expose the API-loaded instance's applied config.

## Honest takeaway for users

If you are running Calibration Scope and watching LM Studio's "Loaded Models"
window: **the Load tab shows the model's saved default, not what the benchmark
is actually using.** To see the run's real config, check the benchmark's own
record — the run's `lmstudio_runtime_config` (surfaced in the run detail view),
not the LM Studio GUI. This is an LM Studio display limitation, not a benchmark
error. We keep testing to confirm whether a future LM Studio version surfaces
the API-applied config in the GUI.
