use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("subscription parse error: {0}")]
    SubscriptionParse(String),

    #[error("unsupported proxy type: {0}")]
    UnsupportedProxyType(String),

    #[error("invalid proxy entry '{name}': {reason}")]
    InvalidProxy { name: String, reason: String },

    #[error("empty subscription content")]
    EmptySubscription,

    #[error("no proxies found in subscription")]
    NoProxies,

    #[error("fetch failed: {0}")]
    Fetch(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("item not found: {0}")]
    NotFound(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("core error: {0}")]
    Core(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}
