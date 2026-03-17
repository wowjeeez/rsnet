use std::collections::VecDeque;
use std::ffi::{c_char, c_int, CStr, CString};
use libc;
use std::io;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use mio::{Events, Interest, Poll, Token};
use mio::unix::SourceFd;
use crate::vendor::libtailscale;

#[derive(Debug)]
pub enum TsNetError {
    IO(io::Error),
    TAILSCALE(String),
}

impl std::fmt::Display for TsNetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TsNetError::IO(e) => write!(f, "{}", e),
            TsNetError::TAILSCALE(msg) => write!(f, "tailscale error: {}", msg),
        }
    }
}

impl std::error::Error for TsNetError {}

impl From<io::Error> for TsNetError {
    fn from(e: io::Error) -> Self {
        TsNetError::IO(e)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FdControl {
    KEEP,
    TAKE_OVER,
}

pub trait ConnectionHandler {
    fn on_connect(&mut self, fd: RawFd) -> FdControl {
        let _ = fd;
        FdControl::KEEP
    }

    fn on_data(&mut self, data: &[u8]);

    fn poll_write(&mut self) -> Option<Vec<u8>>;

    fn is_done(&self) -> bool;
}

pub trait HandlerFactory {
    type Handler: ConnectionHandler;
    fn new_handler(&self) -> Self::Handler;
}

struct ConnState<H: ConnectionHandler> {
    fd: RawFd,
    handler: H,
    write_buf: VecDeque<u8>,
}


unsafe fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    let res = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if res == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}


fn try_flush<H: ConnectionHandler>(state: &mut ConnState<H>) -> io::Result<bool> {
    while let Some(bytes) = state.handler.poll_write() {
        state.write_buf.extend(bytes);
    }

    while !state.write_buf.is_empty() {
        let (first, second) = state.write_buf.as_slices();
        let slice = if !first.is_empty() { first } else { second };
        let n = unsafe {
            libc::write(state.fd, slice.as_ptr() as *const libc::c_void, slice.len())
        };
        if n > 0 {
            state.write_buf.drain(..n as usize);
        } else if n == 0 {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        } else {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                return Ok(true);
            }
            return Err(err);
        }
    }
    Ok(false)
}


fn drive_conn<H: ConnectionHandler>(fd: RawFd, handler: H) -> io::Result<()> {
    let _owned = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(16);
    const CONN: Token = Token(0);
    poll.registry()
        .register(&mut SourceFd(&fd), CONN, Interest::READABLE)?;

    let mut state = ConnState {
        fd,
        handler,
        write_buf: VecDeque::new(),
    };

    loop {
        poll.poll(&mut events, None)?;

        for event in &events {
            if event.token() != CONN {
                continue;
            }

            let mut close = false;

            if event.is_readable() {
                let mut buf = [0u8; 8192];
                loop {
                    let n = unsafe {
                        libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                    };
                    if n > 0 {
                        state.handler.on_data(&buf[..n as usize]);
                        if state.handler.is_done() {
                            break;
                        }
                    } else if n == 0 {
                        close = true;
                        break;
                    } else {
                        let err = io::Error::last_os_error();
                        if err.kind() != io::ErrorKind::WouldBlock {
                            close = true;
                        }
                        break;
                    }
                }
            }

            if !close {
                match try_flush(&mut state) {
                    Ok(wants_writable) => {
                        let interest = if wants_writable {
                            Interest::READABLE | Interest::WRITABLE
                        } else {
                            Interest::READABLE
                        };
                        if poll.registry()
                            .reregister(&mut SourceFd(&fd), CONN, interest)
                            .is_err()
                        {
                            close = true;
                        }
                    }
                    Err(_) => { close = true; }
                }
            }

            if !close && state.handler.is_done() {
                close = true;
            }

            if close {
                return Ok(());
            }
        }
    }
}

fn str_to_c(s: &str) -> Result<CString, TsNetError> {
    CString::new(s).map_err(|e| TsNetError::TAILSCALE(e.to_string()))
}

pub struct RawTsNetServer {
    server: ::std::os::raw::c_int,
}

impl RawTsNetServer {
    pub fn new(hostname: &str) -> Self {
        unsafe {
            let server = libtailscale::tailscale_new();
            let hostname_c = CString::new(hostname).expect("hostname contained null byte");
            libtailscale::tailscale_set_hostname(server, hostname_c.as_ptr());
            RawTsNetServer { server }
        }
    }

    pub fn set_control_server(&self, url: &str) -> Result<(), TsNetError> {
        let url_c = str_to_c(url)?;
        unsafe {
            let err = libtailscale::tailscale_set_control_url(self.server, url_c.as_ptr());
            if err != 0 {
                return Err(self.read_error());
            }
        }
        Ok(())
    }

    pub fn set_auth_key(&self, key: &str) -> Result<(), TsNetError> {
        let key_c = str_to_c(key)?;
        unsafe {
            let err = libtailscale::tailscale_set_authkey(self.server, key_c.as_ptr());
            if err != 0 {
                return Err(self.read_error());
            }
        }
        Ok(())
    }

    pub fn set_dir(&self, dir: &str) -> Result<(), TsNetError> {
        let dir_c = str_to_c(dir)?;
        unsafe {
            let err = libtailscale::tailscale_set_dir(self.server, dir_c.as_ptr());
            if err != 0 {
                return Err(self.read_error());
            }
        }
        Ok(())
    }

    pub fn set_log_fd(&self, fd: OwnedFd) -> Result<(), TsNetError> {
        unsafe {
            let err = libtailscale::tailscale_set_logfd(self.server, fd.into_raw_fd());
            if err != 0 {
                return Err(self.read_error());
            }
        }
        Ok(())
    }

    pub fn set_ephemeral(&self, state: bool) -> Result<(), TsNetError> {
        unsafe {
            let err = libtailscale::tailscale_set_ephemeral(self.server, if state { 1 } else { 0 });
            if err != 0 {
                return Err(self.read_error());
            }
        }
        Ok(())
    }

    pub fn close(&self) -> Result<(), TsNetError> {
        unsafe {
            let err = libtailscale::tailscale_close(self.server);
            if err != 0 {
                return Err(self.read_error());
            }
        }
        Ok(())
    }

    fn read_error(&self) -> TsNetError {
        let mut buf = vec![0 as c_char; 256];
        unsafe {
            libtailscale::tailscale_errmsg(self.server, buf.as_mut_ptr(), 255);
            *buf.last_mut().unwrap() = 0;
            let msg = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
            TsNetError::TAILSCALE(msg)
        }
    }

    pub fn up(&self) -> Result<(), TsNetError> {
        unsafe {
            let err = libtailscale::tailscale_up(self.server);
            if err != 0 {
                return Err(self.read_error());
            }
        }
        Ok(())
    }

    pub fn start(&self) -> Result<(), TsNetError> {
        unsafe {
            let err = libtailscale::tailscale_start(self.server);
            if err != 0 {
                return Err(self.read_error());
            }
        }
        Ok(())
    }

    pub fn listen<F: HandlerFactory>(
        &self,
        network: &str,
        addr: &str,
        factory: F,
    ) -> Result<(), TsNetError> {
        let network_c = str_to_c(network)?;
        let addr_c = str_to_c(addr)?;

        let mut listener_fd: c_int = 0;
        let err = unsafe {
            libtailscale::tailscale_listen(
                self.server,
                network_c.as_ptr(),
                addr_c.as_ptr(),
                &mut listener_fd,
            )
        };
        if err != 0 {
            return Err(self.read_error());
        }

        let _listener_owned = unsafe { OwnedFd::from_raw_fd(listener_fd) };

        unsafe { set_nonblocking(listener_fd) }?;

        let mut poll = Poll::new()?;
        let mut events = Events::with_capacity(128);

        const LISTENER: Token = Token(0);
        poll.registry()
            .register(&mut SourceFd(&listener_fd), LISTENER, Interest::READABLE)?;

        let mut conns: HashMap<Token, ConnState<F::Handler>> = HashMap::new();
        let mut next_token: usize = 1;

        loop {
            poll.poll(&mut events, None)?;

            for event in &events {
                match event.token() {
                    LISTENER => {
                        loop {
                            let mut conn_fd: c_int = 0;
                            let res = unsafe {
                                libtailscale::tailscale_accept(listener_fd, &mut conn_fd)
                            };
                            if res != 0 {
                                let e = io::Error::last_os_error();
                                if e.kind() != io::ErrorKind::WouldBlock {
                                    return Err(TsNetError::IO(e));
                                }
                                break;
                            }

                            let mut buf = vec![0 as c_char; 256];
                            let remote_addr = unsafe {
                                libtailscale::tailscale_getremoteaddr(self.server, conn_fd, buf.as_mut_ptr(), 255)
                            };
                            if remote_addr != 0 {
                                let e = io::Error::last_os_error();
                                if e.kind() != io::ErrorKind::WouldBlock {
                                    return Err(TsNetError::IO(e));
                                }
                                break;
                            }

                            if unsafe { set_nonblocking(conn_fd) }.is_err() {
                                unsafe { libc::close(conn_fd) };
                                continue;
                            }
                            while conns.contains_key(&Token(next_token)) {
                                next_token = next_token.wrapping_add(1);
                                if next_token == 0 {
                                    next_token = 1;
                                }
                            }
                            let token = Token(next_token);
                            next_token = next_token.wrapping_add(1);
                            if next_token == 0 {
                                next_token = 1;
                            }
                            if poll.registry()
                                .register(&mut SourceFd(&conn_fd), token, Interest::READABLE)
                                .is_err()
                            {
                                unsafe { libc::close(conn_fd) };
                                continue;
                            }
                            let conn_owned = unsafe { OwnedFd::from_raw_fd(conn_fd) };
                            let mut handler = factory.new_handler();
                            match handler.on_connect(conn_owned.as_raw_fd()) {
                                FdControl::KEEP => {
                                    conns.insert(token, ConnState {
                                        fd: conn_owned.into_raw_fd(),
                                        handler,
                                        write_buf: VecDeque::new(),
                                    });
                                }
                                FdControl::TAKE_OVER => {
                                    let fd = conn_owned.into_raw_fd();
                                    let _ = poll.registry().deregister(&mut SourceFd(&fd));
                                }
                            }
                        }
                    }
                    token => {
                        let should_close = if let Some(state) = conns.get_mut(&token) {
                            let mut close = false;

                            if event.is_readable() {
                                let mut buf = [0u8; 8192];
                                loop {
                                    let n = unsafe {
                                        libc::read(
                                            state.fd,
                                            buf.as_mut_ptr() as *mut libc::c_void,
                                            buf.len(),
                                        )
                                    };
                                    if n > 0 {
                                        state.handler.on_data(&buf[..n as usize]);
                                        if state.handler.is_done() {
                                            break;
                                        }
                                    } else if n == 0 {
                                        close = true;
                                        break;
                                    } else {
                                        let err = io::Error::last_os_error();
                                        if err.kind() != io::ErrorKind::WouldBlock {
                                            close = true;
                                        }
                                        break;
                                    }
                                }
                            }

                            if !close {
                                match try_flush(state) {
                                    Ok(wants_writable) => {
                                        let interest = if wants_writable {
                                            Interest::READABLE | Interest::WRITABLE
                                        } else {
                                            Interest::READABLE
                                        };
                                        let _ = poll.registry().reregister(
                                            &mut SourceFd(&state.fd),
                                            token,
                                            interest,
                                        );
                                    }
                                    Err(_) => { close = true; }
                                }
                            }

                            if !close && state.handler.is_done() {
                                close = true;
                            }

                            close
                        } else {
                            false
                        };

                        if should_close {
                            if let Some(state) = conns.remove(&token) {
                                let _ = poll.registry().deregister(&mut SourceFd(&state.fd));
                                unsafe { libc::close(state.fd) };
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn dial<H: ConnectionHandler>(
        &self,
        network: &str,
        addr: &str,
        mut handler: H,
    ) -> Result<(), TsNetError> {
        let network_c = str_to_c(network)?;
        let addr_c = str_to_c(addr)?;

        let mut conn_fd: c_int = 0;
        let err = unsafe {
            libtailscale::tailscale_dial(
                self.server,
                network_c.as_ptr(),
                addr_c.as_ptr(),
                &mut conn_fd,
            )
        };
        if err != 0 {
            return Err(self.read_error());
        }

        let conn_owned = unsafe { OwnedFd::from_raw_fd(conn_fd) };

        unsafe { set_nonblocking(conn_fd) }.map_err(TsNetError::IO)?;

        match handler.on_connect(conn_owned.as_raw_fd()) {
            FdControl::TAKE_OVER => {
                let _ = conn_owned.into_raw_fd();
                return Ok(());
            }
            FdControl::KEEP => {}
        }

        drive_conn(conn_owned.into_raw_fd(), handler).map_err(TsNetError::IO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsnet_error_display() {
        let e = TsNetError::TAILSCALE("oops".to_string());
        assert_eq!(e.to_string(), "tailscale error: oops");

        let io_err = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        let e2 = TsNetError::IO(io_err);
        assert!(e2.to_string().contains("broken pipe"));
    }

    #[test]
    fn set_nonblocking_causes_wouldblock() {
        use std::os::unix::net::UnixStream;
        use std::os::fd::IntoRawFd;

        let (r, _w) = UnixStream::pair().unwrap();
        let fd = r.into_raw_fd();
        unsafe { set_nonblocking(fd).unwrap(); }

        let mut buf = [0u8; 1];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 1) };
        let err = io::Error::last_os_error();
        assert_eq!(n, -1);
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);

        unsafe { libc::close(fd); }
    }

    #[test]
    fn try_flush_writes_pending_data() {
        use std::os::unix::net::UnixStream;
        use std::os::fd::IntoRawFd;
        use std::io::Read;

        let (mut r, w) = UnixStream::pair().unwrap();
        let write_fd = w.into_raw_fd();
        unsafe { set_nonblocking(write_fd).unwrap(); }

        struct OneShot(Option<Vec<u8>>);
        impl ConnectionHandler for OneShot {
            fn on_data(&mut self, _: &[u8]) {}
            fn poll_write(&mut self) -> Option<Vec<u8>> { self.0.take() }
            fn is_done(&self) -> bool { self.0.is_none() }
        }

        let mut state = ConnState {
            fd: write_fd,
            handler: OneShot(Some(b"hello".to_vec())),
            write_buf: VecDeque::new(),
        };

        let wants_writable = try_flush(&mut state).unwrap();
        assert!(!wants_writable, "all bytes should fit in one write");

        let mut buf = [0u8; 5];
        r.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");

        unsafe { libc::close(write_fd); }
    }

    #[test]
    fn default_on_connect_returns_keep() {
        use std::os::unix::net::UnixStream;
        use std::os::fd::IntoRawFd;

        struct MinimalHandler;
        impl ConnectionHandler for MinimalHandler {
            fn on_data(&mut self, _: &[u8]) {}
            fn poll_write(&mut self) -> Option<Vec<u8>> { None }
            fn is_done(&self) -> bool { true }
        }

        let (r, _w) = UnixStream::pair().unwrap();
        let fd = r.into_raw_fd();
        let mut h = MinimalHandler;
        let control = h.on_connect(fd);
        assert!(matches!(control, FdControl::KEEP));
        unsafe { libc::close(fd); }
    }

    #[test]
    fn custom_on_connect_can_return_takeover() {
        use std::os::unix::net::UnixStream;
        use std::os::fd::IntoRawFd;

        struct TakeOverHandler;
        impl ConnectionHandler for TakeOverHandler {
            fn on_connect(&mut self, fd: RawFd) -> FdControl {
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                assert_ne!(flags, -1, "fd passed to on_connect must be valid");
                FdControl::TAKE_OVER
            }
            fn on_data(&mut self, _: &[u8]) {}
            fn poll_write(&mut self) -> Option<Vec<u8>> { None }
            fn is_done(&self) -> bool { true }
        }

        let (r, _w) = UnixStream::pair().unwrap();
        let fd = r.into_raw_fd();
        let mut h = TakeOverHandler;
        let control = h.on_connect(fd);
        assert!(matches!(control, FdControl::TAKE_OVER));
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert_ne!(flags, -1, "fd must remain valid after on_connect returns");
        unsafe { libc::close(fd); }
    }

    #[test]
    fn echo_handler_buffers_and_echoes() {
        struct EchoHandler {
            to_write: Vec<u8>,
            done: bool,
        }
        impl EchoHandler {
            fn new() -> Self { EchoHandler { to_write: vec![], done: false } }
        }
        impl ConnectionHandler for EchoHandler {
            fn on_data(&mut self, data: &[u8]) {
                self.to_write.extend_from_slice(data);
                self.done = true;
            }
            fn poll_write(&mut self) -> Option<Vec<u8>> {
                if self.to_write.is_empty() { None } else { Some(self.to_write.drain(..).collect()) }
            }
            fn is_done(&self) -> bool { self.done && self.to_write.is_empty() }
        }

        let mut h = EchoHandler::new();
        assert!(h.poll_write().is_none());
        h.on_data(b"hello");
        assert_eq!(h.poll_write().unwrap(), b"hello");
        assert!(h.poll_write().is_none());
        assert!(h.is_done());
    }

    #[test]
    fn drive_conn_calls_on_data_until_done() {
        use std::os::unix::net::UnixStream;
        use std::os::fd::IntoRawFd;
        use std::io::Write;

        let (mut client, server_side) = UnixStream::pair().unwrap();
        let server_fd = server_side.into_raw_fd();
        unsafe { set_nonblocking(server_fd).unwrap(); }

        client.write_all(b"ping").unwrap();
        drop(client);

        struct CollectHandler { received: Vec<u8> }
        impl ConnectionHandler for CollectHandler {
            fn on_data(&mut self, data: &[u8]) { self.received.extend_from_slice(data); }
            fn poll_write(&mut self) -> Option<Vec<u8>> { None }
            fn is_done(&self) -> bool { !self.received.is_empty() }
        }

        let result = drive_conn(server_fd, CollectHandler { received: vec![] });
        assert!(result.is_ok());
    }

    #[test]
    fn dial_method_signature_compiles() {
        fn _assert_dial_exists(server: &RawTsNetServer) {
            struct NullHandler;
            impl ConnectionHandler for NullHandler {
                fn on_data(&mut self, _: &[u8]) {}
                fn poll_write(&mut self) -> Option<Vec<u8>> { None }
                fn is_done(&self) -> bool { true }
            }
            let _: Result<(), TsNetError> = server.dial("tcp", "100.64.0.1:80", NullHandler);
        }
    }

    #[test]
    fn drive_conn_sends_poll_write_output() {
        use std::os::unix::net::UnixStream;
        use std::os::fd::IntoRawFd;
        use std::io::{Read, Write};

        let (mut client, server_side) = UnixStream::pair().unwrap();
        let server_fd = server_side.into_raw_fd();
        unsafe { set_nonblocking(server_fd).unwrap(); }

        client.write_all(b"go").unwrap();

        struct ReplyHandler { reply: Option<Vec<u8>>, done: bool }
        impl ConnectionHandler for ReplyHandler {
            fn on_data(&mut self, _: &[u8]) { self.done = true; }
            fn poll_write(&mut self) -> Option<Vec<u8>> { self.reply.take() }
            fn is_done(&self) -> bool { self.done && self.reply.is_none() }
        }

        let handler = ReplyHandler { reply: Some(b"reply".to_vec()), done: false };
        let t = std::thread::spawn(move || drive_conn(server_fd, handler));

        let mut buf = [0u8; 5];
        client.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"reply");
        drop(client);
        t.join().unwrap().unwrap();
    }
}
