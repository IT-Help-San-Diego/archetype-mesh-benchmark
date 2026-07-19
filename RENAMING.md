# RENAMING — the renameability contract

The product name is **Calibration Scope** (ratified 2026-07-19, replacing the
working name "Calibration Scope", and superseding the earlier "Carrier Scope"
ratification which was dropped because `carrierscope.com` is a live commercial
mark). **Domain: `calibrationscope.com`** (registered 2026-07-19, Amazon
Registrar; processing). Calibration Scope is derived from the instrument's ACT:
**calibrating any cognitive source** (silicon models + carbon humans) against
sealed ground truth, *scoping signal from carrier* within that calibration,
under the Owl Semaphore V4 taxonomy (I/N/C/M), with the Verification Principle
as discipline (SHA-3 sealed, no answer-leakage) and Intellectual Resistance as
the program roof. It is NOT a fourth framework — it is the instrument that puts
the other three (Owl Semaphore, Carrier Color, Verification Principle) to work.
The Carrier Color *variable* (`carrier_variance` in `owl_signal_carrier`) stays
detected underneath; it is not the name.

**Domain (registered):** `calibrationscope.com`. Local-access subdomain would be
`local.calibrationscope.com A 127.0.0.1` (see `docs/local-access-strategy.md`).

## Naming rule (locked)

> The product name stays **substrate-neutral**: **Calibration Scope**. It does
> NOT carry "LM", "AI", or any substrate/era marker. The instrument's act
> (calibrating a cognitive source, scoping signal from carrier within it)
> applies to silicon AND carbon; tying the name to "AI" or "local models"
> (the current trend) would (1) narrow it below what it is, (2) date it to a
> trend that won't hold, and (3) force a rename the moment human calibration
> ships. Discoverability ("what is this?") is handled by a **concrete tagline**,
> not by polluting the name:
>
> *Calibration Scope — a scientific instrument that calibrates any cognitive
> source, silicon or carbon, scoping signal from carrier with sealed, auditable
> evidence.*
>
> Substrate/use-case lives in the tagline where it can change; the name stays.
>
> The product NAME and the DOMAIN are the SAME word (Calibration Scope /
> calibrationscope.com) — do not maintain a separate brand name and domain.

**Human calibration first (core priority):** silicon and carbon are measured
under one battery, but the HUMAN is the prior term — a person takes the battery
and gets their own pass/fail vector BEFORE the machine mirror is trusted. Mode 1
("thinks like them") and Mode 2 ("fills the gaps") both presuppose human
calibration first. Measure yourself before you trust the reflection.

## Where the name lives (the anchor points)

Verified 2026-07-19 by grep. The rename to **Calibration Scope** was executed
in commit <rename commit> — all anchors below were updated to their final
values (no further rename pending):

| File | Role | Final value |
|------|------|-------------|
| `Cargo.toml` | package + binary name | `calibration-scope` / `calibration-scope-dashboard` |
| `src/main.rs` | log filter + startup log | `calibration_scope_dashboard` / "Starting Calibration Scope Dashboard" |
| `src/routes/cloud_keys.rs` | secrets dir const | `.calibration-scope` |
| `assets/dashboard.html` | title + hero | "Calibration Scope" |
| `scripts/deploy.sh` | bin/plist/service/errlog paths | `calibration-scope-dashboard` |
| secrets dir | `~/.archetype-mesh/` → | `~/.calibration-scope/` |
| database | `archetype_mesh` → | `calibration_scope` |
| launchd plist | Label + binary + WorkingDirectory + DATABASE_URL | `ai.hermes.calibration-scope-dashboard` / `calibration_scope` |
| local folder | `archetype-mesh-benchmark` → | `calibration-scope` |

NOTE: there is no `ARCHETYPE_MESH_*` env prefix in code — the only env-style
reference is the `DATABASE_URL` (and `GEMINI_API_KEY` etc., deployment env
only). The secrets directory (`.calibration-scope`) is the canonical key store.
`CARGO_MANIFEST_DIR` is baked at compile time, so a folder move REQUIRES a
forced rebuild (`touch src/main.rs && cargo build --release`) — cargo does not
track the manifest dir as a dependency.

## Rename procedure (mechanical)

1. `Cargo.toml` → update `package.name` and `[[bin]] name`.
2. `src/main.rs` → update process name string + `CARRIER_SCOPE_` env prefix
   (rename the prefix consistently; it is the config-key namespace).
3. `assets/dashboard.html` → global replace the visible product name.
4. `README.md` + other docs → global replace prose references.
5. `git grep` the old name → confirm zero code-logic references remain.

Expected effort: ~15 minutes. No logic depends on the name as a magic string.

## Hard rule (enforced in review)

> Never hardcode the product name as a magic string inside business logic.
> The name belongs in: `Cargo.toml` (package/binary), `main.rs` (env prefix +
> process name), and titled string literals only. If code needs the name at
> runtime, read it from a single configured constant, not an inline literal.

## Status

- Name ratified: **Carrier Scope**. Repo still carries the old "Archetype
  Mesh" code anchors pending the mechanical rename (deferred to domain
  registration / a deliberate rename commit). This doc preserves the contract.
