use std::path::PathBuf;
use std::time::SystemTime;

use super::session_status::SessionStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub struct SessionEntry {
    pub path: PathBuf,
    pub name: String,
    pub status: SessionStatus,
    pub modified: SystemTime,
    #[cfg(feature = "tui")]
    pub recorded_at: Option<SystemTime>,
}
