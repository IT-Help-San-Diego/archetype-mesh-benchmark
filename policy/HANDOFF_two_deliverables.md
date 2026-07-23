# HANDOFF — two ready-to-run deliverables for Hermes / Claude Code
_From: Claude Science, 2026-07-23. Both prepared locally (no external compute needed to WRITE them);_
_each needs its compute target to EXECUTE. Commit all files to the repo, then hand to the executor bot._

## Deliverable 1 — Carrier Color analysis harness  (VALIDATED, ready)
**File:** `carrier_color_analysis.py`  (commit under `analysis/` or `scripts/`)
**Status:** DONE and self-tested. It ships with an embedded validation suite proving:
  - it DETECTS a real 3–8pt paired carrier effect (baseline vs Lean p_holm=0.003), and
  - it reports NULL on a flat/immune model (0 false-positive pairs).
Run the self-test any time: `python carrier_color_analysis.py --self-test` → ALL PASS.

**What Hermes does:**
1. Run the paired experiment per `carrier_color_experiment_spec_v1.md` (same ≥500-item
   logic set under all 5 carriers, 4 models spanning the immunity band, carrier order
   randomized per item, clean infra + token-ceiling log + SHA seal).
2. Emit the trial-level CSV with columns EXACTLY:
   `model,item_id,carrier,trial,pass,tokens_prompt,tokens_completion,seed`
3. Analyze in one command: `python carrier_color_analysis.py results.csv --out analysis.json`
4. The harness returns per-model McNemar matrices (Holm-corrected) + the carrier×capability
   interaction. NOTE baked into the output: the interaction is UNDERPOWERED with <4 models —
   that is why the spec asks for 4+. Do not over-read a null interaction.
5. Then hand `results.csv` + `analysis.json` back to Claude Science for the §10.8-v1 rewrite
   (confirm or honestly retract the ordering, whichever the data says).

## Deliverable 2 — Rust root task modification  (BUILD-GATED, needs the box)
**File:** `example_main.rs`  → replaces `crates/example/src/main.rs` in a checkout of
`seL4/rust-root-task-demo` on the EC2 builder. (Committing to calibration-scope as a
staged patch is fine; it is NOT upstream's file.)
**What it does:** demonstrates seL4 capability CONFINEMENT — mints TWO badged capabilities
(A=0x1337 preserved, B=0x5C09E "SCOPE") from ONE notification and verifies each delivery
carries its own sender identity and neither can forge the other's. This is the kernel-level
analogue of the Genie/harness thesis: a confined capability can only do what its badge
authorizes. The literal `TEST_PASS` marker is PRESERVED so test.py still gates the boot.

**!!! HONESTY GATE — this has NOT been compiled or booted (box was stopped when written).**
It is written against the exact rust-sel4 API in upstream `main.rs`, but until it BUILDS
clean AND boots to TEST_PASS under QEMU on the box, it is UNVERIFIED. Do not treat a green
as given.

**What Hermes/Claude Science does (box must be RUNNING):**
1. `touch /tmp/PROOF_RUNNING`  ← sentinel so the idle watchdog won't stop the box mid-build (§13e).
2. In the demo checkout: replace `crates/example/src/main.rs` with `example_main.rs`.
3. Build + boot via the endorsed Docker flow (DECISIONS §6):
   `sudo docker run --rm --mount type=bind,src=$repo,dst=/work rust-root-task-demo make test`
4. Expect serial: `sender A badge = 0x1337` / `sender B badge = 0x5c09e` /
   `capabilities confined ...` / `TEST_PASS`.
5. If green: evict the boot receipt to `evidence/sel4/capability-confinement/` (MANIFEST +
   boot.log + image.elf, sha256+sha3-256), same pattern as the first bundle.
6. `rm /tmp/PROOF_RUNNING` when done.
7. If it does NOT build/boot: that is expected-possible (build-gated). Capture the compiler
   error, hand it back to Claude Science to fix the API usage — do NOT force it green.

## Epistemic log — REQUIRED for both (per EPISTEMIC_LOG_POLICY.md)
Append one line to `EPISTEMIC_LOG.jsonl` per run:
  - Carrier Color paired run → action `rerun`, target the CSV, `supersedes` the N=102 §10.8 data,
    sha256 of the CSV, receipt = the SHA-seal path.
  - Rust boot → action `verify`, target `example_main.rs`, sha256 of image.elf, receipt =
    the evidence/sel4/capability-confinement path. (Only log `verify` if it ACTUALLY booted green.)

## Files in this drop
  analysis:  carrier_color_analysis.py         (validated harness)
  spec:      carrier_color_experiment_spec_v1.md (already committed — the run design)
  rust:      example_main.rs                    (build-gated patch)
  policy:    EPISTEMIC_LOG_POLICY.md            (integrity policy — all agents)
  log:       EPISTEMIC_LOG.jsonl                (seeded; append-only)
