use std::collections::{HashMap, VecDeque};
use std::ffi::{c_int, CStr, CString};
use std::io::{self, BufRead};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};

use crate::vendor::libtailscale;


#[derive(Debug)]
pub enum TsNetError {
    Io(io::Error),
    Tailscale(String),
}

impl std::fmt::Display for TsNetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TsNetError::Io(e) => write!(f, "{e}"),
            TsNetError::Tailscale(msg) => write!(f, "tailscale error: {msg}"),
        }
    }
}

impl std::error::Error for TsNetError {}

impl From<io::Error> for TsNetError {
    fn from(e: io::Error) -> Self {
        TsNetError::Io(e)
    }
}


#[derive(Debug, PartialEq, Eq)]
pub enum FdControl {
    Keep,
    TakeOver,
}

pub trait ConnectionHandler {
    fn on_connect(&mut self, _fd: RawFd) -> FdControl {
        FdControl::Keep
    }
    fn on_data(&mut self, data: &[u8]);
    fn poll_write(&mut self) -> Option<Vec<u8>>;
    fn is_done(&self) -> bool;
}

pub trait HandlerFactory {
    type Handler: ConnectionHandler;
    fn new_handler(&self) -> Self::Handler;
}

pub struct Listener {
    shutdown_fd: OwnedFd,
}

impl Listener {
    pub fn shutdown(&self) -> io::Result<()> {
        let n = unsafe {
            libc::write(self.shutdown_fd.as_raw_fd(), b"x".as_ptr().cast(), 1)
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
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
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn try_flush<H: ConnectionHandler>(state: &mut ConnState<H>) -> io::Result<bool> {
    while let Some(bytes) = state.handler.poll_write() {
        state.write_buf.extend(bytes);
    }
    while !state.write_buf.is_empty() {
        let (front, _) = state.write_buf.as_slices();
        let slice = if front.is_empty() {
            state.write_buf.make_contiguous()
        } else {
            front
        };
        let n = unsafe { libc::write(state.fd, slice.as_ptr().cast(), slice.len()) };
        match n.cmp(&0) {
            std::cmp::Ordering::Greater => { state.write_buf.drain(..n as usize); }
            std::cmp::Ordering::Equal => return Err(io::ErrorKind::BrokenPipe.into()),
            std::cmp::Ordering::Less => {
                let err = io::Error::last_os_error();
                return if err.kind() == io::ErrorKind::WouldBlock { Ok(true) } else { Err(err) };
            }
        }
    }
    Ok(false)
}

// read from fd into handler, flush responses, return true if connection should close.
fn pump_conn<H: ConnectionHandler>(
    state: &mut ConnState<H>,
    readable: bool,
    poll: &Poll,
    token: Token,
) -> bool {
    let mut close = false;

    if readable {
        let mut buf = [0u8; 8192];
        loop {
            let n = unsafe { libc::read(state.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                state.handler.on_data(&buf[..n as usize]);
                if state.handler.is_done() { break; }
            } else if n == 0 {
                close = true;
                break;
            } else {
                if io::Error::last_os_error().kind() != io::ErrorKind::WouldBlock {
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
                if poll.registry().reregister(&mut SourceFd(&state.fd), token, interest).is_err() {
                    close = true;
                }
            }
            Err(_) => close = true,
        }
    }

    close || state.handler.is_done()
}

fn drive_conn<H: ConnectionHandler>(fd: RawFd, handler: H) -> io::Result<()> {
    let _owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(16);
    const CONN: Token = Token(0);
    poll.registry().register(&mut SourceFd(&fd), CONN, Interest::READABLE)?;

    let mut state = ConnState { fd, handler, write_buf: VecDeque::new() };

    loop {
        poll.poll(&mut events, None)?;
        for event in &events {
            if event.token() == CONN && pump_conn(&mut state, event.is_readable(), &poll, CONN) {
                return Ok(());
            }
        }
    }
}

fn str_to_c(s: &str) -> Result<CString, TsNetError> {
    CString::new(s).map_err(|e| TsNetError::Tailscale(e.to_string()))
}

fn read_error_for(handle: c_int) -> TsNetError {
    let mut buf = [0i8; 256];
    unsafe {
        libtailscale::tailscale_errmsg(handle, buf.as_mut_ptr(), buf.len() - 1);
        buf[255] = 0;
        TsNetError::Tailscale(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

macro_rules! ts_call {
    ($handle:expr, $ffi_fn:path $(, $arg:expr)*) => {{
        let rc = unsafe { $ffi_fn($handle $(, $arg)*) };
        if rc != 0 { Err(read_error_for($handle)) } else { Ok(()) }
    }};
}


/// A low level, Mio pooled Tailscale device node backed via C FFI.
/// `Send`-safe.
pub struct RawTsTcpServer {
    handle: c_int,
}

unsafe impl Send for RawTsTcpServer {}

impl RawTsTcpServer {
    pub fn new(hostname: &str) -> Result<Self, TsNetError> {
        let handle = unsafe { libtailscale::tailscale_new() };
        let s = RawTsTcpServer { handle };
        s.set_hostname(hostname)?;
        s.capture_logs()?;
        Ok(s)
    }

    fn capture_logs(&self) -> Result<(), TsNetError> {
        let mut fds = [0i32; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let read_fd = fds[0];
        let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

        self.set_log_fd(write_fd)?;

        std::thread::spawn(move || {
            let file = unsafe { std::fs::File::from_raw_fd(read_fd) };
            let reader = io::BufReader::new(file);
            for line in reader.lines() {
                match line {
                    Ok(msg) if !msg.is_empty() => tracing::debug!(target: "libtailscale", "{msg}"),
                    Err(_) => break,
                    _ => {}
                }
            }
        });

        Ok(())
    }

    pub fn set_hostname(&self, hostname: &str) -> Result<(), TsNetError> {
        ts_call!(self.handle, libtailscale::tailscale_set_hostname, str_to_c(hostname)?.as_ptr())
    }

    pub fn set_control_url(&self, url: &str) -> Result<(), TsNetError> {
        ts_call!(self.handle, libtailscale::tailscale_set_control_url, str_to_c(url)?.as_ptr())
    }

    pub fn set_auth_key(&self, key: &str) -> Result<(), TsNetError> {
        ts_call!(self.handle, libtailscale::tailscale_set_authkey, str_to_c(key)?.as_ptr())
    }

    pub fn set_dir(&self, dir: &str) -> Result<(), TsNetError> {
        ts_call!(self.handle, libtailscale::tailscale_set_dir, str_to_c(dir)?.as_ptr())
    }

    pub fn set_ephemeral(&self, on: bool) -> Result<(), TsNetError> {
        ts_call!(self.handle, libtailscale::tailscale_set_ephemeral, c_int::from(on))
    }

    pub fn set_log_fd(&self, fd: OwnedFd) -> Result<(), TsNetError> {
        ts_call!(self.handle, libtailscale::tailscale_set_logfd, fd.into_raw_fd())
    }

    pub fn start(&self) -> Result<(), TsNetError> {
        ts_call!(self.handle, libtailscale::tailscale_start)
    }

    pub fn up(&self) -> Result<(), TsNetError> {
        ts_call!(self.handle, libtailscale::tailscale_up)
    }

    /// Returns Tailscale IPs as a comma-separated string (e.g. `"100.64.0.1,fd7a::1"`).
    pub fn getips(&self) -> Result<String, TsNetError> {
        let mut buf = [0i8; 256];
        unsafe {
            let err = libtailscale::tailscale_getips(self.handle, buf.as_mut_ptr(), buf.len());
            if err != 0 {
                return Err(read_error_for(self.handle));
            }
            Ok(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    pub fn close(&mut self) -> Result<(), TsNetError> {
        if self.handle == -1 {
            return Ok(());
        }
        let h = self.handle;
        self.handle = -1;
        let rc = unsafe { libtailscale::tailscale_close(h) };
        if rc != 0 {
            return Err(TsNetError::Tailscale("tailscale_close failed (see tailscale logs)".into()));
        }
        Ok(())
    }

    /// Start accepting connections on the tailnet. Returns immediately;
    /// the accept loop runs on a background thread. Drop the [`Listener`] to stop it.
    pub fn listen<F: HandlerFactory + Send + 'static>(
        &self,
        network: &str,
        addr: &str,
        factory: F,
    ) -> Result<Listener, TsNetError>
    where
        F::Handler: Send,
    {
        let network_c = str_to_c(network)?;
        let addr_c = str_to_c(addr)?;

        let mut listener_fd: c_int = 0;
        let err = unsafe {
            libtailscale::tailscale_listen(
                self.handle, network_c.as_ptr(), addr_c.as_ptr(), &mut listener_fd,
            )
        };
        if err != 0 {
            return Err(read_error_for(self.handle));
        }

        unsafe { set_nonblocking(listener_fd) }?;

        let mut pipe_fds = [0i32; 2];
        if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
            unsafe { libc::close(listener_fd) };
            return Err(io::Error::last_os_error().into());
        }
        let shutdown_read = pipe_fds[0];
        let shutdown_write = unsafe { OwnedFd::from_raw_fd(pipe_fds[1]) };
        unsafe { set_nonblocking(shutdown_read) }?;

        let server_handle = self.handle;

        std::thread::spawn(move || {
            let _listener_guard = unsafe { OwnedFd::from_raw_fd(listener_fd) };
            let _shutdown_guard = unsafe { OwnedFd::from_raw_fd(shutdown_read) };

            let Ok(mut poll) = Poll::new() else { return };
            let mut events = Events::with_capacity(128);

            const LISTENER: Token = Token(0);
            const SHUTDOWN: Token = Token(usize::MAX);
            let _ = poll.registry().register(&mut SourceFd(&listener_fd), LISTENER, Interest::READABLE);
            let _ = poll.registry().register(&mut SourceFd(&shutdown_read), SHUTDOWN, Interest::READABLE);

            let mut conns: HashMap<Token, ConnState<F::Handler>> = HashMap::new();
            let mut next_token: usize = 1;

            'outer: loop {
                if poll.poll(&mut events, None).is_err() { break; }

                for event in &events {
                    match event.token() {
                        SHUTDOWN => {
                            for (_, st) in conns.drain() {
                                let _ = poll.registry().deregister(&mut SourceFd(&st.fd));
                                unsafe { libc::close(st.fd) };
                            }
                            break 'outer;
                        }
                        LISTENER => {
                            loop {
                                let mut conn_fd: c_int = 0;
                                if unsafe { libtailscale::tailscale_accept(listener_fd, &mut conn_fd) } != 0 {
                                    break; // EWOULDBLOCK or real error
                                }
                                if unsafe { set_nonblocking(conn_fd) }.is_err() {
                                    unsafe { libc::close(conn_fd) };
                                    continue;
                                }

                                while next_token == 0 || next_token == usize::MAX || conns.contains_key(&Token(next_token)) {
                                    next_token = next_token.wrapping_add(1);
                                }
                                let token = Token(next_token);
                                next_token = next_token.wrapping_add(1);
                                if next_token == 0 { next_token = 1; }

                                let conn_owned = unsafe { OwnedFd::from_raw_fd(conn_fd) };
                                if poll.registry().register(&mut SourceFd(&conn_fd), token, Interest::READABLE).is_err() {
                                    continue; // ownedFd closes fd on drop
                                }
                                let mut handler = factory.new_handler();
                                match handler.on_connect(conn_owned.as_raw_fd()) {
                                    FdControl::Keep => {
                                        conns.insert(token, ConnState {
                                            fd: conn_owned.into_raw_fd(),
                                            handler,
                                            write_buf: VecDeque::new(),
                                        });
                                    }
                                    FdControl::TakeOver => {
                                        let fd = conn_owned.into_raw_fd();
                                        let _ = poll.registry().deregister(&mut SourceFd(&fd));
                                    }
                                }
                            }
                        }
                        token => {
                            let close = conns.get_mut(&token)
                                .is_some_and(|st| pump_conn(st, event.is_readable(), &poll, token));
                            if close && let Some(st) = conns.remove(&token) {
                                let _ = poll.registry().deregister(&mut SourceFd(&st.fd));
                                unsafe { libc::close(st.fd) };
                            }
                        }
                    }
                }
            }

            let _ = (&_listener_guard, &_shutdown_guard, &server_handle);
        });

        Ok(Listener { shutdown_fd: shutdown_write })
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
                self.handle, network_c.as_ptr(), addr_c.as_ptr(), &mut conn_fd,
            )
        };
        if err != 0 {
            return Err(read_error_for(self.handle));
        }

        let conn_owned = unsafe { OwnedFd::from_raw_fd(conn_fd) };
        unsafe { set_nonblocking(conn_fd) }?;

        match handler.on_connect(conn_owned.as_raw_fd()) {
            FdControl::TakeOver => {
                let _ = conn_owned.into_raw_fd(); // caller owns it now
                Ok(())
            }
            FdControl::Keep => drive_conn(conn_owned.into_raw_fd(), handler).map_err(Into::into),
        }
    }
}

impl Drop for RawTsTcpServer {
    fn drop(&mut self) {
        let _ = self.close();
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::IntoRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn error_display() {
        let e = TsNetError::Tailscale("oops".into());
        assert_eq!(e.to_string(), "tailscale error: oops");

        let e2 = TsNetError::Io(io::ErrorKind::BrokenPipe.into());
        assert!(e2.to_string().contains("broken pipe"));
    }

    #[test]
    fn set_nonblocking_causes_wouldblock() {
        let (r, _w) = UnixStream::pair().unwrap();
        let fd = r.into_raw_fd();
        unsafe { set_nonblocking(fd).unwrap() };

        let mut buf = [0u8; 1];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), 1) };
        assert_eq!(n, -1);
        assert_eq!(io::Error::last_os_error().kind(), io::ErrorKind::WouldBlock);
        unsafe { libc::close(fd) };
    }

    #[test]
    fn try_flush_writes_pending_data() {
        let (mut r, w) = UnixStream::pair().unwrap();
        let wfd = w.into_raw_fd();
        unsafe { set_nonblocking(wfd).unwrap() };

        struct OneShot(Option<Vec<u8>>);
        impl ConnectionHandler for OneShot {
            fn on_data(&mut self, _: &[u8]) {}
            fn poll_write(&mut self) -> Option<Vec<u8>> { self.0.take() }
            fn is_done(&self) -> bool { self.0.is_none() }
        }

        let mut state = ConnState { fd: wfd, handler: OneShot(Some(b"hello".to_vec())), write_buf: VecDeque::new() };
        assert!(!try_flush(&mut state).unwrap());

        let mut buf = [0u8; 5];
        r.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        unsafe { libc::close(wfd) };
    }

    #[test]
    fn default_on_connect_returns_keep() {
        struct H;
        impl ConnectionHandler for H {
            fn on_data(&mut self, _: &[u8]) {}
            fn poll_write(&mut self) -> Option<Vec<u8>> { None }
            fn is_done(&self) -> bool { true }
        }
        let (r, _w) = UnixStream::pair().unwrap();
        let fd = r.into_raw_fd();
        assert_eq!(H.on_connect(fd), FdControl::Keep);
        unsafe { libc::close(fd) };
    }

    #[test]
    fn on_connect_takeover_keeps_fd_valid() {
        struct H;
        impl ConnectionHandler for H {
            fn on_connect(&mut self, fd: RawFd) -> FdControl {
                assert_ne!(unsafe { libc::fcntl(fd, libc::F_GETFL) }, -1);
                FdControl::TakeOver
            }
            fn on_data(&mut self, _: &[u8]) {}
            fn poll_write(&mut self) -> Option<Vec<u8>> { None }
            fn is_done(&self) -> bool { true }
        }
        let (r, _w) = UnixStream::pair().unwrap();
        let fd = r.into_raw_fd();
        assert_eq!(H.on_connect(fd), FdControl::TakeOver);
        assert_ne!(unsafe { libc::fcntl(fd, libc::F_GETFL) }, -1);
        unsafe { libc::close(fd) };
    }

    #[test]
    fn echo_handler() {
        struct Echo { buf: Vec<u8>, done: bool }
        impl ConnectionHandler for Echo {
            fn on_data(&mut self, d: &[u8]) { self.buf.extend_from_slice(d); self.done = true; }
            fn poll_write(&mut self) -> Option<Vec<u8>> {
                if self.buf.is_empty() { None } else { Some(self.buf.drain(..).collect()) }
            }
            fn is_done(&self) -> bool { self.done && self.buf.is_empty() }
        }

        let mut h = Echo { buf: vec![], done: false };
        assert!(h.poll_write().is_none());
        h.on_data(b"hello");
        assert_eq!(h.poll_write().unwrap(), b"hello");
        assert!(h.is_done());
    }

    #[test]
    fn drive_conn_reads_until_done() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let fd = server.into_raw_fd();
        unsafe { set_nonblocking(fd).unwrap() };

        client.write_all(b"ping").unwrap();
        drop(client);

        struct Collect(Vec<u8>);
        impl ConnectionHandler for Collect {
            fn on_data(&mut self, d: &[u8]) { self.0.extend_from_slice(d); }
            fn poll_write(&mut self) -> Option<Vec<u8>> { None }
            fn is_done(&self) -> bool { !self.0.is_empty() }
        }

        assert!(drive_conn(fd, Collect(vec![])).is_ok());
    }

    #[test]
    fn drive_conn_sends_reply() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let fd = server.into_raw_fd();
        unsafe { set_nonblocking(fd).unwrap() };

        client.write_all(b"go").unwrap();

        struct Reply { data: Option<Vec<u8>>, done: bool }
        impl ConnectionHandler for Reply {
            fn on_data(&mut self, _: &[u8]) { self.done = true; }
            fn poll_write(&mut self) -> Option<Vec<u8>> { self.data.take() }
            fn is_done(&self) -> bool { self.done && self.data.is_none() }
        }

        let t = std::thread::spawn(move || drive_conn(fd, Reply { data: Some(b"reply".to_vec()), done: false }));

        let mut buf = [0u8; 5];
        client.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"reply");
        drop(client);
        t.join().unwrap().unwrap();
    }

    #[test]
    fn listener_drop_signals_eof() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let read_fd = fds[0];
        let listener = Listener { shutdown_fd: unsafe { OwnedFd::from_raw_fd(fds[1]) } };

        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 1];
            let n = unsafe { libc::read(read_fd, buf.as_mut_ptr().cast(), 1) };
            unsafe { libc::close(read_fd) };
            n
        });

        drop(listener);
        assert_eq!(t.join().unwrap(), 0); // EOF
    }

    #[test]
    fn listener_explicit_shutdown() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let read_fd = fds[0];
        let listener = Listener { shutdown_fd: unsafe { OwnedFd::from_raw_fd(fds[1]) } };

        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 1];
            let n = unsafe { libc::read(read_fd, buf.as_mut_ptr().cast(), 1) };
            unsafe { libc::close(read_fd) };
            (n, buf[0])
        });

        listener.shutdown().unwrap();
        let (n, byte) = t.join().unwrap();
        assert_eq!((n, byte), (1, b'x'));
    }

    #[test]
    fn close_is_idempotent() {
        let mut server = RawTsTcpServer { handle: -1 };
        assert!(server.close().is_ok());
        assert!(server.close().is_ok());
    }
}
