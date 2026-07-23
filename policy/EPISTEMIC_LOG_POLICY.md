# EPISTEMIC LOG POLICY — data provenance, retest, and change tracking
_Author: Claude Science, 2026-07-23, at Carey's direction. Applies to ALL agents (Claude Code, Claude Science, Hermes)._
_Governs: how we stay honest about what our data means as it changes over time._

## Why this exists
This project's whole value is that its results can be TRUSTED — objective scoring,
N=3, SHA seals, verified-not-asserted. But data changes: tests get fixed, bugs get
found, runs get re-done, schemas migrate. Every change is a chance for a stale or
contaminated result to keep masquerading as good science. This policy makes every
change visible, reversible-in-audit, and hashed.

Precedents this session that motivated it (all "asserted but not real," all caught
by cross-check): the scaffold answer-key leak, hallucinated Cognitive Atlas IDs caught and
replaced (§10.13), the `who`-based watchdog bug, the 6x cost over-estimate, the
underpowered N=102 Carrier Color spectrum.

RECONCILED, Claude Science live re-derivation against cognitiveatlas.org
2026-07-23 (own tool call, not relayed — receipt: cognitiveatlas_reverify.json):
the "6/6" (Hermes) vs "3 of 6" (§11) dispute was DEFINITIONAL, both true:
  - As a construct->ID crosswalk, 6/6 old IDs were WRONG.
  - Of those, 3 return hard HTTP 404; the other 3 resolve to a VALID but
    DIFFERENT construct (e.g. the id given for "response inhibition" returns
    "response selection"; "decision making"->"risk seeking"; "cognitive
    control"->"risk aversion"). "3 of 6" counted only the hard 404s.
  - All 6 REPLACEMENT ids resolve to the correct construct name. Verified.
Meta-lesson: neither prior number was a lie — they measured different things,
and only a first-hand live check disambiguated them. Two lessons stack: (1) a
*summary* of a correction introduced a second framing ("6/6" vs "3/6") that read
as a contradiction — compression is a carrier that colors the signal; (2) an
earlier draft of THIS file logged Hermes's relayed "6/6" as if Claude Science
had verified it — the exact trust-without-checking failure the policy forbids.
Both are on the record precisely so the pattern is visible.

## Rule 1 — INCOMPLETE or BUG-TAINTED artifacts get quarantined, not deleted
If a test/dataset/run is (a) incomplete (missing families, underpowered), or (b) was
produced by code since found buggy:
- **Flag it** in the epistemic log (below) with status `QUARANTINED`.
- **Do not silently delete or overwrite it** — quarantine preserves the audit trail.
- **Do not cite it as conclusive** until it is re-run/completed and re-verified.
- The artifact keeps its original hash so "what we used to believe" stays inspectable.
Currently-known QUARANTINE candidates (as of 2026-07-23):
  - Carrier Color N=102 unpaired spectrum (§10.8) — underpowered; superseded by the
    paired v1 re-run (carrier_color_experiment_spec_v1.md). Endpoint result stands;
    the ORDERING is quarantined pending the paired run.
  - OWL N/C coverage: LOGIC-05/07/08/09/10/11 — incomplete; retest when authored.

## Rule 2 — any external ID / citation is resolved LIVE before it is treated as real
(Already on the record from §10.13, restated here as the general form.) A plausible,
well-formatted identifier is not a verified one. Cognitive Atlas IDs, DOIs, ontology
terms, package versions, cost figures — resolve against the live source, log the
resolution date, THEN use it. "Measured, not estimated."

## Rule 3 — every data reset / schema change / re-run is logged and hashed
Append one entry to `EPISTEMIC_LOG.jsonl` (one JSON object per line) for EVERY:
  - data reset / truncation / participant wipe
  - migration that changes existing rows' meaning
  - quarantine or un-quarantine of an artifact
  - re-run that supersedes a prior result
Entry schema:
  {
    "ts_utc": "2026-07-23T03:00:00Z",
    "agent": "hermes | claude-code | claude-science",
    "action": "reset | migrate | quarantine | rerun | verify | supersede",
    "target": "table/artifact/run id or path",
    "reason": "one sentence — WHY",
    "supersedes": "prior artifact hash or run id, if any (else null)",
    "sha256": "hash of the NEW artifact/state after the change",
    "receipt": "path under evidence/ if a sealed receipt exists (else null)"
  }
The log is append-only. Never edit a past line; correct by appending a new entry.

## Rule 4 — hash everything durable
Any artifact that leaves scratch (committed, evicted, published) carries a sha256
(and sha3-256 for tamper-evidence on high-value receipts, per evidence/sel4/ and
evidence/watchdog/). The hash goes in the artifact's MANIFEST and in the epistemic
log entry that introduced it. Hashes recomputed after any transfer must match.

## What "still doing good science" means here (Carey's standard, recorded)
Data changing is NORMAL and fine. Hiding that it changed is not. As long as every
reset/fix/re-run is logged, hashed, and the superseded thing is quarantined rather
than quietly replaced, the record stays honest and the science holds. The log IS
the proof of integrity.
