# tsnet

Rust bindings for Tailscale's [libtailscale](https://github.com/tailscale/libtailscale) C library. Embed a Tailscale node directly into your Rust process — get an IP on your tailnet entirely from userspace, no system daemon required.

## Prerequisites

- **Go** (to compile libtailscale from the git submodule)
- **Rust nightly** (edition 2024)
- A [Tailscale auth key](https://login.tailscale.com/admin/settings/keys)

After cloning, init the submodule:

```
git submodule update --init
```

## Hello example

A minimal HTTP server that joins your tailnet and responds with `Hello from Rust + tsnet!` to any request.

### Run it

```
cargo run --example hello -- <auth-key> <hostname>
```

- `auth-key` — a Tailscale auth key (`tskey-auth-...`). Generate one from your [admin console](https://login.tailscale.com/admin/settings/keys). Reusable + ephemeral recommended for development.
- `hostname` — the name this node appears as on your tailnet (e.g. `hello-rust`).

### Test it

From any device on your tailnet:

```
curl http://hello-rust.YOUR-TAILNET.ts.net
```

```
Hello from Rust + tsnet!
```

The node registers as ephemeral, so it disappears from your tailnet automatically when the process exits.

### How it works

The example implements two traits:

- **`ConnectionHandler`** — called per-connection with `on_data` (incoming bytes) and `poll_write` (outgoing bytes). The hello handler waits for any request data, then returns a hardcoded HTTP response.
- **`HandlerFactory`** — creates a fresh handler for each accepted connection.

```rust
let server = RawTsTcpServer::new("hello-rust")?;
server.set_auth_key("tskey-auth-...")?;
server.set_ephemeral(true)?;
server.up()?;

let listener = server.listen("tcp", ":80", HttpHelloFactory)?;
// listener runs on a background thread, drop it to stop
```

`listen()` returns immediately. The accept loop runs on a background thread. Drop the `Listener` or call `listener.shutdown()` to stop it.

## Logging

Go-side logs are automatically piped through the `tracing` crate at `debug` level with target `libtailscale`. To see them:

```rust
// Add tracing-subscriber to your dependencies, then:
tracing_subscriber::fmt()
    .with_env_filter("libtailscale=debug")
    .init();
```

Or set `RUST_LOG=libtailscale=debug` if using `EnvFilter`.
