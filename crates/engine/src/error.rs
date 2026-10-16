use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now")]
    Disabled(String),
    #[error("invalid argument for `{command}`: {reason}")]
    InvalidArgument { command: String, reason: String },
}
