mod vendor;
pub mod glue;

pub use glue::error::TsNetError;
pub use glue::stream::{StreamInfo, TailscaleStream};
pub use glue::listener::Listener;
pub use glue::localapi::LocalClient;
pub use glue::service::{Service, ServiceBuilder, ServiceMode};
pub use glue::server::RawTsTcpServer;

pub use glue::types::{
    AppConnectorPrefs, AutoUpdatePrefs, ClientVersion, ExitNodeStatus,
    Location, Node, PeerStatus, Prefs, Status, TailnetStatus,
    UserProfile, WhoIsResponse,
};
