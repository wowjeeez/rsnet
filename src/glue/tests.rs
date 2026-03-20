use std::io;
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::glue::error::TsNetError;
use crate::glue::stream::TailscaleStream;
use crate::glue::listener::Listener;
use crate::glue::server::RawTsTcpServer;

fn make_nonblocking_pair() -> (i32, i32) {
    let (a, b) = UnixStream::pair().unwrap();
    let a_fd = a.into_raw_fd();
    let b_fd = b.into_raw_fd();
    unsafe {
        libc::fcntl(a_fd, libc::F_SETFL, libc::fcntl(a_fd, libc::F_GETFL) | libc::O_NONBLOCK);
        libc::fcntl(b_fd, libc::F_SETFL, libc::fcntl(b_fd, libc::F_GETFL) | libc::O_NONBLOCK);
    }
    (a_fd, b_fd)
}

#[test]
fn error_display() {
    let e = TsNetError::Tailscale("oops".into());
    assert_eq!(e.to_string(), "tailscale error: oops");

    let e2 = TsNetError::Io(io::ErrorKind::BrokenPipe.into());
    assert!(e2.to_string().contains("broken pipe"));
}

#[tokio::test]
async fn stream_read_write() {
    let (a, b) = make_nonblocking_pair();
    let mut sa = TailscaleStream::from_raw(a).unwrap();
    let mut sb = TailscaleStream::from_raw(b).unwrap();

    sa.write_all(b"hello").await.unwrap();
    let mut buf = [0u8; 5];
    sb.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello");
}

#[tokio::test]
async fn stream_bidirectional() {
    let (a, b) = make_nonblocking_pair();
    let mut sa = TailscaleStream::from_raw(a).unwrap();
    let mut sb = TailscaleStream::from_raw(b).unwrap();

    sa.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    sb.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");

    sb.write_all(b"pong").await.unwrap();
    sa.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"pong");
}

#[tokio::test]
async fn stream_eof_on_close() {
    let (a, b) = make_nonblocking_pair();
    let mut sa = TailscaleStream::from_raw(a).unwrap();
    let mut sb = TailscaleStream::from_raw(b).unwrap();

    sa.write_all(b"data").await.unwrap();
    drop(sa);

    let mut buf = Vec::new();
    sb.read_to_end(&mut buf).await.unwrap();
    assert_eq!(&buf, b"data");
}

#[tokio::test]
async fn listener_accept_via_pipe() {
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let read_fd = fds[0];
    unsafe {
        libc::fcntl(read_fd, libc::F_SETFL, libc::fcntl(read_fd, libc::F_GETFL) | libc::O_NONBLOCK);
    }
    let listener = Listener::new(read_fd);
    assert!(listener.is_ok());
    unsafe { libc::close(fds[1]) };
}

#[test]
fn close_is_idempotent() {
    let mut server = RawTsTcpServer::for_test(-1);
    assert!(server.close().is_ok());
    assert!(server.close().is_ok());
}
