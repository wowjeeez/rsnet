use std::collections::HashMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Status {
    pub version: Option<String>,
    #[serde(rename = "TUN")]
    pub tun: Option<bool>,
    pub backend_state: Option<String>,
    pub have_node_key: Option<bool>,
    #[serde(rename = "AuthURL")]
    pub auth_url: Option<String>,
    #[serde(rename = "TailscaleIPs")]
    pub tailscale_ips: Option<Vec<String>>,
    #[serde(rename = "Self")]
    pub self_node: PeerStatus,
    pub exit_node_status: Option<ExitNodeStatus>,
    pub health: Option<Vec<String>>,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: Option<String>,
    pub current_tailnet: Option<TailnetStatus>,
    pub cert_domains: Option<Vec<String>>,
    pub peer: Option<HashMap<String, PeerStatus>>,
    pub user: Option<HashMap<String, UserProfile>>,
    pub client_version: Option<ClientVersion>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PeerStatus {
    #[serde(rename = "ID")]
    pub id: Option<String>,
    pub public_key: Option<String>,
    pub host_name: Option<String>,
    #[serde(rename = "DNSName")]
    pub dns_name: Option<String>,
    #[serde(rename = "OS")]
    pub os: Option<String>,
    #[serde(rename = "UserID")]
    pub user_id: Option<i64>,
    #[serde(rename = "TailscaleIPs")]
    pub tailscale_ips: Option<Vec<String>>,
    pub allowed_ips: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub primary_routes: Option<Vec<String>>,
    pub addrs: Option<Vec<String>>,
    pub cur_addr: Option<String>,
    pub relay: Option<String>,
    pub peer_relay: Option<String>,
    pub rx_bytes: Option<i64>,
    pub tx_bytes: Option<i64>,
    pub created: Option<String>,
    pub last_write: Option<String>,
    pub last_seen: Option<String>,
    pub last_handshake: Option<String>,
    pub online: Option<bool>,
    pub exit_node: Option<bool>,
    pub exit_node_option: Option<bool>,
    pub active: Option<bool>,
    #[serde(rename = "PeerAPIURL")]
    pub peer_api_url: Option<Vec<String>>,
    pub capabilities: Option<Vec<String>>,
    pub cap_map: Option<HashMap<String, Vec<serde_json::Value>>>,
    #[serde(rename = "sshHostKeys")]
    pub ssh_host_keys: Option<Vec<String>>,
    pub sharee_node: Option<bool>,
    pub in_network_map: Option<bool>,
    pub in_magic_sock: Option<bool>,
    pub in_engine: Option<bool>,
    pub expired: Option<bool>,
    pub key_expiry: Option<String>,
    pub location: Option<Location>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TailnetStatus {
    pub name: Option<String>,
    #[serde(rename = "MagicDNSSuffix")]
    pub magic_dns_suffix: Option<String>,
    #[serde(rename = "MagicDNSEnabled")]
    pub magic_dns_enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExitNodeStatus {
    #[serde(rename = "ID")]
    pub id: Option<String>,
    pub online: Option<bool>,
    #[serde(rename = "TailscaleIPs")]
    pub tailscale_ips: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ClientVersion {
    pub running_latest: Option<bool>,
    pub latest_version: Option<String>,
    pub urgent_security_update: Option<bool>,
    pub notify: Option<bool>,
    #[serde(rename = "NotifyURL")]
    pub notify_url: Option<String>,
    pub notify_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Location {
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub city: Option<String>,
    pub city_code: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WhoIsResponse {
    pub node: Option<Node>,
    pub user_profile: Option<UserProfile>,
    pub cap_map: Option<HashMap<String, Vec<serde_json::Value>>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserProfile {
    #[serde(rename = "ID")]
    pub id: Option<i64>,
    pub login_name: Option<String>,
    pub display_name: Option<String>,
    #[serde(rename = "ProfilePicURL")]
    pub profile_pic_url: Option<String>,
    pub groups: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Node {
    #[serde(rename = "ID")]
    pub id: Option<i64>,
    #[serde(rename = "StableID")]
    pub stable_id: Option<String>,
    pub name: Option<String>,
    pub user: Option<i64>,
    pub sharer: Option<i64>,
    pub key: Option<String>,
    pub key_expiry: Option<String>,
    pub machine: Option<String>,
    pub disco_key: Option<String>,
    pub addresses: Option<Vec<String>>,
    pub allowed_ips: Option<Vec<String>>,
    pub endpoints: Option<Vec<String>>,
    #[serde(rename = "DERP")]
    pub derp: Option<String>,
    #[serde(rename = "HomeDERP")]
    pub home_derp: Option<i32>,
    pub created: Option<String>,
    pub cap: Option<i32>,
    pub tags: Option<Vec<String>>,
    pub primary_routes: Option<Vec<String>>,
    pub last_seen: Option<String>,
    pub online: Option<bool>,
    pub machine_authorized: Option<bool>,
    pub capabilities: Option<Vec<String>>,
    pub cap_map: Option<HashMap<String, Vec<serde_json::Value>>>,
    pub computed_name: Option<String>,
    pub computed_name_with_host: Option<String>,
    pub expired: Option<bool>,
    pub is_wire_guard_only: Option<bool>,
    pub is_jailed: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Prefs {
    #[serde(rename = "ControlURL")]
    pub control_url: Option<String>,
    pub route_all: Option<bool>,
    #[serde(rename = "ExitNodeID")]
    pub exit_node_id: Option<String>,
    #[serde(rename = "ExitNodeIP")]
    pub exit_node_ip: Option<String>,
    #[serde(rename = "ExitNodeAllowLANAccess")]
    pub exit_node_allow_lan_access: Option<bool>,
    #[serde(rename = "CorpDNS")]
    pub corp_dns: Option<bool>,
    #[serde(rename = "RunSSH")]
    pub run_ssh: Option<bool>,
    pub run_web_client: Option<bool>,
    pub want_running: Option<bool>,
    pub logged_out: Option<bool>,
    pub shields_up: Option<bool>,
    pub advertise_tags: Option<Vec<String>>,
    pub hostname: Option<String>,
    pub force_daemon: Option<bool>,
    pub advertise_routes: Option<Vec<String>>,
    pub advertise_services: Option<Vec<String>>,
    #[serde(rename = "NoSNAT")]
    pub no_snat: Option<bool>,
    pub no_stateful_filtering: Option<String>,
    pub netfilter_mode: Option<i32>,
    pub operator_user: Option<String>,
    pub profile_name: Option<String>,
    pub auto_update: Option<AutoUpdatePrefs>,
    pub app_connector: Option<AppConnectorPrefs>,
    pub posture_checking: Option<bool>,
    pub netfilter_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AutoUpdatePrefs {
    pub check: Option<bool>,
    pub apply: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AppConnectorPrefs {
    pub advertise: Option<bool>,
}
