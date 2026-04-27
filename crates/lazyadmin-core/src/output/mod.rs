use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct MessageOutput {
    pub message: String,
}
