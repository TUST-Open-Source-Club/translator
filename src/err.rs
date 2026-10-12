use thiserror::Error;

/// Application error kinds; each maps to a distinct exit code.
#[derive(Debug, Error)]
pub enum AppError {
    /// 1: generic runtime failures (inference, I/O, ...); also the fallback for uncoded errors
    #[allow(dead_code)]
    #[error("{0}")]
    Runtime(String),
    /// 2: bad usage (invalid args, no input, image region out of bounds, ...)
    #[error("{0}")]
    Usage(String),
    /// 3: model problems (missing explicit path, load failure, ...)
    #[error("{0}")]
    Model(String),
    /// 4: download failures
    #[error("{0}")]
    Download(String),
    /// 5: feature not supported by the selected model
    #[error("{0}")]
    Unsupported(String),
}

impl AppError {
    pub fn code(&self) -> u8 {
        match self {
            AppError::Runtime(_) => 1,
            AppError::Usage(_) => 2,
            AppError::Model(_) => 3,
            AppError::Download(_) => 4,
            AppError::Unsupported(_) => 5,
        }
    }
}

pub fn usage(msg: impl Into<String>) -> anyhow::Error {
    AppError::Usage(msg.into()).into()
}

pub fn model(msg: impl Into<String>) -> anyhow::Error {
    AppError::Model(msg.into()).into()
}

pub fn unsupported(msg: impl Into<String>) -> anyhow::Error {
    AppError::Unsupported(msg.into()).into()
}

/// Wrap an error as the given kind, keeping it if it already is an AppError.
pub fn or_kind(e: anyhow::Error, kind: fn(String) -> AppError) -> anyhow::Error {
    if e.downcast_ref::<AppError>().is_some() {
        e
    } else {
        anyhow::Error::new(kind(format!("{e:#}")))
    }
}

/// Exit code for a top-level error: AppError carries its own, anything else is a runtime error.
pub fn exit_code(e: &anyhow::Error) -> u8 {
    e.downcast_ref::<AppError>()
        .map(AppError::code)
        .unwrap_or(1)
}
