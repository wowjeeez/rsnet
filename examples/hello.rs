use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use tracing_subscriber::EnvFilter;
use tsnet::{ConnectionHandler, FdControl, HandlerFactory, RawTsTcpServer};

const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 38\r\nConnection: close\r\n\r\nHello from Rust + Tailscale C FFI API!";

struct HttpHelloHandler {
    got_request: bool,
    sent_response: bool,
}

impl ConnectionHandler for HttpHelloHandler {
    fn on_connect(&mut self, fd: RawFd) -> FdControl {
        eprintln!("  connection established (fd={})", fd);
        FdControl::Keep
    }

    fn on_data(&mut self, data: &[u8]) {
        if let Some(line) = std::str::from_utf8(data).ok().and_then(|s| s.lines().next()) {
            eprintln!("  request: {}", line);
        }
        self.got_request = true;
    }

    fn poll_write(&mut self) -> Option<Vec<u8>> {
        if self.got_request && !self.sent_response {
            self.sent_response = true;
            Some(RESPONSE.to_vec())
        } else {
            None
        }
    }

    fn is_done(&self) -> bool {
        self.sent_response
    }
}

struct HttpHelloFactory;

impl HandlerFactory for HttpHelloFactory {
    type Handler = HttpHelloHandler;
    fn new_handler(&self) -> Self::Handler {
        HttpHelloHandler {
            got_request: false,
            sent_response: false,
        }
    }
}

static mut SIGNAL_WRITE_FD: RawFd = -1;

extern "C" fn on_signal(_: libc::c_int) {
    unsafe {
        libc::write(SIGNAL_WRITE_FD, b"x".as_ptr() as *const libc::c_void, 1);
    }
}

fn main() {
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

    eprintln!("=== tsnet hello example ===");
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
        eprintln!("[config] persisting state to: {}", dir);
        server.set_dir(dir).expect("failed to set state dir");
        server.set_ephemeral(false).expect("failed to set ephemeral");
        eprintln!("[config] ephemeral: false (state will persist across restarts)");
    } else {
        server.set_ephemeral(true).expect("failed to set ephemeral");
        eprintln!("[config] ephemeral: true (node removed when process exits)");
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
    server.set_hostname(hostname).expect("failed to re-set hostname");

    let mut pipe_fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let signal_read = unsafe { OwnedFd::from_raw_fd(pipe_fds[0]) };
    unsafe { SIGNAL_WRITE_FD = pipe_fds[1] };
    unsafe { libc::signal(libc::SIGINT, on_signal as *const () as usize) };

    eprintln!();
    eprintln!("[listen] starting TCP listener on :80");
    let listener = server
        .listen("tcp", ":80", HttpHelloFactory)
        .expect("failed to listen");
    eprintln!("[listen] accepting connections on background thread");

    eprintln!();
    eprintln!("Ready! Try from any device on your tailnet:");
    eprintln!("  curl http://{}.YOUR-TAILNET.ts.net", hostname);
    eprintln!();
    eprintln!("Press Ctrl+C to stop.");
    eprintln!();

    let mut buf = [0u8; 1];
    unsafe { libc::read(signal_read.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 1) };

    eprintln!();
    eprintln!("[shutdown] stopping listener...");
    let _ = listener.shutdown();
    eprintln!("[shutdown] closing server...");
    let _ = server.close();
    eprintln!("[shutdown] done.");
    std::process::exit(0);
}
