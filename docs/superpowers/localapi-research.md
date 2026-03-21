# Tailscale LocalAPI - Comprehensive Research

## What Is the LocalAPI?

The LocalAPI is an HTTP server embedded within `tailscaled` (the Tailscale daemon). It provides programmatic access to Tailscale functionality for local clients -- the CLI (`tailscale` command), GUI apps, and any program running on the same machine. It is **not** the same as the public Tailscale REST API at `api.tailscale.com`; the LocalAPI talks to your **local daemon** only.

All endpoints live under the path prefix `/localapi/v0/`.

Source: [ipn/localapi/localapi.go](https://github.com/tailscale/tailscale/blob/main/ipn/localapi/localapi.go)

---

## Authentication

### From a Unix socket (Go / `LocalClient`)

On Linux/macOS, `tailscaled` listens on a Unix domain socket (typically `/var/run/tailscale/tailscaled.sock`). Connections over this socket are authenticated by the OS (peer credentials). No password or header is needed when using `local.Client` in Go -- it connects via the socket directly.

### From TCP / Loopback (tsnet, libtailscale, non-Go languages)

When using `tsnet.Server.Loopback()`, the daemon starts a TCP listener on `127.0.0.1:<random-port>` and returns two credentials:

```go
func (s *Server) Loopback() (addr string, proxyCred string, localAPICred string, err error)
```

**Two layers of authentication are required for HTTP/LocalAPI requests:**

1. **HTTP Header**: `Sec-Tailscale: localapi` (mandatory; returns 403 without it)
2. **HTTP Basic Auth**: password = `localAPICred` (the username is ignored)

**For SOCKS5 proxy connections** (outbound traffic through the tailnet):
- Username: `"tsnet"` (hardcoded)
- Password: `proxyCred`

The credentials are random 16-byte hex strings generated at startup.

### Permission Model

Each handler has permission requirements checked against the caller's identity:

| Permission | Meaning |
|---|---|
| `PermitRead` | Read-only operations (status, whois, prefs GET) |
| `PermitWrite` | Mutating operations (login, logout, set prefs). Implies admin/root |
| `PermitCert` | Can fetch TLS certificates |

When accessed via `Loopback()`, **both PermitRead and PermitWrite are true** -- full access.

---

## How It Relates to `tsnet` and `libtailscale`

### tsnet (Go)

`tsnet.Server` embeds a full Tailscale node in-process. Two ways to access the LocalAPI:

1. **`s.LocalClient()`** -- Returns a `*local.Client` that communicates over an in-memory pipe (`memnet`). No TCP, no credentials needed. **Preferred for Go code.**

2. **`s.Loopback()`** -- Starts a TCP listener on `127.0.0.1:<port>`. Returns `(addr, proxyCred, localAPICred)`. Use this when you need to access the LocalAPI from non-Go code or via HTTP directly.

### libtailscale (C FFI / Rust / other languages)

`libtailscale` wraps tsnet as a C library. It exposes `tailscale_loopback()` which is the C equivalent of `Loopback()`:

```c
int tailscale_loopback(tailscale sd, char** addr_out, char** proxy_cred_out, char** local_api_cred_out);
```

This gives you a `127.0.0.1:<port>` address and two credential strings. You then make HTTP requests to `http://<addr>/localapi/v0/<endpoint>` with:
- Header: `Sec-Tailscale: localapi`
- Basic Auth password: `local_api_cred_out`

---

## Complete Endpoint Reference

### Core / Always Available

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `status` | GET | Read | **Node status** -- your node's IP, hostname, online status, and all peers with their IPs, hostnames, OS, online status, last seen, exit node info, etc. This is the main "who's connected" endpoint. |
| `whois?addr=<ip:port>` | GET | Read | **Peer lookup** -- resolve an IP address to a Tailscale node identity. Returns the node info, user profile (name, login, display name, profile pic), and capabilities. Essential for authentication in web services. |
| `prefs` | GET | Read | **Get preferences** -- current node configuration (hostname, advertised routes, exit node, shields up, etc.) |
| `prefs` | PATCH | Write | **Set preferences** -- modify node configuration |
| `check-prefs` | POST | Write | **Validate preferences** -- dry-run check without applying |
| `profiles/` | GET/POST/DELETE | Write | **Profile management** -- list, create, switch, rename, delete user profiles (multi-account support) |
| `start` | POST | Write | **Start backend** -- start the Tailscale engine with given options |
| `login-interactive` | POST | Write | **Interactive login** -- initiate browser-based auth flow |
| `logout` | POST | Write | **Logout** -- log out and wipe node keys |
| `reset-auth` | POST | Write | **Reset auth** -- reset authentication state |
| `set-expiry-sooner` | POST | Write | **Expire key sooner** -- accelerate node key expiration |
| `ping?ip=<addr>&type=<disco\|TSMP\|ICMP>` | POST | Read | **Ping peer** -- ping a peer node, returns latency, path (direct vs DERP relay), endpoint info |
| `derpmap` | GET | None | **DERP map** -- get the current DERP relay server map |
| `reload-config` | POST | Write | **Reload config** -- reload configuration from disk |
| `shutdown` | POST | Write | **Shutdown daemon** -- shut down tailscaled |
| `goroutines` | GET | Write | **Goroutine dump** -- debug goroutine stack traces |
| `check-so-mark-in-use` | GET | Read | **Check SO_MARK** -- whether the Linux SO_MARK socket option is in use |

### Networking & Routing (conditional: HasAdvertiseRoutes)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `check-ip-forwarding` | GET | Read | Check if IP forwarding is enabled on the host |
| `check-udp-gro-forwarding` | GET | Read | Check UDP GRO forwarding status |
| `set-udp-gro-forwarding` | POST | Write | Enable UDP GRO forwarding |

### DNS (conditional: HasDNS)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `dns-osconfig` | GET | Write | Get OS DNS configuration |
| `dns-query?name=<domain>` | POST | Write | Perform DNS query through Tailscale's DNS |

### Exit Nodes (conditional: HasUseExitNode)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `suggest-exit-node` | GET | None | Get recommended exit node based on latency/location |
| `set-use-exit-node-enabled` | POST | Write | Enable/disable exit node usage |

### TLS Certificates (conditional: HasACME)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `set-dns` | POST | Write | Set DNS records (for ACME/cert provisioning) |
| `cert/<domain>` | GET | Cert | Fetch TLS certificate for a Tailscale domain name |

### Serve / Funnel (conditional: HasServe)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `query-feature?feature=<name>` | GET | Read | Check if a feature (like serve, funnel) is available |
| `watch-ipn-bus` | GET | Read | **Stream state changes** -- long-lived SSE stream of IPN notifications (state changes, peer updates, etc.) |

### Taildrop / File Sharing (from localapi_drive.go)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `file-targets` | GET | Read | List devices that can receive files via Taildrop |
| `file-put/<nodekey>/<filename>` | PUT | Write | Send a file to a peer via Taildrop |
| `files/` | GET | Read | List received files waiting to be accepted |
| `files/<name>` | GET/DELETE | Read/Write | Download or delete a received file |

### Drive / File Sharing (from localapi_drive.go)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `drive/shares` | GET/PUT | Read/Write | List or set Tailscale Drive shares |
| `drive/shares/<name>` | POST/DELETE | Write | Rename or remove a Drive share |

### Tailnet Lock (from tailnetlock.go)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `tka/status` | GET | Read | Tailnet lock status |
| `tka/init` | POST | Write | Initialize tailnet lock |
| `tka/modify` | POST | Write | Add/remove trusted keys |
| `tka/sign` | POST | Write | Sign a node key |
| `tka/disable` | POST | Write | Disable tailnet lock |
| `tka/force-local-disable` | POST | Write | Force disable locally |
| `tka/affected-sigs` | POST | Write | Get affected signatures |
| `tka/wrap-preauth-key` | POST | Write | Wrap a pre-auth key for use with TKA |
| `tka/verify-deeplink` | GET | Read | Verify a tailnet lock deep link |
| `tka/generate-recovery-aum` | POST | Write | Generate recovery AUM |
| `tka/cosign-recovery-aum` | POST | Write | Co-sign a recovery AUM |

### Debug & Diagnostics (conditional: HasDebug)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `bugreport` | POST | Read | Generate diagnostic bug report |
| `pprof` | GET | Write | Go pprof profiling endpoint |
| `metrics` | GET | Write | Prometheus-format metrics |
| `id-token?aud=<audience>` | POST | Write | Request OIDC ID token for the node |
| `alpha-set-device-attrs` | PATCH | Write | Set device posture attributes |
| `handle-push-message` | POST | Write | Process push notifications |
| `set-push-device-token` | POST | Write | Register device for push notifications |
| `set-gui-visible` | POST | None | Tell daemon whether GUI is visible (macOS/Windows) |
| `disconnect-control` | POST | Write | Disconnect from control server |
| `logtap` | GET | Write | Stream daemon logs in real-time |
| `watch-ipn-bus` | GET | Read | Stream IPN state change notifications |
| `debug-bus-*` | various | Write | Event bus debugging endpoints |

### Metrics (conditional: HasClientMetrics / HasUserMetrics)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `upload-client-metrics` | POST | None | Submit client performance metrics |
| `usermetrics` | GET | None | User-facing metrics (Prometheus format) |

### App Connectors (conditional: HasAppConnectors)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `appc-route-info` | GET | None | App connector route information |

### Client Update (conditional: HasClientUpdate)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `update/check` | GET | None | Check for available Tailscale updates |

### System Policy (from syspolicy_api.go)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `syspolicy/` | various | Read | Query system/MDM policy settings |

### Outbound Proxy (conditional: HasOutboundProxy / HasSSH)

| Endpoint | Method | Permission | Description |
|---|---|---|---|
| `dial` | POST | None | Establish outbound TCP connection through the tailnet |

---

## Most Useful Endpoints for tsnet Applications

### 1. `GET /localapi/v0/status`

The single most important endpoint. Returns everything about your node and all peers:

```json
{
  "BackendState": "Running",
  "Self": {
    "ID": "n1234567890",
    "HostName": "mynode",
    "DNSName": "mynode.tail1234.ts.net.",
    "TailscaleIPs": ["100.64.1.1", "fd7a:115c:a1e0::1"],
    "Online": true,
    "OS": "linux",
    ...
  },
  "Peer": {
    "<nodekey>": {
      "HostName": "otherpeer",
      "DNSName": "otherpeer.tail1234.ts.net.",
      "TailscaleIPs": ["100.64.1.2"],
      "Online": true,
      "LastSeen": "2024-01-01T00:00:00Z",
      "ExitNode": false,
      "OS": "macOS",
      ...
    }
  },
  "User": {
    "12345": {
      "LoginName": "user@example.com",
      "DisplayName": "User Name",
      ...
    }
  }
}
```

### 2. `GET /localapi/v0/whois?addr=<ip>:<port>`

Identify who is connecting to you. Given a Tailscale IP, returns the full node and user identity. This is the foundation for Tailscale-based authentication.

### 3. `GET /localapi/v0/watch-ipn-bus`

Long-lived streaming connection. Emits JSON `ipn.Notify` objects whenever state changes -- peer comes online, preferences change, new login, etc. Essential for reactive applications.

### 4. `POST /localapi/v0/ping?ip=<addr>&type=disco`

Ping a peer to check connectivity and measure latency. The `type` parameter controls the ping mechanism (disco, TSMP, or ICMP).

### 5. `GET /localapi/v0/cert/<domain>`

Fetch a TLS certificate for your node's Tailscale FQDN. Enables HTTPS on your tsnet services.

---

## Response Headers

All LocalAPI responses include:

```
Tailscale-Version: <version>
Tailscale-Cap: <capability-version>
Content-Security-Policy: default-src 'none'; frame-ancestors 'none'; script-src 'none'; script-src-elem 'none'; script-src-attr 'none'
X-Frame-Options: DENY
X-Content-Type-Options: nosniff
```

---

## Gotchas and Rate Limits

1. **No documented rate limits** on the LocalAPI itself -- it's a local daemon, not a cloud service. However, endpoints that call the control plane (like `login-interactive`, `cert/`) are subject to control server rate limits.

2. **The `Sec-Tailscale: localapi` header is mandatory** when connecting over TCP/loopback. Without it, you get a 403. This header cannot be set by browsers (it's a forbidden header name prefix), which prevents CSRF attacks.

3. **Unix socket vs TCP**: On Linux, the default socket path is `/var/run/tailscale/tailscaled.sock`. On macOS with the App Store version, it may be in a different location. When using tsnet's `Loopback()`, you always get TCP.

4. **`watch-ipn-bus` is long-lived**: It's a streaming endpoint (Server-Sent Events style). The connection stays open and pushes JSON notifications. Don't treat it as a normal request/response.

5. **Conditional endpoints**: Many endpoints only exist if the binary was compiled with the right build tags (e.g., `HasDebug`, `HasServe`). If an endpoint returns 404, it may not be compiled in.

6. **`status` can be expensive**: On large tailnets with many peers, the status response can be large. Use it judiciously if polling.

7. **Permissions are enforced server-side**: Even with valid credentials, if the handler requires `PermitWrite` and you only have `PermitRead`, you'll get 403.

8. **`ping` is POST, not GET**: Despite being a "read" operation conceptually, ping requires POST because it initiates network activity.

9. **Profile switching**: The `profiles/` endpoint supports multi-account. Switching profiles effectively switches which tailnet you're connected to.

10. **`id-token` is powerful**: Returns a signed OIDC JWT for the node, which can be verified by third-party services. Useful for machine-to-machine auth.

---

## Example: Calling LocalAPI from Rust via Loopback

```rust
// After calling tailscale_loopback() to get addr, proxy_cred, local_api_cred:

let client = reqwest::Client::new();
let resp = client
    .get(format!("http://{}/localapi/v0/status", addr))
    .header("Sec-Tailscale", "localapi")
    .basic_auth("", Some(&local_api_cred))
    .send()
    .await?;

let status: serde_json::Value = resp.json().await?;
```

---

## Sources

- [ipn/localapi/localapi.go (GitHub)](https://github.com/tailscale/tailscale/blob/main/ipn/localapi/localapi.go)
- [tsnet/tsnet.go (GitHub)](https://github.com/tailscale/tailscale/blob/main/tsnet/tsnet.go)
- [localapi Go package docs](https://pkg.go.dev/tailscale.com/ipn/localapi)
- [tsnet Go package docs](https://pkg.go.dev/tailscale.com/tsnet)
- [Tailscale blog: The subtle magic of tsnet](https://tailscale.com/blog/tsup-tsnet)
- [tsnet docs](https://tailscale.com/kb/1244/tsnet)
- [Tailscale API docs](https://tailscale.com/kb/1101/api)
- [DeepWiki: Local API Server](https://deepwiki.com/tailscale/tailscale/6.3-cicd-and-testing)
- [Rust tailscale-localapi crate](https://docs.rs/tailscale-localapi)
- [libtailscale C library](https://github.com/badboy/libtailscale)
