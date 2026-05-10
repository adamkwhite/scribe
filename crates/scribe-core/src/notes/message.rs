use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct Message {
    pub(super) role: String,
    pub(super) content: String,
}
