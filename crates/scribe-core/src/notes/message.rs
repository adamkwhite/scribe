use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(test, derive(Clone, Debug, PartialEq, Eq))]
pub(super) struct Message {
    pub(super) role: String,
    pub(super) content: String,
}
