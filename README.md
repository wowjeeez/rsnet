# rsnet

Rust bindings for Tailscale's [libtailscale](https://github.com/tailscale/libtailscale) C library. Embed a Tailscale node directly into your Rust process — get an IP on your tailnet entirely from userspace, no system daemon required.

Fully async (tokio). Streams implement `AsyncRead + AsyncWrite + Unpin`.

## Prerequisites

- **Go** (to compile libtailscale from the git submodule)
- **Rust stable** (edition 2021+)
- A [Tailscale auth key](https://login.tailscale.com/admin/settings/keys)

```
git clone --recurse-submodules <repo-url>
```

## Quick start

```rust
let mut server = RawTsTcpServer::new("my-node")?;
server.set_auth_key("tskey-auth-...")?;
server.set_dir("/var/lib/my-node")?;
server.up()?;

let listener = server.listen("tcp", ":80")?;
loop {
    let stream = listener.accept().await?; // TailscaleStream: AsyncRead + AsyncWrite
    tokio::spawn(handle_connection(stream));
}
```

## Features

| Feature | Default | Adds |
|---|---|---|
| `ssl` | yes | `TlsListener`, `listen_tls()`, `listen_tls_with_pem()` via tokio-rustls |
| `localapi-serde-json` | no | Typed `Status`, `PeerStatus`, `WhoIsResponse` structs, `whoami()`, `fqdn()`, `whois()` |

```toml
[dependencies]
rsnet = "0.1"                                    # ssl only (default)
rsnet = { version = "0.1", features = ["localapi-serde-json"] }  # + typed localapi
rsnet = { version = "0.1", default-features = false }            # core only
```

## Examples

### Plain HTTP

```
cargo run --example hello -- <auth-key> <hostname>
curl http://<hostname>.YOUR-TAILNET.ts.net
```

### HTTPS with auto TLS

```
cargo run --example hello_tls --features localapi-serde-json -- <auth-key> <hostname>
curl https://<hostname>.YOUR-TAILNET.ts.net
```

Fetches LetsEncrypt certs from Tailscale's LocalAPI automatically. First run takes a few seconds for ACME.

## TLS

Three levels of control:

```rust
// auto: fetches fqdn + certs from localapi, listens on :443
let tls = server.listen_tls().await?;

// manual certs: bring your own PEM
let tls = server.listen_tls_with_pem("tcp", ":8443", &cert_pem, &key_pem)?;

// full control: build your own rustls config
let tls = TlsListener::new(listener, my_rustls_server_config);

// all return TlsStream<TailscaleStream> from accept
let tls_stream = tls.accept().await?;
```

## LocalAPI

Access Tailscale's node-local HTTP API for status, peer info, certs, and more:

```rust
let client = server.local_client()?;

// raw requests (always available)
let (status_code, body) = client.get("/localapi/v0/status").await?;
let cert_pem = client.cert("my-node.tailnet.ts.net").await?;
let (cert, key) = client.cert_pair("my-node.tailnet.ts.net").await?;

// typed responses (requires localapi-serde-json feature)
let me = client.whoami().await?;           // PeerStatus
let domain = client.fqdn().await?;         // String
let status = client.status().await?;       // Status (with all peers)
let who = client.whois("100.x.y.z:443").await?; // WhoIsResponse
```

## Logging

Go-side logs are piped through `tracing` at debug level with target `libtailscale`:

```
RUST_LOG=libtailscale=debug cargo run --example hello -- ...
```

## State persistence

Set a state directory to avoid re-authentication on every restart:

```rust
server.set_dir("/var/lib/my-node")?;
```

Without this, libtailscale uses a default path keyed by binary name. Combined with `set_ephemeral(true)`, the node is deleted from your tailnet when the process exits and needs re-auth next run.
