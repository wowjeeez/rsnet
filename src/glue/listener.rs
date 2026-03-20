use std::ffi::c_int;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use tokio::io::unix::AsyncFd;

use crate::vendor::libtailscale;
use crate::glue::stream::TailscaleStream;

pub struct Listener {
    inner: AsyncFd<OwnedFd>,
}

impl Listener {
    pub(crate) fn new(fd: RawFd) -> Result<Self, io::Error> {
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { inner: AsyncFd::new(owned)? })
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
                return TailscaleStream::from_raw(conn_fd);
            }

            let err = io::Error::last_os_error();
            match err.kind() {
                // no connection ready yet, or transient timeout on the socketpair
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                    drop(guard);
                    continue;
                }
                _ => return Err(err),
            }
        }
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.inner.get_ref().as_raw_fd()
    }
}
