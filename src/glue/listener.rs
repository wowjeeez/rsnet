use std::ffi::{c_int, CStr};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use tokio::io::unix::AsyncFd;

use crate::vendor::libtailscale;
use crate::glue::stream::{StreamInfo, TailscaleStream};

pub struct Listener {
    inner: AsyncFd<OwnedFd>,
    port: Option<u16>,
}

impl Listener {
    pub(crate) fn new(fd: RawFd) -> Result<Self, io::Error> {
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { inner: AsyncFd::new(owned)?, port: None })
    }

    pub(crate) fn new_with_port(fd: RawFd, port: u16) -> Result<Self, io::Error> {
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { inner: AsyncFd::new(owned)?, port: Some(port) })
    }

    pub async fn accept(&self) -> io::Result<TailscaleStream> {
        loop {
            let guard = self.inner.readable().await?;

            let listener_fd = self.inner.get_ref().as_raw_fd();
            let mut conn_fd: c_int = 0;
            let res = unsafe { libtailscale::tailscale_accept(listener_fd, &mut conn_fd) };

            if res == 0 {
                let flags = unsafe { libc::fcntl(conn_fd, libc::F_GETFL) };
                if flags == -1 || unsafe { libc::fcntl(conn_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
                    unsafe { libc::close(conn_fd) };
                    return Err(io::Error::last_os_error());
                }

                let peer_addr = get_remote_addr(listener_fd, conn_fd);

                let info = StreamInfo {
                    local_port: self.port,
                    peer_addr,
                };
                return TailscaleStream::from_raw_with_info(conn_fd, info);
            }

            let err = io::Error::last_os_error();
            match err.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                    drop(guard);
                    continue;
                }
                _ => return Err(err),
            }
        }
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.inner.get_ref().as_raw_fd()
    }
}

fn get_remote_addr(listener_fd: RawFd, conn_fd: c_int) -> Option<String> {
    let mut buf = [0i8; 256];
    let res = unsafe {
        libtailscale::tailscale_getremoteaddr(
            listener_fd, conn_fd, buf.as_mut_ptr(), buf.len() - 1,
        )
    };
    if res != 0 {
        return None;
    }
    buf[255] = 0;
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_str()
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}
