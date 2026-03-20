mod vendor;
pub mod glue;

pub use glue::error::TsNetError;
pub use glue::stream::TailscaleStream;
pub use glue::listener::Listener;
pub use glue::localapi::LocalClient;
pub use glue::server::RawTsTcpServer;
