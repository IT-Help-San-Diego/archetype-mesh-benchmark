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
