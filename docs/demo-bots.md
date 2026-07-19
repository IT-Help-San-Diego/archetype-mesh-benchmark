# Demo Bots Panel — Design (foundation feature, part a)

## The Goldilocks directive (user, 2026-07-19)
"AI locally comes to life when it finds its Goldilocks zone around the
architectural hardware and its limitations." The Demo Bots panel is the
instrument to FIND that zone: start at the lightest model that can run a
real test, prove it passes (or show where it fails), scale up until a
model is genuinely usable. Smallest-first, experiment upward.

## Catalog-search reality (verified, do not re-litigate)
- LM Studio REST API has NO catalog-search/browse endpoint. You cannot
  query "models under 2GB with vision" from the API. (404 on list variants.)
- Hugging Face model API does NOT reliably expose GGUF file sizes in
  model metadata (siblings[].size empty for GGUF repos). Per-file HEAD
  requests work but are slow/rate-limited.
- CONSEQUENCE: the manifest is curated from OUR OWN verified benchmark
  leaderboard (real pass/fail per axis + measured size_gb), NOT from a
  live catalog query. This is the honest, data-correct source.

## What LM Studio DOES expose per model (post-download, live-verified)
type (llm/embedding/vlm), capabilities {vision:bool, trained_for_tool_use:bool},
max_context_length, quantization {name,bits_per_weight}, publisher.
Size is NOT in tags — only download/status gives bytes (our downloads).

## Manifest proposal (smallest-first, from verified local leaderboard)
All three are pullable in minutes; all demonstrate a real axis.

1. BOT A — "how small can we go and still run the test"
   qwen2.5-1.5b-instruct (1 GB, verified downloaded, size_gb=1 written
   by our pipeline). Smallest local model we have. Will likely score low
   on reasoning (1.5B) — that's the POINT: shows the user the floor and
   that the instrument works end-to-end. Probe, not a recommendation.

2. BOT B — "scaffold heals a failure" (smallest verified scaffold story)
   ibm/granite-3.2-8b (8B). Raw 45/90 → scaffolded 63/90 on logic axis.
   Demonstrates the core science: a model that fails raw logic but passes
   when given the generalized scaffold. No answer-leakage.

3. BOT C — "vision + speed" (smallest verified-vision local bot)
   qwen/qwen3-vl-8b (8B, vision 12/12 verified). Or the Nemotron 30B+4B
   spec-decode pair for the speed demo (heavier). For lightweight install,
   qwen3-vl-8b is the smallest verified-vision local bot we have.

NOTE: No tiny model passes all 4 axes. Verified 4/4 local = Gemma 4 31B
(31GB) + Nemotron 30B+4B spec pair. The panel's job is to show the
progression 1GB→8GB→(optional 30GB) so the user FINDS their Goldilocks
zone, not to fake a tiny 4/4.

## "Already installed" handling (user concern: they may already have some)
Each Demo Bot card checks the LIVE registry (models table), not just our
download tracker:
- If key present in models (user downloaded it themselves, or prior
  session): show "✓ Installed · {size_gb}" — NO download button. Real
  size_gb from existing row.
- If absent: show "Download" button → POST /api/lmstudio/download.
- While downloading (our job_id active): show live "⏳ 73% · 4.2/5.7 GB"
  or "⏸ paused" from SSE model_download_progress.
- Completed: card flips to "✓ Installed · {size_gb}", size_gb written by
  poller (verified end-to-end, commit 0804ab2).

## Pause/cancel (API gap, honest)
No in-tool pause/cancel (LM Studio REST has no such endpoint — 404/415
on probes). Card notes: "Pause/resume in your LM Studio downloader."
Verified: pausing in LM Studio reflects as status:paused in our SSE.

## UI placement (deferred — user's focused-mode/left-column design)
The panel is a section in the left column / onboarding area. Exact slot
is part of the broader focused-mode design the user deferred. This doc
locks the DATA + STATES; the visual slot is a follow-up decision.

## Lightweight guarantee
Panel reads existing SSE registry snapshot + downloadProgress map. No new
polling. Backend poller is zero-cost when idle (verified).
