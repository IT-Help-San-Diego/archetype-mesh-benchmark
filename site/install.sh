#!/bin/sh
# Calibration Scope installer — https://calibrationscope.com
#
#   curl -fsSL https://calibrationscope.com/install.sh | sh
#
# What this does, in order (macOS):
#   1. If the instrument already answers on 127.0.0.1:8768 — opens it, done.
#      If something ELSE holds that port, or another calibration-scope service
#      exists, it stops and tells you — it never installs a competitor.
#   2. brew install it-help-san-diego/tap/calibration-scope
#      (self-contained binary; postgresql@17 comes along as a dependency)
#   3. Ensures a PostgreSQL server is reachable on 127.0.0.1:5432 — uses yours
#      if one is listening, otherwise starts the brew one as a boot-persistent
#      service.
#   4. Creates the calibration_scope database and VERIFIES the connection
#      before writing the dashboard service.
#   5. Installs a launchd service (com.calibrationscope.dashboard), then opens
#      http://127.0.0.1:8768.
#
# Design rules this script obeys:
#   - NEVER interactive: no prompts, no editors, no pagers, and psql/createdb
#     run with -w (never ask for a password). It either finishes or exits
#     loudly with exactly what to do next. (A one-liner is only as good as it
#     can close on its own self.)
#   - Everything is wrapped in main() and invoked on the LAST line, so a
#     truncated download parses to nothing instead of executing half a script.
#   - Safe to re-run: a re-run repairs a broken install. (Upgrades are brew's
#     job: brew upgrade it-help-san-diego/tap/calibration-scope)
#   - No telemetry, nothing phones home. This script talks to brew, your
#     local Postgres, and 127.0.0.1 — that's all.
#
# Knobs (env vars, all optional):
#   CALIBRATION_SCOPE_DATABASE_URL  use this connection string instead of the
#                                   auto-provisioned postgres://<you>@127.0.0.1

set -eu

TAP="it-help-san-diego/tap"
FORMULA="calibration-scope"
LABEL="com.calibrationscope.dashboard"
PORT=8768
DB_NAME="calibration_scope"
# Bumped by the release runbook (docs/RELEASING.md) — used only for the Linux
# pointer; the macOS path always installs the tap's current formula.
RELEASE_TAG="v0.1.0-beta.1"

# Colors only when talking to a terminal — logs and CI capture get plain text.
if [ -t 1 ]; then Y='\033[1;33m'; N='\033[0m'; else Y=''; N=''; fi
if [ -t 2 ]; then R='\033[1;31m'; N2='\033[0m'; else R=''; N2=''; fi
say()  { printf "${Y}[calibration-scope]${N} %s\n" "$*"; }
fail() { printf "${R}[calibration-scope] FAIL:${N2} %s\n" "$*" >&2; exit 1; }

main() {
    case "$(uname -s)" in
        Darwin) ;;
        Linux)
            say "Linux: grab the prebuilt binary with:"
            say "  curl -LsSf https://github.com/IT-Help-San-Diego/calibration-scope/releases/download/${RELEASE_TAG}/calibration-scope-installer.sh | sh"
            say "then install PostgreSQL with your distro's package manager (apt install postgresql), set"
            say "DATABASE_URL (see .env.example in the repo), and run calibration-scope-dashboard."
            exit 0
            ;;
        *) fail "unsupported OS: $(uname -s) — macOS and Linux only" ;;
    esac

    DB_USER="${USER:-$(id -un)}"

    # ── 1. Port & sibling-service safety. Never install a competitor. ───────
    if curl -sf -m 2 "http://127.0.0.1:${PORT}/api/status" >/dev/null 2>&1; then
        say "Calibration Scope is already running on 127.0.0.1:${PORT} — opening it."
        say "(To upgrade:  brew upgrade ${TAP}/${FORMULA}  then restart the service.)"
        open "http://127.0.0.1:${PORT}" || true
        exit 0
    fi
    HOLDER="$(lsof -nP -iTCP:${PORT} -sTCP:LISTEN -Fc 2>/dev/null | sed -n 's/^c//p' | head -1 || true)"
    [ -n "$HOLDER" ] && fail "port ${PORT} is held by '${HOLDER}' but /api/status doesn't answer —
  that's a stuck instance or another app. Fix or stop it, then re-run."
    OTHER_PLIST="$(grep -ls "calibration-scope" "$HOME"/Library/LaunchAgents/*.plist 2>/dev/null | grep -v "${LABEL}.plist" | head -1 || true)"
    [ -n "$OTHER_PLIST" ] && fail "another calibration-scope service already exists: ${OTHER_PLIST}
  Installing a second one would fight it for port ${PORT} at every boot.
  Remove it (launchctl bootout + rm) or keep using it — then re-run if needed."

    # ── 2. Homebrew is the engine of the macOS path. ────────────────────────
    command -v brew >/dev/null 2>&1 || fail "Homebrew is required (it delivers the binary AND PostgreSQL).
  Install it from https://brew.sh — then re-run this one-liner."
    BREW_PREFIX="$(brew --prefix)"
    PGBIN="${BREW_PREFIX}/opt/postgresql@17/bin"

    say "Installing ${TAP}/${FORMULA} (binary + postgresql@17 dependency)..."
    brew tap "${TAP}" </dev/null >/dev/null 2>&1 || true
    if brew list "${FORMULA}" >/dev/null 2>&1; then
        brew upgrade "${TAP}/${FORMULA}" </dev/null \
            || say "brew upgrade did not complete (brew's output above says why) — continuing with the installed version."
    else
        brew install "${TAP}/${FORMULA}" </dev/null
    fi
    BIN="${BREW_PREFIX}/bin/calibration-scope-dashboard"
    [ -x "$BIN" ] || fail "expected binary at $BIN after brew install — check 'brew doctor'."

    # ── 3. A PostgreSQL server on 127.0.0.1:5432 — yours, or brew's. ────────
    if "$PGBIN/pg_isready" -h 127.0.0.1 -p 5432 -q 2>/dev/null; then
        PG_OWNER="$(lsof -nP -iTCP:5432 -sTCP:LISTEN -Fc 2>/dev/null | sed -n 's/^c//p' | head -1 || true)"
        say "Found an existing PostgreSQL server on 5432 (process: ${PG_OWNER:-unknown}) — using it."
        say "NOTE: the dashboard will depend on THAT server at every boot. If this is an"
        say "      ssh tunnel or a container forward, stop it and re-run to use brew's service."
    else
        OTHER_PG="$(brew services list 2>/dev/null | awk '$1 ~ /^postgresql@/ && $1 != "postgresql@17" && $2 != "none" {print $1; exit}' || true)"
        [ -n "$OTHER_PG" ] && fail "brew service ${OTHER_PG} is registered but not answering on 5432.
  Starting a second cluster (postgresql@17) beside it would split-brain your setup.
  Fix yours first:  brew services restart ${OTHER_PG}  — then re-run."
        say "Starting postgresql@17 as a brew service (registers to start at every boot)..."
        brew services restart postgresql@17 </dev/null >/dev/null 2>&1 || true
        i=0
        until "$PGBIN/pg_isready" -h 127.0.0.1 -p 5432 -q 2>/dev/null; do
            i=$((i + 1))
            [ "$i" -ge 30 ] && fail "PostgreSQL did not come up in 30s.
  Look at: brew services info postgresql@17"
            sleep 1
        done
    fi

    # ── 4. Database + verified connection BEFORE the dashboard service. ─────
    DATABASE_URL="${CALIBRATION_SCOPE_DATABASE_URL:-postgres://${DB_USER}@127.0.0.1:5432/${DB_NAME}}"
    PGCONNECT_TIMEOUT=5 "$PGBIN/createdb" -w -h 127.0.0.1 -p 5432 "${DB_NAME}" 2>/dev/null \
        && say "Created database ${DB_NAME}." \
        || say "Database ${DB_NAME} already exists (or needs other auth) — verifying the connection next."
    PSQL_ERR="$(PGCONNECT_TIMEOUT=5 "$PGBIN/psql" -w "$DATABASE_URL" -Atqc "SELECT 1" 2>&1 >/dev/null)" \
        || fail "cannot connect to ${DB_NAME} as ${DB_USER}. psql said: ${PSQL_ERR}
  Fix one of these, then re-run:
    - allow passwordless local access for ${DB_USER} (pg_hba trust/peer for 127.0.0.1), or
    - create the database/role yourself:  createdb ${DB_NAME}  (as a user that can), or
    - re-run with your own connection string:
        CALIBRATION_SCOPE_DATABASE_URL='postgres://user:pass@127.0.0.1:5432/${DB_NAME}' sh install.sh"

    # ── 5. launchd service: survives reboots, restarts on crash — but not in
    #      a storm: KeepAlive only after unsuccessful exits, 30s throttle.
    #      (Values below are not XML-escaped: macOS short names and brew
    #      prefixes cannot contain & < > — revisit if arbitrary strings are
    #      ever interpolated here.) ─────────────────────────────────────────
    LOGDIR="$HOME/Library/Logs/calibration-scope"
    mkdir -p "$LOGDIR"
    PLIST="$HOME/Library/LaunchAgents/${LABEL}.plist"
    say "Writing ${PLIST}..."
    cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>${LABEL}</string>
    <key>ProgramArguments</key>
    <array><string>${BIN}</string></array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>DATABASE_URL</key><string>${DATABASE_URL}</string>
        <key>CALIBRATION_SCOPE_NO_DOTENV</key><string>1</string>
    </dict>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key>
    <dict><key>SuccessfulExit</key><false/></dict>
    <key>ThrottleInterval</key><integer>30</integer>
    <key>StandardOutPath</key><string>${LOGDIR}/out.log</string>
    <key>StandardErrorPath</key><string>${LOGDIR}/err.log</string>
</dict>
</plist>
PLIST_EOF

    launchctl bootout "gui/$(id -u)/${LABEL}" 2>/dev/null || true
    launchctl enable "gui/$(id -u)/${LABEL}" 2>/dev/null || true
    i=0
    until launchctl bootstrap "gui/$(id -u)" "$PLIST" </dev/null 2>/dev/null; do
        i=$((i + 1))
        [ "$i" -ge 3 ] && fail "launchctl bootstrap failed — this needs a GUI login session.
  Over SSH: log in at the console once, or run manually:
    launchctl bootstrap gui/$(id -u) ${PLIST}"
        sleep 1
    done
    launchctl kickstart "gui/$(id -u)/${LABEL}" </dev/null 2>/dev/null || true

    say "Waiting for the instrument to come up on 127.0.0.1:${PORT}..."
    i=0
    until curl -sf -m 2 "http://127.0.0.1:${PORT}/api/status" >/dev/null 2>&1; do
        i=$((i + 1))
        [ "$i" -ge 30 ] && fail "service did not answer in 30s.
  Logs: ${LOGDIR}/err.log   Live state: launchctl print gui/$(id -u)/${LABEL}"
        sleep 1
    done

    # ── 6. The extras, honestly labeled. ────────────────────────────────────
    if ! curl -sf -m 2 "http://127.0.0.1:1234/api/v0/models" >/dev/null 2>&1; then
        say "NOTE: LM Studio isn't serving on :1234 — local-model runs need it."
        say "      Get it at https://lmstudio.ai (Developer tab -> start the local server)."
        say "      Cloud models work without it (add API keys on the setup page)."
    fi

    say ""
    say "Done. Dashboard:   http://127.0.0.1:${PORT}"
    say "Evidence DB:       ${DATABASE_URL}  (open it with TablePlus or any client — the schema IS the API)"
    say "Service logs:      ${LOGDIR}/out.log  ${LOGDIR}/err.log"
    say "Uninstall:         launchctl bootout gui/\$(id -u)/${LABEL} && rm ${PLIST} && brew uninstall ${FORMULA}"
    say "                   (+ 'brew services stop postgresql@17' if this script started it."
    say "                    Your evidence database is KEPT — 'dropdb ${DB_NAME}' only if you want it gone.)"
    open "http://127.0.0.1:${PORT}" || true
}

main "$@"
