#!/bin/bash
# Idle watchdog: shutdown (=EC2 stop) after 6 consecutive idle checks (30 min).
# Idle = no SSH sessions AND 1-min load < 0.2 (build jobs keep it alive).
#
# SENTINEL OVERRIDE: touch /tmp/PROOF_RUNNING before a detached long job
# (l4v Isabelle/HOL proof, big rebuild) that may go I/O-quiet for >30 min
# while disconnected. While that file exists and is <24h old, the watchdog
# stands down. Staleness cap prevents a forgotten sentinel from recreating
# the always-on bill this watchdog exists to kill. rm it when the job ends.
STATE=/run/idle-shutdown.count
SENTINEL=/tmp/PROOF_RUNNING
if [ -f "$SENTINEL" ]; then
  AGE=$(( $(date +%s) - $(stat -c %Y "$SENTINEL") ))
  if [ "$AGE" -lt 86400 ]; then
    echo 0 > $STATE
    logger -t idle-shutdown "sentinel present (age ${AGE}s) — standing down"
    exit 0
  fi
  logger -t idle-shutdown "sentinel STALE (age ${AGE}s > 24h) — ignoring it"
fi
# Count ESTABLISHED TCP on :22 (ss), not `who`: non-interactive SSH
# (agent exec channels) never appears in who/utmp — proven live 2026-07-23
# when the counter hit 1 WHILE an agent was connected.
SESSIONS=$(ss -Htn state established sport = :22 | wc -l)
LOAD=$(awk "{print (\$1 < 0.2) ? 0 : 1}" /proc/loadavg)
if [ "$SESSIONS" -eq 0 ] && [ "$LOAD" -eq 0 ]; then
  N=$(( $(cat $STATE 2>/dev/null || echo 0) + 1 ))
  echo $N > $STATE
  logger -t idle-shutdown "idle check $N/6"
  if [ "$N" -ge 6 ]; then
    logger -t idle-shutdown "30 min idle — stopping instance"
    /sbin/shutdown -h now
  fi
else
  echo 0 > $STATE
fi
