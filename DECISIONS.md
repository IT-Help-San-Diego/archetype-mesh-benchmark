# calibration-scope — DECISIONS.md

**Purpose.** Single source of truth shared across tools that don't share memory
(Claude Code in the terminal, Claude Science in the project, the Hermes agent).
Each tool reads this FIRST and appends decisions here. If it isn't committed to
the repo, neither tool can rely on it — the repo is the shared brain.

_Maintainers: Carey James Balboa (ORCID 0009-0000-5237-9065), IT Help San Diego Inc._
_Last updated: 2026-07-21 — merged from Claude Science's memo (§0–§6) + Claude
Code's repo state (§7–§10). Ethos: **accuracy before speed, validation before
trust** — don't trust a fix, gate it; aspire to seL4-level (proved, not asserted)._

---

## 0. Tool roles & the memory boundary (READ THIS FIRST)

Claude Code and Claude Science are **separate products with separate memory
stores.** They do NOT read each other's minds. The ONLY shared substrate is the
**git repo + filesystem.** Everything below follows from that fact.

| Tool | Owns | Memory |
|---|---|---|
| **Claude Code** (terminal) | The repo: source, commits, tests, the running instrument. Source-of-truth author. | CLAUDE.md + its own session context |
| **Claude Science** (project) | Analysis & figures, open-science packaging, remote compute (seL4 build/boot/proof on AWS). Produces **artifacts**. Does NOT commit to the repo. | This project's artifacts + notes |
| **Hermes agent** | Infra execution (e.g. stood up the EC2 box). Commits straight to `main`. | Its own |
| **Carey (human)** | The router. Relays decisions between tools. Holds the thread. | — |

**Handoff mechanism:** files through git. Claude Science produces an artifact →
Carey or Claude Code commits it into the repo → both tools can then see it.
Claude Code changes the schema/tests → Claude Science reads committed state from
GitHub and re-runs. Whatever is committed is shared; whatever isn't, isn't.

**Why the separation is deliberate (not a limitation):** Claude Code *authors*;
Claude Science *independently builds, validates on clean infrastructure, and
packages for the outside world.* Giving Claude Science commit access would
collapse author and validator into one stage and lose the independent check.
Keep the boundary.

---

## 1. Git workflow

- **Policy: main-direct for a solo project.** Matches how the Hermes agent already
  works. Claude Code commits to `main` directly.
- **Condition (the seL4 discipline applied to our own repo): don't trust a fix —
  gate it.** A data-integrity fix lands on `main` only when *verified, not just
  asserted* (quarantine confirmed, clean data confirmed untouched, guarded by an
  automated verifier).
- **PR #1 `sweep2-foundations` → `main`: MERGED 2026-07-21** (merge `3261a85`).
  It fixed the scaffold-run answer-key leak; the merge gate was satisfied first
  (27 scaffolded runs quarantined; 508 clean-room runs confirmed uncontaminated).
  We then adopted main-direct.
- **Verifier-as-a-gate (#19): DONE.** The logic ground-truth verifier runs in CI
  on every push — `main` fails if a ground truth is wrong. This is what lets
  main-direct stay safe. See §9.

## 2. Claude Science does NOT touch git

Claude Science has no commit access to `calibration-scope` and should not be
given any — see §0. Its outputs are **artifacts** handed to Carey/Claude Code to
commit. The only `git clone` it runs is the *public* `seL4/rust-root-task-demo`
onto the EC2 build box — a throwaway build tree, unrelated to this repo.

## 3. Open-science direction (Claude Science memo)

Ship the packaging discipline owl-semaphore already has, and finish the neuro
bridge already stubbed in the repo (`neurovault.rs`). Six moves, by leverage:
1. **Frictionless data package** — `datapackage.json` beside every CSV (typed,
   validated schema). Generated from the real benchmark schema; primaryKey
   `(date, model, test_id)`, verdict enum locked to real values.
2. **DOI + CITATION.cff** — enable Zenodo↔GitHub webhook, tag a release, get a
   concept DOI. `CITATION.cff` written with ORCID wired in. Biggest unlock: a
   scientist can't build on what they can't cite.
3. **Split instrument from app** — publish thin `calibration-scope-py` on PyPI
   (`load_results()` reads the data package; `run_battery()` hits any
   OpenAI-compatible endpoint). Formalize the planned MCP server layer as the
   import/export surface. Keep the dashboard as reference implementation.
4. **Speak neuroscience vocabulary** — `ontology_crosswalk.json` maps each test
   family → cognitive construct → Cognitive Atlas ID → human-task analogue.
   Lets a construct resolve identically for a model verdict and a NeuroVault
   brain map. **Confirm the Atlas IDs before publishing — they are curated stubs.**
5. **Publish methods** — target JOSS (its review is a free rigor checklist) + a
   data descriptor for the benchmark.
6. **Lower contribution barrier** — CONTRIBUTING.md + "add a test family" tutorial.

**What NOT to change:** don't dilute the rigor (objective scoring / N=3 /
clean-room / SHA seals *is* the product); don't couple the science to
Hermes/LM Studio; ship the neuro bridge as explicitly *hypothesis-generating*
with scope limits.

_(Note: the `ingest/` folder is a gitignored idea dumping-ground. Anything
validated and kept must be moved to its proper home — e.g. the logic verifier is
now `scripts/verify_logic_ground_truth.py`.)_

## 4. Compute lifecycle (AWS EC2)

- **Three roles:** (1) dev-time/validation — YES, ephemeral. (2) CI/release-gate —
  YES, the target. (3) runtime/production server — **NO, explicitly rejected.**
- Published tool stays **local-first, no-telemetry, self-hosted** (binds 127.0.0.1).
  No permanent server. Any future "permanent" need is **static storage** (S3 +
  static site), not a running instance.
- **Cost:** idle = disk only (a *stopped* instance bills EBS, ~few $/mo; only
  *running* bills CPU). On-demand pricing is flat 24/7 — time of day doesn't
  change it. **Spot** is the real discount (~60-90% off) — use for the heavy
  proof box. Pattern: stopped-with-fast-start.
- **Terminology:** "powerful model at Amazon" = EC2 **instance type** (vCPU/RAM),
  NOT an LLM. The reasoning model (Claude) runs in the Claude Science interface,
  not on EC2, and is not billed by AWS.
- **Secrets:** only in Customize → Credentials / Compute. **Never in chat.**

## 5. Remote host: ssh:cal-scope-sel4 (VERIFIED 2026-07-21)

- Ubuntu 22.04.5, x86_64, 8 vCPU / 15 GB RAM / ~90 GB free (c7i.2xlarge). User
  `ubuntu` (sudo). scratch_root = `/home/ubuntu/scratch`. Persistent build trees
  in `~/projects`.
- Bootstrapped: Rust 1.97.1 + bare-metal targets (aarch64-unknown-none,
  riscv64imac-unknown-none-elf); QEMU 6.2.0 (aarch64/riscv64/x86); seL4 tooling
  (repo, cmake, ninja, dtc, cross-gcc); Docker 29.1.3; sel4-deps 0.7.0.
- **Posture: stopped-when-idle, start-on-demand.** Isabelle/HOL (l4v) proof is NOT
  here — that's a separate heavy Spot box.

## 6. seL4 + Rust build — VERIFIED GREEN (2026-07-21)

- **Target:** aarch64 under QEMU, raw `rust-sel4` root task. Endorsed reproducible
  Docker flow (pinned): seL4 **v15.0.0** (qemu-arm-virt, HypervisorSupport=ON,
  cortex-a57) + `rust-sel4` rev `7a2321f` + `sel4-kernel-loader`, Rust
  `nightly-2026-03-18`. Demo repo: `seL4/rust-root-task-demo` @ 7dcc192.
- **Result:** kernel booted, dropped to userspace, root task printed
  `Hello, World! badge=0x1337 TEST_PASS`. Validation harness (test.py) asserts
  the `TEST_PASS` serial marker — a real behavioral gate, not "it compiled."
- **Artifacts:** `image.elf` (489 KB, SHA256 b663e54b…), `boot_validation.log`.
- **Build gotcha:** `make BUILD=/work/build` writes image.elf into the repo tree,
  not the job workdir — either build with BUILD=$PWD/build or download after.

---

## 7. Repo / instrument state (Claude Code, updated 2026-07-21)

Two adversarially-verified audit sweeps of the foundations, remediated and on
`main`. **92 tests pass, clippy clean, CI GREEN** (see §8).

- **Anti-cheat (answer leakage) fixed.** Scaffolded runs were pasting each test's
  `formal_spec` — whose `⊢`/`⊬` turnstile *is* the VALID/INVALID answer — into the
  model prompt. Replaced with a **leak-free scaffold**: the argument form as an
  open question (`⊢?`), verdict withheld — the legitimate "you seem weak here,
  look at this structure" hint. 6 unit tests prove no verdict leaks. The 27
  contaminated `scaffolded` runs are quarantined (`answer_leak_contamination`);
  the **508 clean-room runs — the real science — were never contaminated.**
- **Quarantine fixed** to fire only when infra noise *dominates* a run (a perfect
  78/78 run was being hidden for one infra blip). Migrations 044–046 reconciled
  history.
- **Fabricated "3× / 88%" spec-decode metric removed** — now aggregated from real
  persisted draft-token counters.
- **Security:** DNS-rebinding Host-header guard (`security.rs`); `/api/cloud-keys`
  no longer leaks the secrets path or key prefixes.
- **Portability:** `objc2-metal` (macOS Metal GPU ceiling) gated behind
  `cfg(target_os = "macos")` — the core binary now builds on Linux (returns an
  honest `None` off-Mac). This is what let CI go green.
- Plus ~15 correctness fixes (spec-decode v0 counters, Run-Just-This axis filter,
  fountain verdicts, fallible scoring parse, identifier normalization, …).

### Known-open (Claude Code, next up)
1. Test-battery data fixes (VVP-01 prompt leak, fib `\n`, substring-scored
   numerics, fallacy label mismatches).
2. Provenance sealing (I3/I6) — test fields editable after seal; aborted/errored
   runs unsealed.
3. Aggregate honesty — exclude `scaffolded` runs from leaderboards; PASS-RATE
   dial relabel.
4. **GUI magic** — replace the duplicate-ID `cloneNode` Focused mode with a
   single-source render; make the spec-stream (lean formulas) *pop*; clear
   "loaded / running" indicators.
5. Strategic refactors (green-lit): runtime seam for ollama/llama.cpp; dashboard
   split; trial-granular quarantine; unify config-scan/normalizer/DB/executor.

## 8. CI (GitHub Actions — GREEN as of 2026-07-21)

CI had **never** passed before this (the binary depended unconditionally on
`objc2-metal`, macOS-only, so the ubuntu runner couldn't compile it; and a
pinned `sqlx-cli 0.9.3` doesn't exist). Now three green jobs:
- **`logic-gate`** — Python + Postgres; runs the logic verifier (§9). Independent
  of the Rust build so it protects the science even if the app has a platform hiccup.
- **`quality-gate`** — fmt · clippy(-D warnings) · build · `cargo test --lib`
  (unit tests only; the integration suite needs macOS host APIs + seeded DB + live
  LM Studio and runs locally). Migrations applied via `psql` (not sqlx-cli).
- **`codeql`** — Rust security scan.

## 9. Logic ground-truth verifier (the seL4-style gate)

`scripts/verify_logic_ground_truth.py` — a **complete decision procedure**
(exhaustive truth-tables + finite-model search with the finite-model-property
justification; every INVALID backed by an explicit countermodel). Proves the
seeded logic ground truths are logically correct — nobody takes them on faith.
**28/28 verified.** `--check-owl-families` adds a live-DB consistency check
(every N/C paraphrase row must share its owl-root's formal skeleton). Both run in
CI. _Future deepening: parse EVERY logic test's `formal_spec` from the live DB
and re-derive its verdict, so a NEW test (not in the hardcoded battery) is
validated too._

## 10. Operational notes (Claude Code)

- Service is launchd-managed (`ai.hermes.calibration-scope-dashboard`); serves
  `assets/dashboard.html` from disk (UI edits live without rebuild). Rust changes
  need `cargo build --release` + restart. **0 clippy warnings enforced.**
- LM Studio spec-decode counters are only in `/api/v0/chat/completions` `usage`
  (`accepted_draft_tokens_count`), never `/v1`.
- Gotcha: a freshly-built binary can stall the **launchd exec** in a macOS
  security scan (`dyld3::open`); the same binary runs fine in a foreground shell.
  Recovers after the scan settles / a reboot.

---

## 11. Next steps (open — both tools)

- [x] Merge PR #1 (leak fix verified) → adopt main-direct → wire verifier gate #19. **DONE.**
- [x] CI green. **DONE.**
- [ ] Modify the Rust root task to do something in *our* system; rebuild; hold TEST_PASS. _(Claude Science)_
- [ ] Wire seL4 build+boot+validate as a CI-style release gate (compute role #2). _(Claude Science)_
- [ ] Stand up the heavy Spot box; run l4v Isabelle/HOL proof (empirical boot → proven correct). _(Claude Science)_
- [ ] Open-science moves #1–#6 (data package + DOI first); confirm Cognitive Atlas IDs before publishing. _(Claude Science → artifacts → Claude Code commits)_
- [ ] Test-battery data fixes · provenance sealing · aggregate honesty · GUI magic. _(Claude Code — §7)_
- [ ] **Stop the EC2 box when idle** (billing CPU while running). _(Carey/Claude Science)_
