"""Calibration Scope read-only HTTP client.

A thin wrapper over the dashboard's REST API that returns plain Python dicts.
No model is ever called, no test is ever run — this is a data consumer.

Uses only the standard library (urllib) — zero dependencies, works on any
Python 3.9+. All data is sealed with SHA-3 provenance by the backend; this
client passes it through unchanged.
"""
from __future__ import annotations

import json
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Optional

DEFAULT_URL = "http://127.0.0.1:8768"


class Client:
    """Read-only client for a running Calibration Scope instance.

    Args:
        base_url: The dashboard URL (default: http://127.0.0.1:8768).
        timeout: Request timeout in seconds (default: 30).
    """

    def __init__(self, base_url: str = DEFAULT_URL, timeout: float = 30.0):
        self.base_url = base_url.rstrip("/")
        self._timeout = timeout

    def _get(self, path: str, **params: Any) -> Any:
        """GET a JSON endpoint, raise on error."""
        url = f"{self.base_url}{path}"
        if params:
            clean = {k: v for k, v in params.items() if v is not None}
            if clean:
                url += "?" + urllib.parse.urlencode(clean)
        try:
            req = urllib.request.Request(url, headers={"Accept": "application/json"})
            with urllib.request.urlopen(req, timeout=self._timeout) as resp:
                body = resp.read()
                ct = resp.headers.get("content-type", "")
                if "json" not in ct:
                    # Some endpoints (e.g. /api/status) return plain text.
                    return {"status": body.decode("utf-8", errors="replace")}
                return json.loads(body)
        except urllib.error.HTTPError as e:
            body = e.read().decode("utf-8", errors="replace")[:200]
            raise RuntimeError(
                f"API error {e.code} from {path}: {body}"
            ) from e
        except urllib.error.URLError as e:
            raise ConnectionError(
                f"Cannot reach Calibration Scope at {self.base_url}. "
                f"Is the dashboard running? (Error: {e})"
            ) from e

    def status(self) -> Any:
        """Health check — returns backend status including DB and LM Studio state."""
        return self._get("/api/status")

    def models(self) -> Any:
        """List all models in the registry with their verdicts and metadata."""
        return self._get("/api/models")

    def leaderboard(self) -> Any:
        """The loot board — champions (4-axis pass), squad, and rankings."""
        return self._get("/api/loot")

    def get_run(self, run_id: int) -> Any:
        """Full details of a specific benchmark run, including trial-level results."""
        return self._get(f"/api/runs/{run_id}")

    def list_runs(self, limit: int = 50, offset: int = 0) -> Any:
        """Recent benchmark runs."""
        return self._get("/api/runs", limit=limit, offset=offset)

    def signal_carrier(
        self,
        model_key: Optional[str] = None,
        axis: Optional[str] = None,
        min_forms: int = 1,
    ) -> Any:
        """Signal/Carrier split for models AND human participants.

        This is the core of the human-calibration feature: both subjects land
        in the same shape, comparable directly.

        Args:
            model_key: Filter to one model (optional).
            axis: Filter to one axis like 'reasoning' (optional).
            min_forms: Minimum surface forms attempted (pass 2 to see only
                      rows where carrier_variance is measurable).
        """
        return self._get(
            "/api/signal-carrier",
            model_key=model_key,
            axis=axis,
            min_forms=min_forms,
        )

    def router_plan(
        self, min_trials: int = 3, fallback_threshold: float = 0.8
    ) -> Any:
        """The capability router — which model to dispatch to per axis, with evidence."""
        return self._get(
            "/api/router/plan",
            min_trials=min_trials,
            fallback_threshold=fallback_threshold,
        )

    def tests(self, axis: Optional[str] = None) -> Any:
        """The test registry — all tests with formal_spec, owl_type, and ground truth."""
        return self._get("/api/tests", axis=axis)

    def close(self):
        """No-op (urllib has no persistent connection to close)."""
        pass

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()
