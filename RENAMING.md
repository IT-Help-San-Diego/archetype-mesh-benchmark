# RENAMING — the renameability contract

The product name is **Carrier Scope** (ratified 2026-07-19, replacing the
working name "Archetype Mesh"). It is derived from the instrument's ACT:
scoping **signal from carrier** (Carrier Color theory) across BOTH substrates
(silicon models + carbon humans) under the Owl Semaphore V4 taxonomy
(I/N/C/M), with the Verification Principle as discipline (SHA-3 sealed, no
answer-leakage) and Intellectual Resistance as the program roof. Carrier Scope
is NOT a fourth framework — it is the instrument that puts the other three
(Owl Semaphore, Carrier Color, Verification Principle) to work.

**Domain (TBD, budget pending):** `carrierscope.com` is **already registered
to a commercial entity** (verified 2026-07-19) — not available. Alternatives to
evaluate when budget allows: `carrierscope.science` (fits the instrument's
scientific framing), `carrierscope.dev` / `carrierscope.io` (dev/tech), or a
prefix form (`getcarrierscope.com`, `trycarrierscope.com`). Local-access
subdomain would be `local.<domain> A 127.0.0.1` (see
`docs/local-access-strategy.md`). Decision deferred.

## Naming rule (locked)

> The product name stays **substrate-neutral**: **Carrier Scope**. It does NOT
> carry "LM", "AI", or any substrate/era marker. The instrument's act (scoping
> signal from carrier) applies to silicon AND carbon; tying the name to "AI" or
> "local models" (the current trend) would (1) narrow it below what it is,
> (2) date it to a trend that won't hold, and (3) force a rename the moment
> human calibration ships. Discoverability ("what is this?") is handled by a
> **concrete tagline**, not by polluting the name:
>
> *Carrier Scope — a scientific instrument that scopes signal from carrier in
> any cognitive source, silicon or carbon, with sealed, auditable evidence.*
>
> Substrate/use-case lives in the tagline where it can change; the name stays.

**Human calibration first (core priority):** silicon and carbon are measured
under one battery, but the HUMAN is the prior term — a person takes the battery
and gets their own pass/fail vector BEFORE the machine mirror is trusted. Mode 1
("thinks like them") and Mode 2 ("fills the gaps") both presuppose human
calibration first. Measure yourself before you trust the reflection.

## Where the name lives (the anchor points)

Verified 2026-07-19 by grep (~39 references total for the old name; only ~4 are
code anchors). On rename, update:

| File | Role | What to change |
|------|------|----------------|
| `Cargo.toml` | package name `archetype-mesh-benchmark`, binary `archetype-mesh-dashboard` | → `carrier-scope` / `carrier-scope-dashboard` |
| `src/main.rs` | process name; `ARCHETYPE_MESH_*` env prefix (e.g. `ARCHETYPE_MESH_DATABASE_URL`) | → `CARRIER_SCOPE_*` prefix |
| `assets/dashboard.html` | title string + visible labels | titled literals |
| `README.md` | 11 doc references | prose |

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
