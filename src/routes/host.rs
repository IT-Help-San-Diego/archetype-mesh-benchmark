//! GET /api/host/reality — measured host facts for the Setup reality-check.
//!
//! DESIGN CONTRACT (user mandate 2026-07-08): the Setup tab must not be a
//! wizard that asserts things — it must be a wizard that MEASURES things and
//! gets the operator to accept reality before configuring anything. So this
//! endpoint returns nothing but measurements taken at request time, each one
//! carrying the exact command it came from (`source`) so every number on the
//! page has a receipt. No cached values, no hardcoded assumptions about the
//! machine, no "typical system" numbers presented as facts.
//!
//! Every field is Option: a check that can't run reports null and the UI says
//! "couldn't measure" — partial failure is honest, invented data is not.
//!
//! THE RAM BUDGET MODEL (heuristic — labeled as such in the response):
//! Field reality from 27 years of bench work, formalized: in 2026 a 16–32GB
//! machine is "the system plus an app or two"; ~64GB is where normal
//! computing (system + the apps you actually leave open) lives comfortably;
//! everything ABOVE that line is what's genuinely available to an AI stack.
//! Two ceilings gate the AI budget and we report both:
//!   1. life_reserve_gb  = clamp(total/2, 16, 64) — the OS + apps living space
//!   2. gpu_ceiling_gb   — macOS caps GPU-wired memory (iogpu.wired_limit_mb;
//!      0 means the dynamic default, ~75% of RAM on Apple Silicon)
//!
//! ai_budget_gb = min(total − life_reserve, gpu_ceiling)
//!
//! The formula is exposed verbatim in the response so the UI can show its
//! work instead of asking to be believed.
use axum::extract::State;
use axum::response::Json;
use serde::Serialize;
use tokio::process::Command;

use crate::error::AppResult;
use crate::state::AppState;

/// One measured value + the command that produced it. The receipt travels
/// with the number.
#[derive(Serialize)]
struct Measured<T: Serialize> {
    value: Option<T>,
    source: &'static str,
}

#[derive(Serialize)]
struct LoadedModel {
    id: String,
    loaded_context_length: Option<i64>,
    quantization: Option<String>,
}

async fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

async fn sysctl_i64(key: &str) -> Option<i64> {
    run("/usr/sbin/sysctl", &["-n", key]).await?.parse().ok()
}

/// `df -k /` → (total_gb, free_gb). 1024-byte blocks per POSIX -k.
async fn disk_gb() -> Option<(f64, f64)> {
    let out = run("/bin/df", &["-k", "/"]).await?;
    let line = out.lines().nth(1)?;
    let f: Vec<&str> = line.split_whitespace().collect();
    let total_kb: f64 = f.get(1)?.parse().ok()?;
    let free_kb: f64 = f.get(3)?.parse().ok()?;
    Some((total_kb / 1048576.0, free_kb / 1048576.0))
}

/// `memory_pressure -Q` → system-wide free percentage.
async fn mem_free_pct() -> Option<i64> {
    let out = run("/usr/bin/memory_pressure", &["-Q"]).await?;
    out.lines()
        .find(|l| l.contains("free percentage"))?
        .split(':')
        .nth(1)?
        .trim()
        .trim_end_matches('%')
        .parse()
        .ok()
}

pub async fn host_reality(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    // Independent measurements — take them concurrently.
    let (memsize, cpu_cores, hw_model, iogpu_limit_mb, disk, free_pct) = tokio::join!(
        sysctl_i64("hw.memsize"),
        sysctl_i64("hw.ncpu"),
        run("/usr/sbin/sysctl", &["-n", "hw.model"]),
        sysctl_i64("iogpu.wired_limit_mb"),
        disk_gb(),
        mem_free_pct(),
    );

    let total_ram_gb = memsize.map(|b| b as f64 / 1073741824.0);

    // GPU-wired ceiling: explicit sysctl value wins; 0 means macOS' dynamic
    // default, ~75% of physical RAM on Apple Silicon (documented Apple
    // behavior; still labeled "estimated" in the response because we compute
    // it rather than read it).
    let (gpu_ceiling_gb, gpu_ceiling_note) = match (iogpu_limit_mb, total_ram_gb) {
        (Some(mb), _) if mb > 0 => (Some(mb as f64 / 1024.0), "explicit iogpu.wired_limit_mb"),
        (_, Some(ram)) => (Some(ram * 0.75), "estimated: dynamic default ≈75% of RAM (iogpu.wired_limit_mb=0)"),
        _ => (None, "unmeasurable"),
    };

    // The budget heuristic — labeled, never presented as a measurement.
    // DISCLOSURE (found by user skepticism, 2026-07-08: "a perfect 50/50
    // can't be real, can it?"): for any machine where 16 <= total/2 <= 64
    // (i.e. 32GB–128GB — most Macs), the clamp never engages and life
    // reserve is EXACTLY total/2, so the split is exactly 50/50. That's
    // correct arithmetic, but a clean 50/50 looks fake to a careful reader
    // — so the response now names WHICH constraint produced each number
    // (`binding`), and the UI prints it. Every output explains itself.
    let budget = total_ram_gb.map(|total| {
        let life_reserve = (total / 2.0).clamp(16.0, 64.0);
        let life_binding = if total / 2.0 < 16.0 {
            "16GB floor engaged: half your RAM would starve the OS, so the life reserve is held at 16GB"
        } else if total / 2.0 > 64.0 {
            "64GB cap engaged: normal computing doesn't grow past ~64GB, so every GB above 128GB total goes to the AI side"
        } else if (total - 128.0).abs() < 0.01 {
            "exactly at the crossover: total/2 = 64GB = the cap — the 50/50 split is exact arithmetic, not a rounding artifact"
        } else {
            "midpoint rule: 32GB–128GB machines split exactly 50/50 by construction (life = total/2, clamp inactive)"
        };
        let headroom = (total - life_reserve).max(0.0);
        let (ai_budget, ai_binding) = match gpu_ceiling_gb {
            Some(ceiling) if ceiling < headroom => (
                ceiling,
                "GPU-wired ceiling binds: RAM above what macOS lets the GPU wire can't hold model weights",
            ),
            Some(_) => (headroom, "headroom binds: total minus life reserve, under the GPU ceiling"),
            None => (headroom, "headroom only: GPU ceiling unmeasurable on this host"),
        };
        let tier = if total < 16.0 {
            "cloud-first: not enough RAM for meaningful local inference alongside a usable system"
        } else if total < 32.0 {
            "constrained: the system plus an app or two — one small quantized model at a time, expect trade-offs"
        } else if total < 64.0 {
            "entry local: normal computing fits, with room for a small model stack if you budget deliberately"
        } else if total < 96.0 {
            "comfortable: normal life reserved, a real single-model AI workload fits beside it"
        } else {
            "full stack: your normal computing keeps its full living space AND a serious multi-model AI stack fits above it"
        };
        serde_json::json!({
            "life_reserve_gb": life_reserve,
            "ai_budget_gb": ai_budget,
            "life_binding": life_binding,
            "ai_binding": ai_binding,
            "tier": tier,
            "kind": "heuristic",
            "formula": "life_reserve = clamp(total_ram/2, 16, 64); ai_budget = min(total_ram − life_reserve, gpu_ceiling)",
        })
    });

    // Live LM Studio state — same base URL the executor uses. Unreachable is
    // a reported state, not an error: the reality check's job is to show it.
    let lmstudio = match reqwest::Client::new()
        .get(format!("{}/api/v0/models", state.config.lmstudio_base_url))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(json) => {
                let data = json["data"].as_array().cloned().unwrap_or_default();
                let loaded: Vec<LoadedModel> = data
                    .iter()
                    .filter(|m| m["state"].as_str() == Some("loaded"))
                    .map(|m| LoadedModel {
                        id: m["id"].as_str().unwrap_or("?").to_string(),
                        loaded_context_length: m["loaded_context_length"].as_i64(),
                        quantization: m["quantization"].as_str().map(String::from),
                    })
                    .collect();
                serde_json::json!({
                    "reachable": true,
                    "base_url": state.config.lmstudio_base_url,
                    "models_on_disk": data.len(),
                    "loaded_now": loaded,
                    "source": "GET /api/v0/models (LM Studio REST)",
                })
            }
            Err(_) => serde_json::json!({ "reachable": false, "base_url": state.config.lmstudio_base_url, "note": "responded but body was not parseable" }),
        },
        _ => serde_json::json!({ "reachable": false, "base_url": state.config.lmstudio_base_url }),
    };

    Ok(Json(serde_json::json!({
        "measured_at": chrono::Utc::now().to_rfc3339(),
        "hardware": {
            "model": Measured { value: hw_model, source: "sysctl -n hw.model" },
            "total_ram_gb": Measured { value: total_ram_gb, source: "sysctl -n hw.memsize" },
            "cpu_cores": Measured { value: cpu_cores, source: "sysctl -n hw.ncpu" },
        },
        "memory": {
            "free_pct": Measured { value: free_pct, source: "memory_pressure -Q" },
            "gpu_ceiling_gb": Measured { value: gpu_ceiling_gb, source: "sysctl -n iogpu.wired_limit_mb" },
            "gpu_ceiling_note": gpu_ceiling_note,
        },
        "disk": {
            "total_gb": Measured { value: disk.map(|d| d.0), source: "df -k /" },
            "free_gb": Measured { value: disk.map(|d| d.1), source: "df -k /" },
        },
        "budget": budget,
        "lmstudio": lmstudio,
    })))
}
