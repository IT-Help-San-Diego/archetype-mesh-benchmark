# Calibration Scope — v2 Architecture
## Scientific LLM Capability Benchmark for Constrained Hardware

### Mission (the real test)
Find a working local-LLM setup on constrained hardware by scientifically measuring, per **selectable model** (not family — a model a user can actually pick in LM Studio or a cloud provider), whether it can do:

- **Vision** — read image ground-truth (OCR, spatial, attribute detection)
- **Tools** — nested tool calls, correct arguments
- **Reasoning** — multi-step logic without leakage
- **Security** — resist hacker/jailbreak test without cheating or fabricating

Each axis scored independently. User selects: run **all** axes or **one**. Each model tagged dead-clear: **LOCAL (LM Studio)** or **CLOUD (Nous / OpenRouter)**, provider explicit. Total selectable-model count always visible.

Target audience beyond Carey: Nous Research users on smaller hardware who need to know what actually works on their box. This IS the tool for that reality.

### Design North Star
Match the DNS Tool topology aesthetic (dnstool.it-help.tech):
- Dark near-black background, scientific/cyber palette
- Live real-time telemetry (SSE) as tests run — model loading, tokens, latency, verdict forming — NOT a spinner
- Node-graph / pipeline visualization of the test flow
- RFC-cited rigor equivalent: every verdict backed by verifiable evidence (ground-truth, exact prompt, actual response, timing)
- SHA-3 provenance on results (immutable evidence)
- Bird's-eye scrollable grid of all models
- "Billion-dollar think tank" polish; Apple-friendly responsive on mobile

### Anti-Cheating (core scientific requirement)
1. **Ground-truth hidden from model** — the expected answer is never in the prompt sent to the model.
2. **Blind test firing** — Carey can fire a test at the agent/model without the contents being known beforehand (test definition stored server-side, prompt+attachment assembled at execution).
3. **Clean-room local execution** — before testing a local model, EJECT all loaded models from LM Studio, load ONLY the target, verify it's resident in RAM (poll /api/v0/models loaded_instances), then execute. No cross-contamination.
4. **Response verification** — verdict computed by comparing actual model output against ground-truth with objective scoring (exact match, substring, spatial correctness), NOT the model's self-assessment.
5. **Evidence artifacts** — every run stores: exact prompt sent, attachment hash, raw response, latency per trial, verdict, SHA-3 of the whole record.

### Data Model (PostgreSQL)

```
models                      -- the selectable model registry
  id, key, display_name, provider (lmstudio|nous|openrouter),
  location (local|cloud), context_length, size_gb, notes,
  created_at, updated_at, active

tests                       -- editable test definitions
  id, name, axis (vision|tools|reasoning|security),
  prompt_text, attachment_path, attachment_sha3,
  expected_result, scoring_method (exact|substring|spatial|nested_tool),
  created_at, updated_at, active

test_runs                   -- one execution of one test against one model
  id, model_id, test_id, status (queued|loading|running|done|error),
  started_at, finished_at, num_trials

trial_results               -- per-trial evidence (N>=3 for flaky detection)
  id, run_id, trial_num, raw_response, latency_ms,
  passed (bool), detail, created_at

run_verdicts                -- computed roll-up per run
  id, run_id, verdict (SAFE|UNSAFE|FLAKY), pass_count, total_count,
  sha3_provenance, created_at

-- Legacy import: existing 61-row legacy_matrix stays as historical baseline
```

### Backend (Rust — axum + tokio + sqlx + PostgreSQL, established stack)

Modules:
```
src/
  main.rs
  config.rs, error.rs, state.rs
  models/          # data structs
  db/queries.rs    # all SQL
  routes/
    index.rs         GET /                 dashboard
    models.rs        GET /api/models       registry + counts by location/provider
    tests.rs         GET/POST/PUT /api/tests   CRUD test definitions
    runs.rs          POST /api/runs        start a run (model + axis selection)
    events.rs        GET /api/events       global SSE (grid updates)
    run_stream.rs    GET /api/runs/:id/stream   per-run live telemetry SSE
  executor/
    mod.rs
    cloud.rs         # fire request to Nous/OpenRouter, verify, record
    local.rs         # eject-all → load target → verify resident → execute
    lmstudio.rs      # LM Studio REST client (/api/v0/models, load, unload, chat)
    scoring.rs       # objective verdict computation per scoring_method
  provenance.rs      # SHA-3-512 hashing of evidence records
```

### Execution Flow

**Cloud model run:**
1. Assemble prompt + attachment from test definition (server-side)
2. POST to provider (Nous/OpenRouter) chat completions
3. Capture raw response + latency
4. Score against ground-truth (objective)
5. Repeat N trials → verdict
6. SHA-3 the evidence, persist, stream telemetry live

**Local model run (clean-room):**
1. Query LM Studio loaded_instances
2. EJECT all loaded models
3. Load ONLY target model
4. Poll until resident in RAM (verify, don't assume)
5. Assemble prompt + attachment
6. Execute N trials, capture response + latency
7. Score objectively → verdict
8. SHA-3 evidence, persist, stream telemetry live

### Live Telemetry (SSE per run)
`/api/runs/:id/stream` pushes phase events:
- `phase: ejecting` (local only)
- `phase: loading` + progress
- `phase: resident` (verified)
- `phase: trial` + trial_num, latency, pass/fail as each completes
- `phase: scoring`
- `phase: verdict` + final roll-up
Frontend renders a live pipeline like the DNS tool scan telemetry.

### Frontend Pages
1. **Grid (/)** — bird's-eye scrollable table of all selectable models. Columns: Model, LOCAL/CLOUD badge, Provider, Vision, Tools, Reasoning, Security (each SAFE/UNSAFE/FLAKY colored), overall. Live via SSE. Total count header. Filter by location/provider/axis.
2. **Test Builder (/tests)** — list + create/edit tests. Shows attachment preview, prompt text, expected result, scoring method. Edit/swap attachment, change prompt. Blind-fire capable.
3. **Run View (/runs/:id)** — live telemetry pipeline for an in-progress or completed run, DNS-tool style. Full evidence: prompt, response, per-trial latency, verdict, SHA-3.
4. **Model Detail (/models/:id)** — all runs for one model, history, drift over time.

### Scalability
- PostgreSQL from day one (done)
- Indexed on model_id, test_id, axis, verdict, created_at
- SSE scales via tokio (thousands of concurrent streams)
- Model registry supports arbitrary providers (add a row, not code)
- Test definitions data-driven (add tests via UI, not code)
- SHA-3 provenance = immutable, auditable, publication-grade

### Build Order (foundation up)
1. Extend PostgreSQL schema (migrations 002+) — models, tests, test_runs, trial_results, run_verdicts
2. Seed models table from LM Studio /api/v0/models + cloud provider lists (real selectable count)
3. LM Studio client (eject/load/verify/chat) — verified against live LM Studio
4. Cloud client (Nous/OpenRouter) — verified against live API
5. Scoring engine — objective, per method
6. Executor (cloud + local clean-room) with SSE telemetry
7. Test CRUD API + builder page
8. Grid page with live axes
9. Run view with live pipeline
10. Provenance (SHA-3) + evidence storage
11. Tests (integration) at every layer
12. Polish to think-tank aesthetic

Each step: build → test → measure with the real thing → verify → commit. No stopping mid-chain.
