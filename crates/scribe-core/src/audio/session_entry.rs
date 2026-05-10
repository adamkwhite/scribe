use std::path::PathBuf;
use std::time::SystemTime;

use super::session_status::SessionStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEntry {
    pub path: PathBuf,
    pub name: String,
    pub status: SessionStatus,
    pub modified: SystemTime,
}
