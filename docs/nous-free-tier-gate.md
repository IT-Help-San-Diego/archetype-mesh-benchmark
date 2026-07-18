# Nous Portal Free-Tier Gate — Verified Finding

**Date:** 2026-07-18
**Status:** Definitive (verified via live API calls + Hermes agent source)

## Summary

Nous Research's **free models** (e.g. `tencent/hy3:free`) are **not reachable
via a raw `NOUS_API_KEY`** pasted into the benchmark's secrets store
(`~/.archetype-mesh/cloud-keys.json`). They require a **Nous Portal OAuth
identity** that only exists when authenticated through the Portal (the path
the Hermes agent uses via `~/.hermes/auth.json`).

This is a Nous limitation, not a benchmark bug.

## Evidence

1. **Live API call with raw `NOUS_API_KEY`:**
   `POST https://inference-api.nousresearch.com/v1/chat/completions`
   with `model: tencent/hy3:free` →
   `400 "missing tags"` → with `tags: [...]` →
   `400 "missing user tag"` → with `user: "..."` →
   `400 "We couldn't process that at all"`.

   Arbitrary `user`/`tags` string values are rejected. Nous expects a
   recognized identity, which a raw API key cannot supply.

2. **Hermes agent source (`agent/portal_tags.py`):** the Hermes agent sends
   `tags: ["product=hermes-agent", "client=hermes-client-vX.Y.Z"]` — but it
   authenticates via **Portal OAuth** (JWT minted from `~/.hermes/auth.json`),
   which injects the authenticated user identity. The raw API key path has no
   such identity, so free models reject it.

3. **Paid Nous models work with raw key:** `tencent/hy3` (paid) returns
   `"requires available credits"` — a billing error, not an auth/tag error.
   So the raw key authenticates fine; only the *free* tier's tag gate blocks it.

## Implication for the benchmark

- A user pasting a `NOUS_API_KEY` via the setup page (external to Hermes
  Desktop) **cannot** benchmark Nous free models. They CAN benchmark paid
  Nous models (if they add credits).
- To benchmark Nous free models, the benchmark would need to authenticate via
  Nous Portal OAuth (mint JWT from a Portal credential) — a much larger
  integration, not a `tags`/`user` field addition.
- **Gemini 3.5 Flash** (Google AI Studio free tier, no OAuth, no user-tag
  gate) remains the only free cloud model the benchmark can run today.

## What "free isn't free" means here

Nous advertises free models, but they are gated behind Portal OAuth identity
rather than a standalone API key. A user with only an API key (the benchmark's
designed external secret path) cannot reach them. This is the reality we
established: free cloud vision/testing via Nous requires the Portal account,
not just a key.
