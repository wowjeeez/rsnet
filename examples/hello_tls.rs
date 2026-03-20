use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio_rustls::TlsAcceptor;
use tracing_subscriber::EnvFilter;
use tsnet::RawTsTcpServer;

async fn hello(_: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    Ok(Response::new(Full::new(Bytes::from("Hello from Rust + Tailscale TLS!"))))
}

static mut SIGNAL_WRITE_FD: RawFd = -1;

extern "C" fn on_signal(_: libc::c_int) {
    unsafe {
        libc::write(SIGNAL_WRITE_FD, b"x".as_ptr() as *const libc::c_void, 1);
    }
}

fn build_tls_config(cert_pem: &[u8], key_pem: &[u8]) -> Arc<tokio_rustls::rustls::ServerConfig> {
    let certs = rustls_pemfile::certs(&mut &*cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .expect("failed to parse cert pem");

    let key = rustls_pemfile::private_key(&mut &*key_pem)
        .expect("failed to parse key pem")
        .expect("no private key found in pem");

    let config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("failed to build tls config");

    Arc::new(config)
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <auth-key> <hostname> [control-url]", args[0]);
        eprintln!("  auth-key: Tailscale auth key (tskey-auth-...)");
        eprintln!("  hostname: Name this node will appear as on your tailnet");
        eprintln!("  control-url: Coordination server URL (optional)");
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

    eprintln!("=== tsnet TLS example ===");
    eprintln!();

    let mut server = RawTsTcpServer::new(hostname).expect("failed to create server");
    server.set_auth_key(auth_key).expect("failed to set auth key");

    // persist state so we don't need to re-auth on every restart
    let state_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("tsnet-rust");
    std::fs::create_dir_all(&state_dir).expect("failed to create state dir");
    server.set_dir(state_dir.to_str().unwrap()).expect("failed to set state dir");
    eprintln!("[config] state dir: {}", state_dir.display());

    if let Some(url) = args.get(3) {
        server.set_control_url(url).expect("failed to set control url");
    }

    eprintln!("[startup] connecting to tailnet...");
    server.up().expect("failed to bring server up");
    eprintln!("[startup] connected!");

    let ips = server.getips().unwrap_or_default();
    eprintln!("[info] IPs: {}", ips);

    // fetch tls cert from tailscale's localapi
    eprintln!("[tls] fetching certificate from localapi...");
    let client = server.local_client().expect("failed to start loopback");

    let me = client.whoami().await.expect("whoami failed");
    eprintln!("[info] hostname: {:?}, id: {:?}, os: {:?}", me.host_name, me.id, me.os);
    if let Some(ref ips) = me.tailscale_ips {
        for ip in ips {
            eprintln!("[info]   ip: {}", ip);
        }
    }

    let domain = client.fqdn().await.expect("could not get fqdn");

    eprintln!("[tls] requesting cert for {}", domain);
    let (cert_pem, key_pem) = client.cert_pair(&domain).await.expect("failed to fetch cert");
    eprintln!("[tls] got cert ({} bytes) + key ({} bytes)", cert_pem.len(), key_pem.len());

    let tls_config = build_tls_config(&cert_pem, &key_pem);
    let tls_acceptor = TlsAcceptor::from(tls_config);

    // signal handler after up()
    let mut pipe_fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let signal_read = unsafe { OwnedFd::from_raw_fd(pipe_fds[0]) };
    unsafe { SIGNAL_WRITE_FD = pipe_fds[1] };
    unsafe { libc::signal(libc::SIGINT, on_signal as *const () as usize) };

    eprintln!();
    eprintln!("[listen] starting TLS listener on :443");
    let listener = server.listen("tcp", ":443").expect("failed to listen");

    eprintln!();
    eprintln!("Ready! Try:");
    eprintln!("  curl https://{}", domain);
    eprintln!();
    eprintln!("Press Ctrl+C to stop.");
    eprintln!();

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    eprintln!("  connection accepted (fd={})", stream.as_raw_fd());
                    let acceptor = tls_acceptor.clone();
                    tokio::spawn(async move {
                        match acceptor.accept(stream).await {
                            Ok(tls_stream) => {
                                let io = TokioIo::new(tls_stream);
                                if let Err(e) = http1::Builder::new()
                                    .serve_connection(io, service_fn(hello))
                                    .await
                                {
                                    eprintln!("  http error: {}", e);
                                }
                            }
                            Err(e) => eprintln!("  tls handshake error: {}", e),
                        }
                    });
                }
                Err(e) => {
                    eprintln!("  accept error (retrying): {}", e);
                    continue;
                }
            }
        }
    });

    let mut buf = [0u8; 1];
    unsafe { libc::read(signal_read.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 1) };

    eprintln!();
    eprintln!("[shutdown] done.");
    let _ = server.close();
    std::process::exit(0);
}
