use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModelEntry {
    pub key: String,
    pub name: String,
    pub provider: String,
    pub kind: String,
    pub vision: i32,
    pub tools: i32,
    pub local_path: String,
}
