use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use tsnet::{ConnectionHandler, HandlerFactory, RawTsTcpServer};

const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 24\r\nConnection: close\r\n\r\nHello from Rust + tsnet!";

struct HttpHelloHandler {
    got_request: bool,
    sent_response: bool,
}

impl ConnectionHandler for HttpHelloHandler {
    fn on_data(&mut self, _data: &[u8]) {
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
        eprintln!("Usage: {} <auth-key> <hostname>", args[0]);
        eprintln!("  auth-key: Tailscale auth key (tskey-auth-...)");
        eprintln!("  hostname: Name this node will appear as on your tailnet");
        std::process::exit(1);
    }

    let auth_key = &args[1];
    let hostname = &args[2];

    // Self-pipe for ctrl-c: signal handler writes a byte, main thread blocks on read
    let mut pipe_fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let signal_read = unsafe { OwnedFd::from_raw_fd(pipe_fds[0]) };
    unsafe { SIGNAL_WRITE_FD = pipe_fds[1] };
    unsafe { libc::signal(libc::SIGINT, on_signal as *const () as usize) };

    let mut server = RawTsTcpServer::new(hostname).expect("failed to create server");
    server.set_auth_key(auth_key).expect("failed to set auth key");
    server.set_ephemeral(true).expect("failed to set ephemeral");

    eprintln!("Starting tsnet node '{}'...", hostname);
    server.up().expect("failed to bring server up");

    match server.getips() {
        Ok(ips) => eprintln!("Listening on port 80 (IPs: {})", ips),
        Err(_) => eprintln!("Listening on port 80"),
    }
    eprintln!("Try: curl http://{}.YOUR-TAILNET.ts.net", hostname);
    eprintln!("Press Ctrl+C to stop.");

    let listener = server
        .listen("tcp", ":80", HttpHelloFactory)
        .expect("failed to listen");

    // Block until ctrl-c
    let mut buf = [0u8; 1];
    unsafe { libc::read(signal_read.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 1) };

    eprintln!("\nShutting down...");
    let _ = listener.shutdown();
    let _ = server.close();
    std::process::exit(0);
}
