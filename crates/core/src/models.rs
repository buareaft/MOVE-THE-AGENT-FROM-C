use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub path: String,
    pub bytes: u64,
    pub allocated: u64,
    pub category: String,
    pub sessions: usize,
    pub updated: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub mode: String,
    pub root: String,
    pub scanned_at: String,
    pub total_bytes: u64,
    pub allocated_bytes: u64,
    pub file_count: usize,
    pub exclusive_bytes: u64,
    pub shared_bytes: u64,
    pub project_bytes: u64,
    pub unknown_bytes: u64,
    pub backup_bytes: u64,
    pub agents: Vec<Entity>,
    pub projects: Vec<Entity>,
    pub sessions: Vec<Entity>,
    pub jobs: Vec<Job>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub volumes: Vec<DriveUsage>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveUsage {
    pub volume: String,
    pub bytes: u64,
    pub allocated_bytes: u64,
    pub files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub agent: String,
    pub source: String,
    pub destination: String,
    pub backup: String,
    pub status: String,
    pub bytes: u64,
    pub created_at: String,
    pub message: String,
    pub hashes: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub agent: String,
    pub source: String,
    pub destination: String,
    pub bytes: u64,
    pub file_count: usize,
    pub steps: Vec<String>,
    pub warning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String,
    pub files: usize,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub allocated: u64,
    pub identity: String,
    pub category: String,
    pub owner: String,
    pub agent: String,
    pub volume: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Entity>,
    pub total: usize,
}

#[derive(Serialize)]
pub struct Breakdown {
    pub label: String,
    pub bytes: u64,
    pub files: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub windows_supported: bool,
    pub data_mode: String,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    #[serde(flatten)]
    pub snapshot: Snapshot,
    pub project_count: usize,
    pub session_count: usize,
}
impl From<Snapshot> for Dashboard {
    fn from(mut snapshot: Snapshot) -> Self {
        let project_count = snapshot.projects.len();
        let session_count = snapshot.sessions.len();
        snapshot.projects.truncate(8);
        snapshot.sessions.clear();
        Self {
            snapshot,
            project_count,
            session_count,
        }
    }
}

pub(crate) fn timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}
