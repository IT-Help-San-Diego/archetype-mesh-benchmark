"""Calibration Scope — read-only Python client.

Pull sealed benchmark results, leaderboard data, and signal-carrier splits
from a running Calibration Scope instance. Never runs tests, never writes —
a pure data consumer for researchers who want the evidence in Python.

Usage:
    from calibration_scope import Client

    client = Client()  # defaults to http://127.0.0.1:8768
    leaderboard = client.leaderboard()
    for model in leaderboard["champions"]:
        print(f"{model['key']}: {model['axes_passing']}/4 axes")

    # Get signal-carrier data (human + model, same shape)
    sc = client.signal_carrier(min_forms=2)
    for row in sc["rows"]:
        print(f"{row['subject_name']}: signal={row['signal_score']:.2f}")

    # Get a specific run's details
    run = client.get_run(932)
    print(f"Run {run['id']}: {run['pass_count']}/{run['total_count']} — {run['sha3_provenance']}")
"""
from .client import Client

__version__ = "0.1.0"
__all__ = ["Client"]
