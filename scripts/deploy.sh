#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# deploy.sh — the ONE way to ship the Archetype Mesh dashboard.
#
# Exists because every step below has ALREADY bitten us when done by hand:
#
#   1. cargo test silently fails 26/26 integration tests without DATABASE_URL
#      (needs .env sourced — bitten 2026-07-09).
#   2. A broken <script> block ships silently — HTML has no compiler
#      (bitten 2026-07-08: "loadModels is not defined").
#   3. macOS kills the freshly-built binary with OS_REASON_CODESIGNING:
#      cargo's output isn't signed to launchd's cached expectations, and
#      `launchctl kickstart` does NOT clear that cache. The proven fix is
#      codesign --force + xattr -d quarantine + full bootout/bootstrap
#      (bitten 2026-07-09: dashboard down ~10 min, 000 responses).
#   4. Service comes up slowly; a single curl right after restart reads 000
#      and looks like failure (bitten repeatedly).
#   5. When it IS down, the reason lives in the err log and launchd's
#      "last exit reason" — nobody looks there until late.
#
# Usage:
#   scripts/deploy.sh            # gate + build + test + sign + reload + verify
#   scripts/deploy.sh --skip-gate  # skip Lean/oracle gate (still builds+tests)
#
# Exit codes: 0 = deployed and verified live. Anything else = NOT deployed,
# with the reason printed. No partial success states.
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$REPO/target/release/archetype-mesh-dashboard"
PLIST="$HOME/Library/LaunchAgents/ai.hermes.archetype-mesh-dashboard.plist"
SERVICE="ai.hermes.archetype-mesh-dashboard"
DOMAIN="gui/$(id -u)"
ERRLOG="$HOME/.hermes/logs/archetype-mesh-dashboard.err.log"
URL="http://127.0.0.1:8768"
SKIP_GATE=0
[[ "${1:-}" == "--skip-gate" ]] && SKIP_GATE=1

step() { printf '\n\033[1;33m── %s\033[0m\n' "$*"; }
die()  { printf '\n\033[1;31m✗ DEPLOY FAILED: %s\033[0m\n' "$*" >&2; exit 1; }
ok()   { printf '\033[1;32m✓ %s\033[0m\n' "$*"; }

cd "$REPO"

# ── 0. Pre-flight: environment + database ─────────────────────────────────
step "[0/7] Pre-flight"
[[ -f .env ]] || die ".env missing — DATABASE_URL required for tests and runtime"
set -a; source .env; set +a
[[ -n "${DATABASE_URL:-}" ]] || die "DATABASE_URL not set in .env"

docker ps --format '{{.Names}}' | grep -q '^archetype-postgres$' \
  || die "archetype-postgres container not running (docker start archetype-postgres)"
docker exec archetype-postgres pg_isready -q \
  || die "Postgres container up but not accepting connections"
ok "env + database ready"

# ── 1. Frontend syntax: HTML ships uncompiled — WE are its compiler ──────
step "[1/7] JS syntax check (dashboard.html)"
node -e '
  const html = require("fs").readFileSync("assets/dashboard.html","utf8");
  const blocks = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
  if (!blocks.length) { console.error("no inline <script> blocks found"); process.exit(1); }
  for (const [i, b] of blocks.entries()) {
    try { new Function(b[1]); }
    catch (e) { console.error(`script block ${i}: ${e.message}`); process.exit(1); }
  }
  console.log(`${blocks.length} script block(s) parse clean`);
' || die "dashboard.html has a JS syntax error — fix before shipping"
ok "frontend parses"

# ── 2. Build ───────────────────────────────────────────────────────────────
step "[2/7] cargo build --release"
cargo build --release 2>&1 | tail -2
[[ -x "$BIN" ]] || die "binary missing after build: $BIN"
ok "built $(du -h "$BIN" | cut -f1 | xargs)"

# ── 3. Tests (DATABASE_URL already exported — integration tests need it) ──
step "[3/7] cargo test --release"
TEST_OUT=$(cargo test --release 2>&1) || { echo "$TEST_OUT" | tail -30; die "tests failed"; }
echo "$TEST_OUT" | grep -E "^test result" | while read -r line; do
  echo "  $line"
  [[ "$line" == *"0 failed"* ]] || die "test suite reported failures"
done
ok "all suites green"

# ── 4. Verification gate (Lean kernel + Python oracle + tests agree) ─────
if [[ $SKIP_GATE -eq 0 && -x scripts/verification_gate.sh ]]; then
  step "[4/7] verification gate (Lean + oracle + cargo)"
  scripts/verification_gate.sh >/dev/null 2>&1 || die "verification gate refused — three verifiers do NOT agree"
  ok "gate PASS"
else
  step "[4/7] verification gate SKIPPED (--skip-gate)"
fi

# ── 5. Sign + strip quarantine — BEFORE launchd ever sees the binary ─────
#    kickstart alone is NOT enough after a rebuild: launchd's cached
#    signature disagrees with the new Mach-O and the kernel kills it with
#    OS_REASON_CODESIGNING. Ad-hoc re-sign + full bootout/bootstrap is the
#    sequence proven to clear it (2026-07-09).
step "[5/7] codesign + quarantine strip"
xattr -d com.apple.quarantine "$BIN" 2>/dev/null || true
codesign --force --sign - "$BIN" 2>&1 | grep -v "replacing existing" || true
codesign -v "$BIN" || die "signature invalid after re-sign"
ok "ad-hoc signature valid"

# ── 6. Full service reload (bootout → bootstrap, never just kickstart) ───
step "[6/7] launchd reload (bootout → bootstrap)"
launchctl bootout "$DOMAIN/$SERVICE" 2>/dev/null || true   # may not be loaded — fine
# bootout is async; poll until the service is actually gone (max 10s)
for _ in $(seq 1 20); do
  launchctl print "$DOMAIN/$SERVICE" >/dev/null 2>&1 || break
  sleep 0.5
done
launchctl print "$DOMAIN/$SERVICE" >/dev/null 2>&1 && die "service refused to unload"

# Stray-process guard: nothing else may own :8768 when we bootstrap, or the
# new instance dies on bind and launchd flaps. Kill only OUR binary.
STRAY=$(lsof -ti :8768 2>/dev/null || true)
if [[ -n "$STRAY" ]]; then
  for pid in $STRAY; do
    if ps -p "$pid" -o comm= | grep -q archetype-mesh-dashboard; then
      kill "$pid" 2>/dev/null || true
      ok "killed stray dashboard pid $pid holding :8768"
    else
      die "port 8768 held by foreign process $(ps -p "$pid" -o comm=) (pid $pid) — refusing to fight it"
    fi
  done
  sleep 1
fi

launchctl bootstrap "$DOMAIN" "$PLIST" || die "bootstrap failed — plist: $PLIST"
ok "service bootstrapped"

# ── 7. Verify LIVE — HTTP 200 + real JSON + fresh binary, or roll the logs ─
step "[7/7] health verification"
DEADLINE=$((SECONDS + 45))
CODE=000
while (( SECONDS < DEADLINE )); do
  CODE=$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "$URL/" || true)
  [[ "$CODE" == "200" ]] && break
  sleep 2
done
if [[ "$CODE" != "200" ]]; then
  echo "── last exit reason ──"
  launchctl print "$DOMAIN/$SERVICE" 2>/dev/null | grep -E "last exit|state" || true
  echo "── err log tail ──"
  tail -15 "$ERRLOG" 2>/dev/null || echo "(no err log)"
  die "dashboard never reached 200 within 45s (last: $CODE)"
fi

# The page answering is necessary but not sufficient — the API must serve
# real registry JSON (catches migration panics that leave a zombie listener).
curl -s --max-time 5 "$URL/api/models" | python3 -c '
import json,sys
d = json.load(sys.stdin)
models = d if isinstance(d, list) else d.get("models", [])
assert len(models) > 0, "registry empty"
print(f"  registry: {len(models)} models")
' || die "/api/models did not return a valid registry"

PID=$(launchctl print "$DOMAIN/$SERVICE" 2>/dev/null | awk '/pid =/ {print $3; exit}')
RUNNING_BIN=$(ps -p "$PID" -o comm= 2>/dev/null || true)
[[ "$RUNNING_BIN" == "$BIN" ]] || die "running pid $PID is not our binary ($RUNNING_BIN)"
BUILT=$(python3 -c "import os; print(int(os.path.getmtime('$BIN')))")
NOW=$(python3 -c "import time; print(int(time.time()))")
ok "live: pid $PID, binary built $(( NOW - BUILT ))s ago, HTTP 200, registry serving"

printf '\n\033[1;32m═══ DEPLOYED — dashboard verified live at %s ═══\033[0m\n' "$URL"
