# EC2 idle-shutdown watchdog

Auto-stops the seL4 builder (`i-08ca65b7acd2dc275`, c7i.2xlarge, us-west-2)
when it's genuinely abandoned, killing the left-it-on-overnight bill
(~$8.6/day measured — see DECISIONS §13d) while never interrupting real work.

These are the ACTUAL files installed on the box, pulled verbatim via
`systemctl cat` + `scp` on 2026-07-23 — not a restatement. If you change the
box, re-pull; if you change these files, re-run `sudo ./install.sh` there.

## Trigger logic (as read from `idle-shutdown.sh`)

Every 5 minutes (`OnUnitActiveSec=5min`, first check 10 min after boot):

1. **Sentinel override:** if `/tmp/PROOF_RUNNING` exists and is younger than
   24h → reset counter, stand down. (Stale >24h sentinels are ignored so a
   forgotten file can't recreate the always-on bill.)
2. **SSH check:** count ESTABLISHED TCP connections on :22 via
   `ss -Htn state established sport = :22`. **Deliberately NOT `who`** —
   non-interactive SSH (agent exec channels, `ssh host cmd`) never appears
   in utmp; proven live 2026-07-23 when the idle counter advanced to 1
   while an agent was actively connected.
3. **Load check:** 1-min loadavg < 0.2 counts as idle (a real build holds
   load well above this).
4. If **no SSH AND low load**: increment `/run/idle-shutdown.count`
   (tmpfs — resets on boot, so a fresh boot always starts at 0).
   At **6 consecutive idle checks (~30 min)** → `shutdown -h now`, which on
   EC2 = **stopped** (InstanceInitiatedShutdownBehavior=stop, disk-only
   billing). Any SSH session or load spike resets the counter to 0.

## Long detached jobs (l4v Isabelle/HOL proof, big rebuilds)

A detached job that goes I/O-quiet for 30+ min while you're logged out WILL
be stopped. Contract:

```bash
touch /tmp/PROOF_RUNNING     # before launching the detached job
# ... run your job (nohup/screen/tmux, log out freely) ...
rm /tmp/PROOF_RUNNING        # when it finishes (job script should do this)
```

The 24h staleness cap means a crashed job that never cleans up costs at most
one day, not a month.

## Restart after an auto-stop

```bash
aws ec2 start-instances --instance-ids i-08ca65b7acd2dc275 --region us-west-2
```
Elastic IP 44.228.179.31 persists. ~30-60s to SSH-ready.

## Failure modes checked (Claude Science's review list)

- **Timer not enabled →** `install.sh` uses `enable --now`; verify with
  `systemctl is-enabled idle-shutdown.timer` → `enabled`. Confirmed on the
  live box 2026-07-23, including after a real stop/start cycle.
- **`shutdown` from a non-root unit →** the service is a system unit (no
  `User=`), so ExecStart runs as root. Confirmed via
  `systemctl show idle-shutdown.service -p User` (empty = root).
- **Detached-job kill →** the `/tmp/PROOF_RUNNING` sentinel above.
- **`who`-blindness to agent SSH →** fixed; see trigger logic step 2.
