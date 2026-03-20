use std::io;

#[derive(Debug)]
pub enum TsNetError {
    Io(io::Error),
    Tailscale(String),
}

impl std::fmt::Display for TsNetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TsNetError::Io(e) => write!(f, "{e}"),
            TsNetError::Tailscale(msg) => write!(f, "tailscale error: {msg}"),
        }
    }
}

impl std::error::Error for TsNetError {}

impl From<io::Error> for TsNetError {
    fn from(e: io::Error) -> Self {
        TsNetError::Io(e)
    }
}
