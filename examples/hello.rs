use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tracing_subscriber::EnvFilter;
use rsnet::RawTsTcpServer;

async fn hello(_: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    Ok(Response::new(Full::new(Bytes::from("Hello from Rust + Tailscale C FFI API!"))))
}

static mut SIGNAL_WRITE_FD: RawFd = -1;

extern "C" fn on_signal(_: libc::c_int) {
    unsafe {
        libc::write(SIGNAL_WRITE_FD, b"x".as_ptr() as *const libc::c_void, 1);
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <auth-key> <hostname> [control-url] [state-dir]", args[0]);
        eprintln!("  auth-key:    Tailscale auth key (tskey-auth-...)");
        eprintln!("  hostname:    Name this node will appear as on your tailnet");
        eprintln!("  control-url: Coordination server URL (optional, for headscale etc.)");
        eprintln!("  state-dir:   Directory to persist node state (optional, default: ephemeral)");
        std::process::exit(1);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("libtailscale=debug")),
        )
        .init();

    let auth_key = &args[1];
    let hostname = &args[2];
    let control_url = args.get(3).map(|s| s.as_str());
    let state_dir = args.get(4).map(|s| s.as_str());

    eprintln!("=== rsnet hello example ===");
    eprintln!();

    eprintln!("[config] creating server with hostname '{}'", hostname);
    let mut server = RawTsTcpServer::new(hostname).expect("failed to create server");

    eprintln!("[config] setting auth key ({}...)", &auth_key[..auth_key.len().min(12)]);
    server.set_auth_key(auth_key).expect("failed to set auth key");

    if let Some(url) = control_url {
        eprintln!("[config] using control server: {}", url);
        server.set_control_url(url).expect("failed to set control url");
    } else {
        eprintln!("[config] using default Tailscale control server");
    }

    if let Some(dir) = state_dir {
        server.set_dir(dir).expect("failed to set state dir");
        eprintln!("[config] state dir: {}", dir);
    } else {
        let default_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("rsnet");
        std::fs::create_dir_all(&default_dir).expect("failed to create state dir");
        server.set_dir(default_dir.to_str().unwrap()).expect("failed to set state dir");
        eprintln!("[config] state dir: {}", default_dir.display());
    }

    eprintln!();
    eprintln!("[startup] connecting to tailnet...");
    server.up().expect("failed to bring server up");
    eprintln!("[startup] connected!");

    match server.getips() {
        Ok(ips) => {
            eprintln!();
            eprintln!("[info] tailscale IPs:");
            for ip in ips.split(',') {
                eprintln!("  - {}", ip);
            }
        }
        Err(e) => eprintln!("[info] could not get IPs: {}", e),
    }
    eprintln!("[info] hostname: {}", hostname);

    // Register signal handler AFTER up() — Go runtime overrides during startup
    let mut pipe_fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let signal_read = unsafe { OwnedFd::from_raw_fd(pipe_fds[0]) };
    unsafe { SIGNAL_WRITE_FD = pipe_fds[1] };
    unsafe { libc::signal(libc::SIGINT, on_signal as *const () as usize) };

    eprintln!();
    eprintln!("[listen] starting TCP listener on :80");
    let listener = server.listen("tcp", ":80").expect("failed to listen");

    eprintln!();
    eprintln!("Ready! Try from any device on your tailnet:");
    eprintln!("  curl http://{}.YOUR-TAILNET.ts.net", hostname);
    eprintln!();
    eprintln!("Press Ctrl+C to stop.");
    eprintln!();

    // Spawn the accept loop
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    eprintln!("  connection from {} on port {} (fd={})",
                        stream.peer_addr().unwrap_or("unknown"),
                        stream.local_port().map(|p| p.to_string()).unwrap_or("?".into()),
                        stream.as_raw_fd(),
                    );
                    let io = TokioIo::new(stream);
                    tokio::spawn(async move {
                        if let Err(e) = http1::Builder::new()
                            .serve_connection(io, service_fn(hello))
                            .await
                        {
                            eprintln!("  http error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("  accept error: {}", e);
                    break;
                }
            }
        }
    });

    // Block until ctrl-c
    let mut buf = [0u8; 1];
    unsafe { libc::read(signal_read.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 1) };

    eprintln!();
    eprintln!("[shutdown] closing server...");
    let _ = server.close();
    eprintln!("[shutdown] done.");
    std::process::exit(0);
}
