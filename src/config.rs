use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub dashboard_path: PathBuf,
    pub assets_dir: PathBuf,
    pub listen_addr: String,
    pub listen_port: u16,
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

        Config {
            database_url,
            dashboard_path,
            assets_dir,
            listen_addr,
            listen_port,
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.listen_addr, self.listen_port)
    }
}
