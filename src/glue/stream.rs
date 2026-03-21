use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub struct StreamInfo {
    pub local_port: Option<u16>,
    pub peer_addr: Option<String>,
}

pub struct TailscaleStream {
    inner: AsyncFd<OwnedFd>,
    info: StreamInfo,
}

impl TailscaleStream {
    pub fn from_raw(fd: RawFd) -> io::Result<Self> {
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self {
            inner: AsyncFd::new(owned)?,
            info: StreamInfo { local_port: None, peer_addr: None },
        })
    }

    pub(crate) fn from_raw_with_info(fd: RawFd, info: StreamInfo) -> io::Result<Self> {
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { inner: AsyncFd::new(owned)?, info })
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.inner.get_ref().as_raw_fd()
    }

    pub fn local_port(&self) -> Option<u16> {
        self.info.local_port
    }

    pub fn peer_addr(&self) -> Option<&str> {
        self.info.peer_addr.as_deref()
    }

    pub async fn readable(&self) -> io::Result<()> {
        self.inner.readable().await?.retain_ready();
        Ok(())
    }

    pub async fn writable(&self) -> io::Result<()> {
        self.inner.writable().await?.retain_ready();
        Ok(())
    }

    pub fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let fd = self.inner.get_ref().as_raw_fd();
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n >= 0 { Ok(n as usize) } else { Err(io::Error::last_os_error()) }
    }

    pub fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        let fd = self.inner.get_ref().as_raw_fd();
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n >= 0 { Ok(n as usize) } else { Err(io::Error::last_os_error()) }
    }
}

impl AsyncRead for TailscaleStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut guard = match self.inner.poll_read_ready(cx) {
                Poll::Ready(Ok(g)) => g,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };

            let fd = self.inner.get_ref().as_raw_fd();
            let unfilled = buf.initialize_unfilled();
            let n = unsafe { libc::read(fd, unfilled.as_mut_ptr().cast(), unfilled.len()) };

            if n >= 0 {
                buf.advance(n as usize);
                return Poll::Ready(Ok(()));
            }

            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                guard.clear_ready();
                continue;
            }
            return Poll::Ready(Err(err));
        }
    }
}

impl AsyncWrite for TailscaleStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = match self.inner.poll_write_ready(cx) {
                Poll::Ready(Ok(g)) => g,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };

            let fd = self.inner.get_ref().as_raw_fd();
            let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };

            if n >= 0 {
                return Poll::Ready(Ok(n as usize));
            }

            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                guard.clear_ready();
                continue;
            }
            return Poll::Ready(Err(err));
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let fd = self.inner.get_ref().as_raw_fd();
        if unsafe { libc::shutdown(fd, libc::SHUT_WR) } < 0 {
            return Poll::Ready(Err(io::Error::last_os_error()));
        }
        Poll::Ready(Ok(()))
    }
}
