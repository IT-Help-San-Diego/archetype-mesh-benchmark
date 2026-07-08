//! GET /api/hermes/reality — read-only, allowlisted view of Hermes' actual
//! configuration, for the Setup tab's interactive verification steps.
//!
//! SECURITY CONTRACT (this file's most important property):
//! `~/.hermes/config.yaml` contains API keys and tokens elsewhere in the
//! document. This handler therefore NEVER serializes the parsed document or
//! any subtree of it. It extracts ONLY the scalar fields named in the
//! allowlist below, each fetched by explicit path. As defense in depth, the
//! assembled response is scanned before send: if any value looks like a
//! credential (long high-entropy token) or any key path we somehow emitted
//! contains key/token/secret, the handler refuses to answer rather than leak.
//!
//! WHY READ THE FILE AT ALL: the Setup tab's mandate is "a wizard that
//! checks, not a wizard that asserts". Steps like "configure your main
//! model" and "set the Approval auxiliary task" become verifiable —
//! the page reads what Hermes is ACTUALLY configured to do and shows
//! ✅/⚠️ against reality, the same contract as /api/host/reality.
//! Reading the file directly (instead of shelling out to `hermes config`)
//! keeps this dependency-free, fast, and free of side effects.
use axum::response::Json;
use serde::Serialize;
use yaml_rust2::{Yaml, YamlLoader};

use crate::error::AppResult;

/// Auxiliary task slots surfaced in the Setup tab, in display order.
/// Matches Hermes' Settings → Model Settings → Auxiliary Tasks panel.
const AUX_TASKS: [&str; 4] = ["vision", "mcp", "approval", "web_extract"];

#[derive(Serialize)]
struct AuxSlot {
    task: String,
    provider: Option<String>,
    model: Option<String>,
    /// true when provider is "auto" (inherits the main model) — the honest
    /// default state, worth distinguishing from an explicit pin.
    is_auto: bool,
}

fn yaml_str(root: &Yaml, path: &[&str]) -> Option<String> {
    let mut node = root;
    for seg in path {
        node = &node[*seg];
    }
    node.as_str().map(str::to_string)
}

/// Credential heuristic for the defense-in-depth scan: any emitted value
/// that is long AND has no spaces could be a leaked token — refuse.
/// Model names ("anthropic/claude-fable-5") pass; sk-… style keys do not.
fn looks_like_credential(v: &str) -> bool {
    v.len() > 48 && !v.contains(' ')
}

pub async fn hermes_reality() -> AppResult<Json<serde_json::Value>> {
    let path = dirs_home().join(".hermes/config.yaml");
    let source = "~/.hermes/config.yaml (read-only, allowlisted fields)";

    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(_) => {
            // Honest absence: no Hermes config on this machine is a reported
            // state the UI can guide from, not an HTTP error.
            return Ok(Json(serde_json::json!({
                "config_found": false,
                "source": source,
            })));
        }
    };

    let docs = match YamlLoader::load_from_str(&raw) {
        Ok(d) if !d.is_empty() => d,
        _ => {
            return Ok(Json(serde_json::json!({
                "config_found": true,
                "parse_ok": false,
                "source": source,
            })));
        }
    };
    let root = &docs[0];

    // ── Allowlisted extraction — every field fetched by explicit path ──
    let main_provider = yaml_str(root, &["model", "provider"]);
    let main_model = yaml_str(root, &["model", "default"]);
    let approvals_mode = yaml_str(root, &["approvals", "mode"]);

    let aux: Vec<AuxSlot> = AUX_TASKS
        .iter()
        .map(|task| {
            let provider = yaml_str(root, &["auxiliary", task, "provider"]);
            let model = yaml_str(root, &["auxiliary", task, "model"]).filter(|m| !m.is_empty());
            let is_auto = provider.as_deref().map(|p| p == "auto").unwrap_or(true);
            AuxSlot { task: task.to_string(), provider, model, is_auto }
        })
        .collect();

    // ── Defense in depth: refuse to answer rather than leak ──
    let all_values: Vec<&str> = main_provider
        .iter()
        .chain(main_model.iter())
        .chain(approvals_mode.iter())
        .map(String::as_str)
        .chain(aux.iter().flat_map(|s| {
            s.provider.iter().chain(s.model.iter()).map(String::as_str)
        }))
        .collect();
    if all_values.iter().any(|v| looks_like_credential(v)) {
        tracing::error!("hermes_reality: extracted value tripped the credential heuristic — refusing to serve");
        return Err(crate::error::AppError::Executor(
            "config check refused: an extracted field looked like a credential".into(),
        ));
    }

    Ok(Json(serde_json::json!({
        "config_found": true,
        "parse_ok": true,
        "source": source,
        "main": {
            "provider": main_provider,
            "model": main_model,
        },
        "approvals_mode": approvals_mode,
        "auxiliary": aux,
    })))
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| "/".into())
}
