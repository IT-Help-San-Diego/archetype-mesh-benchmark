//! POST /api/cloud-keys — store cloud API keys securely in a local secrets file.
//! GET /api/cloud-keys — return which providers are configured (never the keys themselves).
//! DELETE /api/cloud-keys/{provider} — remove a provider's key.
//!
//! Keys are stored in ~/.calibration-scope/cloud-keys.json (chmod 600), never
//! in the database, never in git, never logged. The dashboard can set them
//! up without touching the shell or launchd plist.
use axum::extract::Path;
use axum::response::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

use crate::error::{AppError, AppResult};

const SECRETS_DIR: &str = ".calibration-scope";
const SECRETS_FILE: &str = "cloud-keys.json";

fn secrets_path() -> std::path::PathBuf {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"));
    home.join(SECRETS_DIR).join(SECRETS_FILE)
}

#[derive(Serialize, Deserialize, Default)]
struct KeyStore {
    keys: HashMap<String, String>,
}

fn read_keys() -> AppResult<KeyStore> {
    let path = secrets_path();
    if !path.exists() {
        return Ok(KeyStore::default());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| AppError::Executor(format!("Cannot read {}: {}", path.display(), e)))?;
    serde_json::from_str(&raw)
        .map_err(|e| AppError::Executor(format!("Parse error in {}: {}", path.display(), e)))
        .or(Ok(KeyStore::default()))
}

fn write_keys(store: &KeyStore) -> AppResult<()> {
    let path = secrets_path();
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)
        .map_err(|e| AppError::Executor(format!("Cannot create {}: {}", dir.display(), e)))?;

    let json = serde_json::to_string_pretty(store)
        .map_err(|e| AppError::Executor(format!("Serialize error: {}", e)))?;

    // Write with 0600 permissions (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| AppError::Executor(format!("Cannot create {}: {}", path.display(), e)))?;
        use std::io::Write;
        file.write_all(json.as_bytes())
            .map_err(|e| AppError::Executor(format!("Write error: {}", e)))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&path, json)
            .map_err(|e| AppError::Executor(format!("Write error: {}", e)))?;
    }

    tracing::info!("Cloud keys updated: {} provider(s)", store.keys.len());
    Ok(())
}

/// Supported cloud providers for the benchmark.
pub const SUPPORTED_PROVIDERS: &[&str] = &["nous", "openrouter", "openai", "gemini"];

#[derive(Deserialize)]
pub struct SetKeyRequest {
    pub key: String,
}

#[derive(Serialize)]
pub struct KeyStatus {
    pub provider: String,
    pub configured: bool,
    pub key_preview: String, // first 8 chars + "…" — never the full key
}

#[derive(Serialize)]
pub struct KeyListResponse {
    pub providers: Vec<KeyStatus>,
    pub secrets_path: String,
}

pub async fn list_keys() -> AppResult<Json<KeyListResponse>> {
    let store = read_keys()?;
    let providers: Vec<KeyStatus> = SUPPORTED_PROVIDERS
        .iter()
        .map(|p| {
            let key = store.keys.get(*p);
            let preview = key
                .map(|k| {
                    if k.len() > 8 {
                        format!("{}…", &k[..8])
                    } else {
                        "…".to_string()
                    }
                })
                .unwrap_or_default();
            KeyStatus {
                provider: p.to_string(),
                configured: key.is_some(),
                key_preview: preview,
            }
        })
        .collect();

    Ok(Json(KeyListResponse {
        providers,
        secrets_path: secrets_path().to_string_lossy().to_string(),
    }))
}

pub async fn set_key(
    Path(provider): Path<String>,
    Json(req): Json<SetKeyRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !SUPPORTED_PROVIDERS.contains(&provider.as_str()) {
        return Err(AppError::Executor(format!(
            "Unsupported provider: {} (supported: {})",
            provider,
            SUPPORTED_PROVIDERS.join(", ")
        )));
    }

    if req.key.is_empty() {
        return Err(AppError::Executor("Key cannot be empty".into()));
    }

    let mut store = read_keys()?;
    store.keys.insert(provider.clone(), req.key.clone());
    write_keys(&store)?;

    // Also set it in the environment for the current process so cloud runs
    // work immediately without a restart.
    let env_var = match provider.as_str() {
        "nous" => "NOUS_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        _ => unreachable!(),
    };
    std::env::set_var(env_var, &req.key);

    let preview = if req.key.len() > 8 {
        format!("{}…", &req.key[..8])
    } else {
        "…".to_string()
    };

    Ok(Json(serde_json::json!({
        "provider": provider,
        "configured": true,
        "key_preview": preview,
        "message": format!("{} key stored — cloud runs are now available", provider),
    })))
}

pub async fn delete_key(
    Path(provider): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let mut store = read_keys()?;
    store.keys.remove(&provider);
    write_keys(&store)?;

    let env_var = match provider.as_str() {
        "nous" => "NOUS_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        _ => &provider,
    };
    std::env::remove_var(env_var);

    Ok(Json(serde_json::json!({
        "provider": provider,
        "configured": false,
        "message": format!("{} key removed", provider),
    })))
}

/// Load keys from the secrets file into the environment at startup.
/// Called from main() so cloud runs work without manual env setup.
pub fn load_keys_to_env() {
    if let Ok(store) = read_keys() {
        for (provider, key) in &store.keys {
            let env_var = match provider.as_str() {
                "nous" => "NOUS_API_KEY",
                "openrouter" => "OPENROUTER_API_KEY",
                "openai" => "OPENAI_API_KEY",
                "gemini" => "GEMINI_API_KEY",
                _ => continue,
            };
            std::env::set_var(env_var, key);
            tracing::info!("Loaded {} key from secrets file", provider);
        }
    }
}
