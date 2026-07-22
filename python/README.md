# calibration-scope (Python client)

Read-only Python client for [Calibration Scope](https://calibrationscope.com) — pull sealed benchmark results, leaderboard data, and signal-carrier splits from a running instance into Python.

**Zero dependencies.** Uses only the Python standard library. Works on any Python 3.9+.

## Install

```bash
pip install calibration-scope
```

Or for local development:
```bash
cd calibration-scope
pip install -e .
```

## Quick start

```python
from calibration_scope import Client

# Connect to a running Calibration Scope dashboard
client = Client()  # defaults to http://127.0.0.1:8768

# Health check
print(client.status())  # {'status': 'ok'}

# Leaderboard — champions, squad, rankings
lb = client.leaderboard()
for champ in lb.get("champions", []):
    print(f"{champ['key']}: {champ['axes_passing']}/4 axes")

# Signal/Carrier split — models AND humans, same shape
sc = client.signal_carrier(min_forms=2)
for row in sc["rows"]:
    print(f"{row['subject_name']}: signal={row['signal_score']:.2f} "
          f"carrier_var={row['carrier_variance']}")

# Get a specific run's sealed results
run = client.get_run(932)
print(f"Run {run['id']}: {run['pass_count']}/{run['total_count']} — {run['sha3_provenance']}")

# Capability router — which model to dispatch per axis
plan = client.router_plan()
for axis in plan["axes"]:
    if axis["primary"]:
        print(f"{axis['axis']}: {axis['primary']['model_key']} "
              f"({axis['primary']['reason']})")

# Test registry
tests = client.tests(axis="reasoning")
for t in tests:
    print(f"{t['name']} [{t['owl_type']}] — {t.get('formal_spec', '?')}")

client.close()
```

## Context manager

```python
with Client() as client:
    runs = client.list_runs(limit=10)
```

## What this client does NOT do

- **Never runs tests.** This is a read-only data consumer.
- **Never calls an LLM.** No inference, no API keys needed.
- **Never writes.** No mutations to the database or dashboard state.

All data is sealed with SHA-3 provenance by the backend. This client passes it through unchanged — if you need to verify a seal, compare the `sha3_provenance` field against a recomputed hash of the trial verdicts.

## License

BUSL-1.1 — see the [main repository](https://github.com/IT-Help-San-Diego/calibration-scope) for details.
