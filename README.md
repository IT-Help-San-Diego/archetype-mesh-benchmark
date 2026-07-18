# Archetype Mesh Benchmark

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

**Test your local AI models like a scientist — on your own hardware, with evidence you can audit.**

A local-first benchmarking dashboard for [LM Studio](https://lmstudio.ai) and cloud LLMs that measures what models can *actually* do — vision, tool use, reasoning, prompt-injection resistance — using ground-truth tests, N=3 trials, SHA3-sealed evidence, and zero trust in anyone's marketing numbers. Built in Rust (Axum + Tokio + SQLx + PostgreSQL) with a single-file live dashboard driven by Server-Sent Events.

Made by [IT Help San Diego Inc.](https://www.it-help.tech/) · A project of the [Intellectual Resistance](https://intellectualresistance.com/)

## Status

- **In active development** — scientific validation is the main bottleneck, not features.
- **Hermes-ready**: integrated in Hermes Desktop as of July 2026. Model routing, cloud key setup, and local clean-room execution are verified live.
- **Standalone-capable**: runs on any macOS/Linux box with Rust + PostgreSQL + LM Studio. No Hermes dependency.
- **Modular architecture**: backend exposes REST + SSE; frontend is a single static HTML file. MCP server layer is planned so external tools (OpenClaw, bots, scripts) can drive benchmarks programmatically.
- **Public beta**: the core pipeline works (clean-room, blind tests, SHA-3 seals, speculative decoding measurement). The science — competitive cross-reference, fallacy taxonomy expansion, contamination resistance — is being validated now.

---

## Why this exists

Standardized benchmarks (MMLU and friends) are public — models have trained on them, so the numbers can't be trusted alone. Meanwhile, people running local AI on their own machines have no honest way to answer basic questions:

- *Which of my models can actually read text in a screenshot — and which one **fabricates plausible-sounding text** when it can't?*
- *Which model should sit in each job slot (vision, tool routing, command approval) based on **my measured evidence**, not parameter-count intuition?*
- *Is a "failure" a real capability gap, or my infrastructure lying to me?*

This tool answers those questions with the discipline of a lab notebook:

| Principle | Implementation |
|---|---|
| **No answer leakage** | Ground truth lives only in the database; the model is never shown what it's scored against. The test builder rejects prompts that contain their own answer. |
| **N=3 trials, always** | One pass can be luck. Capability axes score PASS / FAIL / FLAKY; security scores SAFE / UNSAFE / FLAKY. |
| **Objective scoring** | Verdicts come from comparing output to ground truth (exact match, substring, regex, spatial relations, tool-call shape) — never from asking a model its opinion. |
| **Clean-room runs** | Before each local run, every loaded model is ejected, only the target is loaded, and RAM residency is verified by polling — never assumed. |
| **Sealed evidence** | Every trial stores the exact prompt, raw response, reasoning trace, and latency. Every run is sealed with a SHA3-512 provenance hash. Test images are SHA3-256-pinned so the stimulus can't drift. |
| **Infra errors ≠ capability failures** | A config bug that blocks requests is recorded as infrastructure noise, excluded from the capability score — a model is never blamed for your network. |
| **Full citation graph** | Every trial row links to its exact test (prompt + pinned image + ground truth) and its run seal. Query the evidence in both directions. |

## What it caught in its first week (real examples)

- A 12B vision model **confidently fabricated an entire sidebar** of a screenshot it couldn't read — invented plausible note titles, zero of which existed. The token receipt (whole image compressed to ~256 vision tokens) explained *why*: the text was physically unreadable at that budget, and the model invented rather than admitting it.
- A 2B model **failed the same test deterministically** — same wrong answer, three trials in a row (it read a menu-bar *icon* and reported the wrong app). Four other models read the same pixels correctly, 3/3 each. That one screenshot is now a permanent regression test.
- The leaderboard formula itself was caught ranking a text-only coder model #1 overall *despite a 100% hard-fail on vision* — because the old score only counted wins. Fixed, regression-tested, documented in the commit history.
- An empty model response (reasoning budget exhausted, `finish_reason: length`) was being **rendered as if it were an answer**. Now it's a loud failure banner: "NO FINAL ANSWER — this is a failure, not a result."

The commit history is deliberately forensic — most fixes cite the live incident that motivated them.

## Features

- **🏁 Benchmark grid** — every model in your LM Studio library plus configured cloud models (Nous, OpenRouter), per-axis verdicts with latency, live SSE telemetry (`ejecting → loading → resident → trial → verdict`), real timestamps, no spinners anywhere.
- **🏆 Loot page** — leaderboard + "recommended squad" (best verified model per job slot), and a **capability router** that assigns primary/fallback models per axis from lifetime evidence, with stated reasons and evidence links (`/api/router/plan`).
- **🧪 Prompt Builder** — side-by-side workbench: compose (text + image) on the left, results on the right. Reasoning traces shown separately from committed answers. Persistent run history — every test you run is kept, queryable, revisitable. Prompt-length checker with instant heuristic + optional exact token count.
- **📋 Test Registry** — blind by default (ground truth requires an explicit audit view), custom test builder with anti-leakage validation, viewable SHA3-pinned image attachments.
- **🖥️ Reality check** — the setup page *measures your machine* (RAM via `sysctl`, GPU ceiling via Metal's `recommendedMaxWorkingSetSize` — a documented API, not folklore, live memory pressure, LM Studio state) and computes an honest AI RAM budget with the formula shown. Every number carries the command it came from.
- **⚙️ Hermes-aware** — if you run [Hermes Agent](https://hermes-agent.nousresearch.com), the setup page reads your actual config (allowlisted fields only — never credentials) and shows verified ✅ state for main model and auxiliary task slots.

## Using this with Hermes Agent / Hermes Desktop

This dashboard pairs naturally with [Hermes Agent](https://hermes-agent.nousresearch.com) (Nous Research's open agent runtime) — it was built alongside a live Hermes deployment:

- **Route by evidence, not vibes:** run the benchmark battery, then open `GET /api/router/plan` — it assigns a verified primary + fallbacks per capability axis. Map those onto Hermes' **Settings → Model Settings → Auxiliary Tasks** slots (vision, MCP tool routing, approval classification, web extract). The Setup tab shows the exact mapping.
- **The "Approval" slot is your security surface:** Hermes' smart auto-approve sends shell commands to a model for APPROVE/DENY/ESCALATE judgment. This dashboard's security axis measures exactly that job — prompt-injection resistance with real injection payloads — so you can pin a *proven* local model there instead of guessing.
- **Config verification, read-only:** the Setup tab reads `~/.hermes/config.yaml` through a strict allowlist (never credentials) and shows live ✅/⚠️ against what Hermes is actually configured to do.

Hermes is not required — the dashboard works standalone with LM Studio and/or any OpenAI-compatible cloud endpoint.

## Stack

- **Backend:** Rust — Axum 0.8, Tokio, SQLx 0.9, PostgreSQL, reqwest, SHA-3. One static binary.
- **Frontend:** one HTML file, zero frameworks, zero build step. SSE for live updates.
- **Evidence store:** PostgreSQL (inspectable with any SQL client — the schema *is* the API).
- **Model I/O:** LM Studio REST (`/api/v0`), OpenAI-compatible cloud endpoints.

No telemetry. No external calls except the model endpoints you configure. Binds to `127.0.0.1` only.

## Quick start

```bash
# Prereqs: Rust toolchain, PostgreSQL, LM Studio with its local server on :1234
git clone https://github.com/IT-Help-San-Diego/archetype-mesh-benchmark.git
cd archetype-mesh-benchmark
cp .env.example .env          # set DATABASE_URL (and optional cloud API keys)
cargo run --release           # migrations run automatically
# open http://127.0.0.1:8768
```

Sync your LM Studio library from the dashboard (**LM Studio → Sync**), pick a model, click **▶ Run** — and watch the live log. Verdicts land on the grid with latency; evidence lands in Postgres with a seal.

## Operations

This repo includes a launchd-managed backend on macOS. Use these commands instead of ad-hoc `cargo run` processes.

### Service
Name: `ai.hermes.archetype-mesh-dashboard`
Binary: `~/Documents/GitHub/archetype-mesh-benchmark/target/release/archetype-mesh-dashboard`
Port: `8768` on `127.0.0.1`
Database: `postgres://<dbuser>:<dbpass>@localhost:5432/archetype_mesh`
Logs: `/tmp/archetype-mesh-dashboard.out`, `/tmp/archetype-mesh-dashboard.err`

### Start / stop / restart
```bash
launchctl start ai.hermes.archetype-mesh-dashboard
launchctl stop ai.hermes.archetype-mesh-dashboard
launchctl kickstart -k gui/$(id -u)/ai.hermes.archetype-mesh-dashboard
```

### Health
```bash
curl http://127.0.0.1:8768/api/status
```

### Run control from the backend API
```bash
# start a run
curl -X POST http://127.0.0.1:8768/api/runs \
  -H 'content-type: application/json' \
  -d '{"model_key":"google/gemma-4-31b-qat","axes":["vision","reasoning"],"load_mode":"clean-room"}'

# list runs
curl http://127.0.0.1:8768/api/runs

# run detail
curl http://127.0.0.1:8768/api/runs/628

# abort an in-flight run
curl -X POST http://127.0.0.1:8768/api/runs/628/abort
```

### Database access
Preferred: **TablePlus** connection `127.0.0.1:5432` → database `archetype_mesh` → your configured PostgreSQL user. The schema *is* the API: every trial row links to its exact test and run seal.

### Troubleshooting
- If the binary was rebuilt, use `launchctl kickstart -k` instead of `launchctl start` so launchd loads the new executable.
- If runs show `status = error` but trials exist, the executor preserved partial evidence; the database still contains complete trial results for post-mortem analysis.
- Quarantined runs are excluded from leaderboard/router scoring by default; review them via `/api/quarantine` when needed.

## Philosophy

This project believes the flood of AI-generated junk science gets fixed by **making rigorous method cheap**, not by gatekeeping. Everyone with a laptop and curiosity can run a controlled experiment: pinned stimulus, committed answers, N=3, sealed results. The dashboard is deliberately a teacher — it explains its formulas, shows its receipts, and marks its heuristics as heuristics.

If a number on the screen can't cite where it came from, that's a bug. File it.

## License & attribution

**Archetype Mesh Benchmark is licensed under the [Apache License, Version 2.0](LICENSE).**

- Copyright © 2026 **IT Help San Diego Inc.** All rights reserved under the terms of the Apache-2.0 license.
- Research published under Carey James Balboa and IT Help San Diego Inc., as part of the [Intellectual Resistance](https://intellectualresistance.com/) program.
- The benchmark methodology, test battery, scoring logic, and SHA3-provenance design are original works of IT Help San Diego Inc.
- Patent grant included (Section 3 of Apache-2.0): contributors grant a perpetual, royalty-free patent license for their contributions.
- Trademark: "Archetype Mesh", "IT Help San Diego", and the Owl of Athena mark are trademarks of IT Help San Diego Inc. The license does not grant rights to use these marks except for reasonable attribution.
- See [NOTICE](NOTICE) for attribution and trademark details.

This is a permissive license: you may use, modify, and redistribute the work (including commercial use), provided you retain the license, note modifications, and preserve attribution. It is intentionally more permissive than the DNS Tool product (BUSL-1.1) because a benchmark's value depends on broad, independent adoption — the methodology is the asset, not a hosted service.

