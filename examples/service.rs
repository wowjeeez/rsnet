use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing_subscriber::EnvFilter;
use rsnet::RawTsTcpServer;

static mut SIGNAL_WRITE_FD: RawFd = -1;

extern "C" fn on_signal(_: libc::c_int) {
    unsafe {
        libc::write(SIGNAL_WRITE_FD, b"x".as_ptr() as *const libc::c_void, 1);
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: {} <auth-key> <hostname> <service-name>", args[0]);
        eprintln!("  auth-key:     Tagged auth key (create with tag:service in admin console)");
        eprintln!("  hostname:     Name this node appears as on your tailnet");
        eprintln!("  service-name: Service name (e.g. svc:my-api)");
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
    let service_name = &args[3];

    let mut server = RawTsTcpServer::new(hostname).expect("failed to create server");
    server.set_auth_key(auth_key).expect("failed to set auth key");

    let state_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rsnet");
    std::fs::create_dir_all(&state_dir).expect("failed to create state dir");
    server.set_dir(state_dir.to_str().unwrap()).expect("failed to set state dir");

    eprintln!("[startup] connecting to tailnet...");
    server.up().expect("failed to bring server up");

    let ips = server.getips().unwrap_or_default();
    eprintln!("[info] IPs: {}", ips);

    eprintln!("[service] binding {} on ports 80 (http) + 443 (https) + 9000 (tcp)...", service_name);
    eprintln!("[service] note: auth key must be tagged (e.g. tag:service) for services to work");
    let mut svc = server.service(service_name)
        .http(80)
        .https(443)
        .tcp(9000)
        .bind()
        .expect("failed to bind service");

    eprintln!("[service] fqdn: {}", svc.fqdn);

    let mut pipe_fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let signal_read = unsafe { OwnedFd::from_raw_fd(pipe_fds[0]) };
    unsafe { SIGNAL_WRITE_FD = pipe_fds[1] };
    unsafe { libc::signal(libc::SIGINT, on_signal as *const () as usize) };

    eprintln!();
    eprintln!("Ready! Accepting on all ports. Press Ctrl+C to stop.");
    eprintln!();

    tokio::spawn(async move {
        loop {
            match svc.accept().await {
                Ok((port, mut stream)) => {
                    eprintln!("  port={} peer={} fd={}",
                        port,
                        stream.peer_addr().unwrap_or("unknown"),
                        stream.as_raw_fd(),
                    );
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 4096];
                        match stream.read(&mut buf).await {
                            Ok(n) if n > 0 => {
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nHello from port {port}!\n"
                                );
                                let _ = stream.write_all(response.as_bytes()).await;
                            }
                            _ => {}
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

    let mut buf = [0u8; 1];
    unsafe { libc::read(signal_read.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 1) };

    eprintln!("\n[shutdown] done.");
    let _ = server.close();
    std::process::exit(0);
}
