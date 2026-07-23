#!/bin/bash
# Install the EC2 idle-shutdown watchdog on a fresh builder box.
# Run as: sudo ./install.sh   (from this directory)
#
# What it does: copies the watchdog script + systemd units EXACTLY as
# version-controlled here, enables the timer (survives reboot), and runs
# one check immediately. See README.md for the trigger logic and the
# /tmp/PROOF_RUNNING sentinel contract for long detached jobs.
set -euo pipefail
cd "$(dirname "$0")"

install -m 755 idle-shutdown.sh /usr/local/bin/idle-shutdown.sh
install -m 644 idle-shutdown.service /etc/systemd/system/idle-shutdown.service
install -m 644 idle-shutdown.timer /etc/systemd/system/idle-shutdown.timer

systemctl daemon-reload
systemctl enable --now idle-shutdown.timer

echo "installed. verification:"
systemctl is-enabled idle-shutdown.timer
systemctl list-timers idle-shutdown.timer --no-pager | head -3
bash -n /usr/local/bin/idle-shutdown.sh && echo "script syntax OK"
