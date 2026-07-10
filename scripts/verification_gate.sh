#!/bin/bash
# ─────────────────────────────────────────────────────────────────────────
# Archetype Mesh — the verification gate (seL4 discipline, our scale)
#
# seL4's contribution to software isn't the kernel — it's the refusal:
# nothing ships unless the proof chain holds end to end. This gate is that
# refusal for the benchmark. THREE INDEPENDENT VERIFIERS must agree with
# the seeded ground truth, then the build and its tests must be green:
#
#   1. Lean 4 kernel      — proofs over arbitrary domains + explicit
#                           countermodels (lean/ArchetypeMesh.lean)
#   2. Python oracle      — exhaustive truth tables + complete small-model
#                           search (scripts/verify_logic_ground_truth.py)
#   3. cargo test         — 68 unit + integration tests incl. scoring,
#                           routing, leaderboard, SSE contracts
#
# Any failure = exit 1 = DO NOT SHIP. No partial credit, no warnings-as-
# passes. Run before every push:  ./scripts/verification_gate.sh
# ─────────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

LEAN="${LEAN:-$HOME/.elan/bin/lean}"
FAIL=0

echo "═══ Archetype Mesh Verification Gate ═══"
echo

echo "── [1/3] Lean 4 kernel: formal proofs + countermodel refutations"
if "$LEAN" lean/ArchetypeMesh.lean; then
  echo "    PASS — every theorem checked by the Lean kernel"
else
  echo "    FAIL — a proof no longer holds. A seeded ground truth or spec changed."
  FAIL=1
fi
echo

echo "── [2/3] Python oracle: truth tables + complete finite-model search"
if python3 scripts/verify_logic_ground_truth.py > /tmp/amb_oracle.log 2>&1; then
  tail -1 /tmp/amb_oracle.log
  echo "    PASS — seeded SQL ground truths match computed verdicts"
else
  cat /tmp/amb_oracle.log
  echo "    FAIL — a seeded test contradicts the decision procedure. DO NOT SHIP."
  FAIL=1
fi
echo

echo "── [3/3] cargo test: unit + integration suites"
if [ -f .env ]; then set -a; source .env; set +a; fi
if cargo test --release 2>&1 | grep "test result" | tee /tmp/amb_cargo.log && ! grep -q "FAILED" /tmp/amb_cargo.log; then
  echo "    PASS — all suites green"
else
  echo "    FAIL — test suite not green. DO NOT SHIP."
  FAIL=1
fi
echo

if [ "$FAIL" -eq 0 ]; then
  echo "═══ GATE: PASS — three independent verifiers agree. Ship it. ═══"
else
  echo "═══ GATE: FAIL — verification chain broken. Fix before shipping. ═══"
  exit 1
fi
