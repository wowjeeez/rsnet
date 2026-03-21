mod vendor;
pub mod glue;

pub use glue::error::TsNetError;
pub use glue::stream::TailscaleStream;
pub use glue::listener::Listener;
pub use glue::localapi::LocalClient;
pub use glue::server::RawTsTcpServer;

#[cfg(feature = "localapi-serde-json")]
pub use glue::types::{
    AppConnectorPrefs, AutoUpdatePrefs, ClientVersion, ExitNodeStatus,
    Location, Node, PeerStatus, Prefs, Status, TailnetStatus,
    UserProfile, WhoIsResponse,
};
