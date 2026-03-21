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
    Ok(Response::new(Full::new(Bytes::from("Hello from Rust + Tailscale TLS!"))))
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
        eprintln!("Usage: {} <auth-key> <hostname> [control-url]", args[0]);
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

    let mut server = RawTsTcpServer::new(hostname).expect("failed to create server");
    server.set_auth_key(auth_key).expect("failed to set auth key");

    let state_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rsnet");
    std::fs::create_dir_all(&state_dir).expect("failed to create state dir");
    server.set_dir(state_dir.to_str().unwrap()).expect("failed to set state dir");

    if let Some(url) = args.get(3) {
        server.set_control_url(url).expect("failed to set control url");
    }

    eprintln!("[startup] connecting to tailnet...");
    server.up().expect("failed to bring server up");

    let ips = server.getips().unwrap_or_default();
    eprintln!("[info] IPs: {}", ips);

    // go handles tls + acme certs natively — no rustls needed
    eprintln!("[tls] starting native TLS listener on :443...");
    let listener = server.listen_native_tls("tcp", ":443").expect("failed to listen");
    eprintln!("[tls] ready!");

    // signal handler after up()
    let mut pipe_fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let signal_read = unsafe { OwnedFd::from_raw_fd(pipe_fds[0]) };
    unsafe { SIGNAL_WRITE_FD = pipe_fds[1] };
    unsafe { libc::signal(libc::SIGINT, on_signal as *const () as usize) };

    eprintln!();
    eprintln!("Try: curl https://{}.YOUR-TAILNET.ts.net", hostname);
    eprintln!("Press Ctrl+C to stop.");
    eprintln!();

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    eprintln!("  connection accepted (fd={})", stream.as_raw_fd());
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
                    continue;
                }
            }
        }
    });

    let mut buf = [0u8; 1];
    unsafe { libc::read(signal_read.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 1) };

    eprintln!("\n[shutdown] done.");
    let _ = server.close();
    std::process::exit(0);
}
