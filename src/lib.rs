mod vendor;
pub mod glue;

pub use glue::error::TsNetError;
pub use glue::stream::TailscaleStream;
pub use glue::listener::Listener;
pub use glue::localapi::LocalClient;
#[cfg(feature = "ssl")]
pub use glue::tls::{TlsListener, tls_config_from_pem};
pub use glue::server::RawTsTcpServer;

#[cfg(feature = "localapi-serde-json")]
pub use glue::localapi::{
    PeerStatus, Status, TailnetStatus, UserProfile, WhoIsResponse,
};
