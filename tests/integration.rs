use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

mod common;

#[tokio::test]
async fn test_status_returns_ok() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], b"ok");
}

#[tokio::test]
async fn test_summary_returns_json() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/summary").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_array());
    assert!(!json.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_models_returns_json() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/models").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_array());
}

#[tokio::test]
async fn test_index_returns_html() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Write-path validation (no inference is ever triggered: every request
//    below is designed to be rejected before an executor task spawns) ──────

fn json_post(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn json_put(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8_lossy(&bytes).to_string()
}

#[tokio::test]
async fn start_runs_rejects_empty_axes() {
    let app = common::test_app().await;
    let response = app
        .oneshot(json_post("/api/runs", r#"{"model_key":"x","axes":[]}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_string(response).await.contains("non-empty"));
}

#[tokio::test]
async fn start_runs_rejects_invalid_axis_with_actionable_message() {
    let app = common::test_app().await;
    let response = app
        .oneshot(json_post("/api/runs", r#"{"model_key":"x","axes":["magic"]}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_string(response).await;
    assert!(body.contains("Invalid axis: magic"), "error must name the bad axis, got: {}", body);
}

#[tokio::test]
async fn start_runs_rejects_unknown_model() {
    let app = common::test_app().await;
    let response = app
        .oneshot(json_post(
            "/api/runs",
            r#"{"model_key":"no-such-model-xyz","axes":["reasoning"]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_string(response).await.contains("Unknown model key"));
}

// Capability pre-flight (found + fixed 2026-07-08): a vision-axis request
// against a model with supports_vision=false must be silently skipped, not
// sent to the executor to burn a real clean-room load + guaranteed-reject
// inference cycle. Uses a real registered non-vision model already in the
// live DB (ibm/granite-3.2-8b) rather than a fixture, since the guard reads
// live model metadata.
#[tokio::test]
async fn start_runs_skips_vision_axis_for_non_vision_model() {
    let app = common::test_app().await;
    let response = app
        .oneshot(json_post(
            "/api/runs",
            r#"{"model_key":"ibm/granite-3.2-8b","axes":["vision"]}"#,
        ))
        .await
        .unwrap();
    // Every requested axis was skipped -> no run to start -> a clean error,
    // not a silently-created queued row that's guaranteed to fail.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_string(response).await;
    assert!(
        body.contains("no vision support"),
        "must name why vision was skipped, got: {}",
        body
    );
}

#[tokio::test]
async fn create_test_rejects_answer_leakage() {
    let app = common::test_app().await;
    let response = app
        .oneshot(json_post(
            "/api/tests",
            r#"{"name":"IT-LEAK","axis":"reasoning","prompt_text":"The answer is Canberra. What is the capital?","expected_result":"Canberra"}"#,
        ))
        .await
        .unwrap();
    // Anti-cheating invariant: a test containing its own answer must never persist.
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("Answer leakage"), "leakage guard must fire, got: {}", body);
}

#[tokio::test]
async fn create_test_rejects_missing_ground_truth() {
    let app = common::test_app().await;
    let response = app
        .oneshot(json_post(
            "/api/tests",
            r#"{"name":"IT-NOGT","axis":"reasoning","prompt_text":"What is 2+2?"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_string(response).await.contains("expected_result"));
}

#[tokio::test]
async fn create_test_rejects_invalid_scoring_method() {
    let app = common::test_app().await;
    let response = app
        .oneshot(json_post(
            "/api/tests",
            r#"{"name":"IT-BADSCORE","axis":"reasoning","prompt_text":"What is 2+2?","expected_result":"4","scoring_method":"vibes"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_string(response).await.contains("Invalid scoring_method"));
}

#[tokio::test]
async fn update_test_rejects_unknown_id() {
    let app = common::test_app().await;
    let response = app
        .oneshot(json_put("/api/tests/999999", r#"{"name":"x"}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_string(response).await.contains("No test with id 999999"));
}

#[tokio::test]
async fn tests_list_is_blind_by_default() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/tests").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Anti-cheating invariant: the default list view must never carry ground truth.
    for t in json["tests"].as_array().unwrap() {
        assert!(t.get("expected_result").is_none(), "blind list leaked ground truth: {}", t);
        assert!(t.get("prompt_text").is_none(), "blind list leaked prompt: {}", t);
    }
}

#[tokio::test]
async fn loot_has_contract_shape() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/loot").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.get("leaderboard").is_some());
    assert!(json.get("recommended_squad").is_some());
    assert!(json.get("missing_axes").is_some());
}

// Regression test for the bug found live 2026-07-08: a model with a 100%
// HARD FAIL on any core axis (vision/tools/reasoning/security) must never
// outrank a model with zero hard fails, no matter how much faster/more-won
// the failing model is on the axes it CAN do. Asserted directly against the
// leaderboard shape rather than re-deriving overall_score's formula here —
// if the fix regresses, this catches the RESULT (wrong rank), not just a
// changed constant.
#[tokio::test]
async fn leaderboard_never_ranks_a_hard_failer_above_a_clean_model() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/loot").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    let leaderboard = json["leaderboard"].as_array().unwrap();

    for a in leaderboard {
        for b in leaderboard {
            let a_hard_fails = a["hard_fails"].as_i64().unwrap_or(0);
            let b_hard_fails = b["hard_fails"].as_i64().unwrap_or(0);
            let a_score = a["overall_score"].as_f64().unwrap();
            let b_score = b["overall_score"].as_f64().unwrap();
            if a_hard_fails > b_hard_fails {
                assert!(
                    a_score < b_score,
                    "{} has MORE hard fails ({}) than {} ({}) but scored >= it ({} vs {}) — the leaderboard bug is back",
                    a["model_key"], a_hard_fails, b["model_key"], b_hard_fails, a_score, b_score
                );
            }
        }
    }
}

// ── Abort endpoint (self-harm audit follow-up, 2026-07-08) ─────────────────
// The end-to-end "actually cancels real inference" behavior is proven live
// against the real LM Studio process (see the session's manual verification:
// killing a streaming client dropped the llmworker process's CPU from 11.2%
// to 0.1% within 1s). What the integration suite CAN verify without spinning
// up real inference is the HTTP contract: idempotent, never an error for a
// run that isn't in flight (asking to stop something already stopped is not
// a mistake to punish).

#[tokio::test]
async fn abort_run_is_a_clean_noop_for_unknown_run_id() {
    let app = common::test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs/999999/abort")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["run_id"], 999999);
    assert_eq!(json["aborted"], false, "aborting a non-existent run must not claim success");
}

#[tokio::test]
async fn abort_run_is_idempotent_when_called_twice() {
    let app = common::test_app().await;
    // Same run_id, two abort calls in a row — neither should error, and
    // neither claims to have signaled a run that was never registered.
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/runs/424242/abort")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        assert_eq!(json["aborted"], false);
    }
}

// ── Capability router (/api/router/plan, 2026-07-08) ───────────────────────
// The router is a pure decision function over the same evidence substrate as
// the leaderboard. These tests pin its POLICY INVARIANTS against real data:
// whatever the DB currently contains, the plan must be internally consistent.

#[tokio::test]
async fn router_plan_returns_all_axes_with_policy_echo() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/router/plan").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();

    // Policy must be echoed — a plan that doesn't state its own rules is
    // unauditable.
    assert_eq!(json["policy"]["min_trials"], 3);
    assert_eq!(json["policy"]["fallback_threshold"], 0.8);

    let axes = json["axes"].as_array().unwrap();
    let names: Vec<&str> = axes.iter().map(|a| a["axis"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["vision", "tools", "reasoning", "security", "auxiliary"]);
}

#[tokio::test]
async fn router_primary_is_always_perfect_and_sufficiently_evidenced() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/router/plan").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();

    for axis in json["axes"].as_array().unwrap() {
        if let Some(primary) = axis["primary"].as_object() {
            let rate = primary["pass_rate"].as_f64().unwrap();
            let trials = primary["total_trials"].as_i64().unwrap();
            assert!(
                (rate - 1.0).abs() < f64::EPSILON,
                "axis {} primary {} has pass_rate {} — a primary must be 100%",
                axis["axis"], primary["model_key"], rate
            );
            assert!(
                trials >= 3,
                "axis {} primary {} has only {} trials — under the min_trials floor",
                axis["axis"], primary["model_key"], trials
            );
            // Every placement must carry sealed evidence.
            assert!(
                primary["evidence"]["run_id"].is_i64(),
                "primary without an evidence run_id is an unaudited claim"
            );
            // A routed axis must say so.
            assert_eq!(axis["status"], "routed");
        }
    }
}

#[tokio::test]
async fn router_never_excludes_a_model_above_threshold_or_routes_one_below() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/router/plan").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();

    for axis in json["axes"].as_array().unwrap() {
        for excluded in axis["excluded"].as_array().unwrap() {
            let rate = excluded["pass_rate"].as_f64().unwrap();
            assert!(
                rate < 0.8,
                "axis {} excluded {} at pass_rate {} — exclusion above the fallback floor",
                axis["axis"], excluded["model_key"], rate
            );
            // Silence is the old leaderboard bug: every exclusion states why.
            assert!(
                !excluded["reason"].as_str().unwrap().is_empty(),
                "exclusion without a reason"
            );
        }
        for fallback in axis["fallbacks"].as_array().unwrap() {
            let rate = fallback["pass_rate"].as_f64().unwrap();
            assert!(
                rate >= 0.8,
                "axis {} fallback {} at pass_rate {} — routable below the floor",
                axis["axis"], fallback["model_key"], rate
            );
        }
    }
}

#[tokio::test]
async fn router_plan_rejects_out_of_bounds_policy_knobs() {
    for uri in [
        "/api/router/plan?min_trials=0",
        "/api/router/plan?min_trials=1001",
        "/api/router/plan?fallback_threshold=0.0",
        "/api/router/plan?fallback_threshold=1.5",
        "/api/router/plan?location=orbit",
    ] {
        let app = common::test_app().await;
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{} should be a 400, not silently clamped",
            uri
        );
    }
}

#[tokio::test]
async fn router_location_filter_actually_filters() {
    let app = common::test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/router/plan?location=local")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    for axis in json["axes"].as_array().unwrap() {
        let mut everyone: Vec<&serde_json::Value> = Vec::new();
        if axis["primary"].is_object() {
            everyone.push(&axis["primary"]);
        }
        everyone.extend(axis["fallbacks"].as_array().unwrap());
        everyone.extend(axis["excluded"].as_array().unwrap());
        for m in everyone {
            assert_eq!(
                m["location"], "local",
                "location=local plan contains cloud model {}",
                m["model_key"]
            );
        }
    }
}

// ── Host reality check (/api/host/reality, 2026-07-08) ─────────────────────
// The Setup tab's step 0. Contract: every numeric claim carries a `source`
// receipt, unmeasurable values are null (never invented), and the budget
// block is labeled a heuristic with its formula exposed.

#[tokio::test]
async fn host_reality_measures_with_receipts_and_labeled_heuristic() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/host/reality").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();

    // Receipts: every Measured field must carry its source command.
    for (section, field) in [
        ("hardware", "total_ram_gb"),
        ("hardware", "cpu_cores"),
        ("memory", "free_pct"),
        ("memory", "gpu_ceiling_gb"),
        ("disk", "free_gb"),
    ] {
        let src = json[section][field]["source"].as_str().unwrap_or("");
        assert!(
            !src.is_empty(),
            "{}.{} has no measurement receipt — numbers without sources are assertions",
            section, field
        );
    }

    // On the machine running this suite, RAM must actually measure (sysctl is
    // always present on macOS); a null here means the measurement path broke.
    let ram = json["hardware"]["total_ram_gb"]["value"].as_f64();
    assert!(ram.is_some() && ram.unwrap() > 0.0, "hw.memsize failed to measure");

    // The budget must self-identify as heuristic and show its formula —
    // it is derived, not measured, and must never masquerade.
    assert_eq!(json["budget"]["kind"], "heuristic");
    assert!(json["budget"]["formula"].as_str().unwrap().contains("clamp"));

    // Internal consistency: ai_budget can never exceed total RAM minus the
    // life reserve (the formula's own upper bound).
    let life = json["budget"]["life_reserve_gb"].as_f64().unwrap();
    let ai = json["budget"]["ai_budget_gb"].as_f64().unwrap();
    assert!(
        ai <= ram.unwrap() - life + 0.001,
        "ai_budget {} exceeds total {} - life_reserve {}",
        ai, ram.unwrap(), life
    );

    // LM Studio block always reports reachability honestly (bool, either way).
    assert!(json["lmstudio"]["reachable"].is_boolean());
}

// ── Hermes config check (/api/hermes/reality, 2026-07-08) ──────────────────
// Reads ~/.hermes/config.yaml via a strict allowlist. The security contract
// is the whole point: no response value may ever look like a credential.

#[tokio::test]
async fn hermes_reality_is_allowlisted_and_never_leaks_credentials() {
    let app = common::test_app().await;
    let response = app
        .oneshot(Request::builder().uri("/api/hermes/reality").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Shape contract: only the allowlisted top-level keys may appear.
    let allowed = ["config_found", "parse_ok", "source", "main", "approvals_mode", "auxiliary"];
    for key in json.as_object().unwrap().keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "unexpected top-level key '{}' in hermes_reality response — allowlist violated",
            key
        );
    }

    // Credential scan on the RAW BODY: no long spaceless token-like strings.
    // Model IDs stay under this bound; sk-/AKIA/ghp_ style keys do not.
    for value in body.split('"') {
        assert!(
            !(value.len() > 48 && !value.contains(' ') && !value.contains('/')),
            "response contains a credential-shaped value ({} chars)",
            value.len()
        );
    }

    // On this machine a config exists; if found+parsed, main model must be
    // non-empty strings (the fields the Setup tab verifies against).
    if json["config_found"] == true && json["parse_ok"] == true {
        assert!(json["main"]["model"].as_str().map(|s| !s.is_empty()).unwrap_or(false));
        // Auxiliary slots present, each with the honest is_auto flag.
        let aux = json["auxiliary"].as_array().unwrap();
        assert_eq!(aux.len(), 4);
        for s in aux {
            assert!(s["is_auto"].is_boolean());
        }
    }
}
