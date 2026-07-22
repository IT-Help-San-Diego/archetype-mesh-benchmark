# Calibration Scope MCP Server — Design (foundation)

**Goal:** a meaningful Model Context Protocol (MCP) server so a user's bot can
connect to Calibration Scope and *tell it to do stuff* — run benchmarks, read
verdicts, abort runs, pull the leaderboard — programmatically, with honest data
and verifiable tools.

**Lessons learned from LM Studio's API (the anti-patterns we will NOT repeat):**

| LM Studio smell | The lesson for OUR MCP |
|---|---|
| No "list all downloads" endpoint (can't see what exists) | **Complete, discoverable tool surface.** `tools/list` returns EVERY tool with a JSON Schema for its args. A client never guesses. |
| Cancel/pause unverified (undocumented endpoint) | **Every tool is documented + verifiable.** If a tool exists in `tools/list`, it works. No "maybe it works" endpoints. |
| No size field in model listing (incomplete data) | **Honest, complete data.** Tools return the real `size_gb` we capture, the real verdicts, the real run state — `—` when unknown, never fabricated. |
| Download returns job_id immediately (good design) | **Useful handles immediately.** `run_benchmark` returns `run_id` instantly; the client polls `get_run` to track. No opaque fire-and-forget. |
| Hidden state (ghost models, no clarity) | **Explicit state.** `get_run` returns the full state machine (queued/running/done/error/aborted) + pass/total + quarantine reason. No hidden ghosts. |

**Transport:** JSON-RPC 2.0 over HTTP POST at `/mcp` (the MCP "streamable HTTP"
pattern). Single endpoint, `Content-Type: application/json`. Methods:
`initialize`, `tools/list`, `tools/call`, `ping`. (Resources + prompts can be
added later; tools are the meaningful surface for "tell it to do stuff".)

**The tool surface (v1 — the meaningful set):**

| Tool | Args | Returns | What it does |
|---|---|---|---|
| `get_status` | — | health, db, running_runs, uptime | Dashboard health + state |
| `list_models` | `location?`, `provider?`, `runnable?` | models[] with verdicts, size_gb, vision, runnable | The registry (honest data) |
| `get_model_verdict` | `model_key` | 4-axis verdict + score + size_gb | One model's verified verdict |
| `run_benchmark` | `model_key`, `axes?`/`test_ids?`, `load_preset?`, `provider?` | `run_id`, `run_ids[]` | Fire a benchmark (clean-room) |
| `get_run` | `run_id` | status, pass_count, total_count, verdict, quarantine | Poll a run's state |
| `abort_run` | `run_id` | `aborted: true` | Stop a live run |
| `get_leaderboard` | `axis?` | ranked models with clean post-fix scores | The verified leaderboard |
| `get_carrier_color` | — | the 5-arm spectrum + immunity threshold | The published Carrier Color findings |
| `get_owl_state` | — | I/N/C/M coverage counts | Owl Semaphore V4 state |
| `get_test_spec` | `test_id` | name, axis, formal_spec, expected_result | A test's Lean formal spec |
| `list_tests` | `axis?`, `active?` | tests[] with formal_spec, owl_type | The test registry |

**Design principles (foundation for scalability):**
1. **Thin wrappers, not duplicate logic.** Each MCP tool maps to the existing
   handler/DB query (runs.rs, models.rs, loot.rs). The MCP server does NOT
   reimplement the benchmark logic — it calls the same code the REST API uses.
2. **Every tool has a JSON Schema** for its args (in `tools/list`). A client can
   validate before calling. No undocumented args.
3. **Honest errors.** A tool that fails returns a JSON-RPC error with a real
   message (not a silent null). Mirrors the "honest data" rule.
4. **No state hiding.** Every tool's return is complete and auditable. The MCP
   server is a transparent window into the same state the dashboard shows.
5. **Scalable:** new tools = new entries in the tool registry + a handler fn.
   The registry is data-driven (a vec of ToolDefs), so adding a tool is a
   one-line registration, not a refactor. **This is the "never say we can't
   rename that field" foundation** — the tool surface is data, not hardcoded
   routes.

**Auth (v1):** local-only (127.0.0.1 bind, same as the dashboard). No auth token
in v1 — the MCP server is a local instrument. If it ever goes beyond localhost,
auth is a foundation-level decision (token + CORS), not a bolt-on.

**Transport note:** MCP officially supports stdio + streamable HTTP. For a local
dashboard, streamable HTTP at `/mcp` is the right choice (bots connect over
localhost HTTP, same as the REST API). stdio is for CLI-spawned servers (not our
case — we're a resident dashboard).
