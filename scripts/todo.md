# Archetype Mesh Benchmark TODO

## Current State
Zig backend is parked. The C-socket server compiles and prints `listening on 127.0.0.1:8768`, but exits before any connection can be accepted. Needs runtime diagnosis before HTTP/SQLite/ledger work.

## Unblocked Now
- Legacy automation/cron removed.
- Python dashboard removed from repo.

## Next Diagnostic Session
- Determine why `main` exits immediately after listen.
- Use Zig Windows-style stdout path or absolute binary path evidence.
- Do not add blind instrumentation; confirm lifecycle with real output capture.
