use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub dashboard_path: PathBuf,
    pub assets_dir: PathBuf,
    pub project_root: PathBuf,
    pub listen_addr: String,
    pub listen_port: u16,
    /// LM Studio REST base (local executor). Override with LMSTUDIO_BASE_URL.
    pub lmstudio_base_url: String,
    /// Cloud API keys — read from env, never persisted. None = cloud runs refuse honestly.
    pub nous_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://REDACTED:REDACTED@localhost:5432/archetype_mesh".to_string()
        });

        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let assets_dir = project_root.join("assets");
        let dashboard_path = assets_dir.join("dashboard.html");

        let listen_addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1".to_string());
        let listen_port: u16 = std::env::var("LISTEN_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8768);

        let lmstudio_base_url = std::env::var("LMSTUDIO_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:1234".to_string());

        Config {
            database_url,
            dashboard_path,
            assets_dir,
            project_root,
            listen_addr,
            listen_port,
            lmstudio_base_url,
            nous_api_key: std::env::var("NOUS_API_KEY").ok().filter(|s| !s.is_empty()),
            openrouter_api_key: std::env::var("OPENROUTER_API_KEY").ok().filter(|s| !s.is_empty()),
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.listen_addr, self.listen_port)
    }
}
