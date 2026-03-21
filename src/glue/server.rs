use std::ffi::{c_int, CStr, CString};
use std::io::{self, BufRead};
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd, RawFd};

use crate::vendor::libtailscale;
use crate::glue::error::TsNetError;
use crate::glue::listener::Listener;
use crate::glue::localapi::LocalClient;
use crate::glue::stream::TailscaleStream;

fn str_to_c(s: &str) -> Result<CString, TsNetError> {
    CString::new(s).map_err(|e| TsNetError::Tailscale(e.to_string()))
}

fn read_error_for(handle: c_int) -> TsNetError {
    let mut buf = [0i8; 256];
    unsafe {
        libtailscale::tailscale_errmsg(handle, buf.as_mut_ptr(), buf.len() - 1);
        buf[255] = 0; // errmsg may not null-terminate if buffer is too small
        TsNetError::Tailscale(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

macro_rules! ts_call {
    ($handle:expr, $ffi_fn:path $(, $arg:expr)*) => {{
        let rc = unsafe { $ffi_fn($handle $(, $arg)*) };
        if rc != 0 { Err(read_error_for($handle)) } else { Ok(()) }
    }};
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

pub struct RawTsTcpServer {
    handle: c_int,
}

// handle is just an integer key into a global map on the go side
unsafe impl Send for RawTsTcpServer {}

impl RawTsTcpServer {
    pub fn new(hostname: &str) -> Result<Self, TsNetError> {
        let handle = unsafe { libtailscale::tailscale_new() };
        let s = RawTsTcpServer { handle };
        s.set_hostname(hostname)?;
        s.capture_logs()?;
        Ok(s)
    }

    // pipe go-side logs through tracing::debug via a reader thread
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

    pub fn loopback(&self) -> Result<(String, String, String), TsNetError> {
        let mut addr_buf = [0i8; 256];
        let mut proxy_buf = [0i8; 33];
        let mut local_buf = [0i8; 33];
        unsafe {
            let err = libtailscale::tailscale_loopback(
                self.handle,
                addr_buf.as_mut_ptr(),
                addr_buf.len(),
                proxy_buf.as_mut_ptr(),
                local_buf.as_mut_ptr(),
            );
            if err != 0 {
                return Err(read_error_for(self.handle));
            }
            let addr = CStr::from_ptr(addr_buf.as_ptr()).to_string_lossy().into_owned();
            let proxy_cred = CStr::from_ptr(proxy_buf.as_ptr()).to_string_lossy().into_owned();
            let local_cred = CStr::from_ptr(local_buf.as_ptr()).to_string_lossy().into_owned();
            Ok((addr, proxy_cred, local_cred))
        }
    }

    pub fn enable_funnel(&self, localhost_port: u16) -> Result<(), TsNetError> {
        ts_call!(
            self.handle,
            libtailscale::tailscale_enable_funnel_to_localhost_plaintext_http1,
            localhost_port as c_int
        )
    }

    // starts the loopback server and returns a client authed to the localapi
    pub fn local_client(&self) -> Result<LocalClient, TsNetError> {
        let (addr, _proxy_cred, local_api_cred) = self.loopback()?;
        Ok(LocalClient::new(addr, local_api_cred))
    }

    pub fn close(&mut self) -> Result<(), TsNetError> {
        if self.handle == -1 {
            return Ok(());
        }
        let h = self.handle;
        // mark closed before calling go it deletes the handle from its map,
        // so read_error_for(h) would return EBADF after this point
        self.handle = -1;
        let rc = unsafe { libtailscale::tailscale_close(h) };
        if rc != 0 {
            return Err(TsNetError::Tailscale("tailscale_close failed (see tailscale logs)".into()));
        }
        Ok(())
    }

    pub fn listen(&self, network: &str, addr: &str) -> Result<Listener, TsNetError> {
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
        Ok(Listener::new(listener_fd)?)
    }

    // native tls listener — go handles certs automatically via tailscale ACME
    // the returned fd is already tls-terminated, no rustls needed
    pub fn listen_native_tls(&self, network: &str, addr: &str) -> Result<Listener, TsNetError> {
        let network_c = str_to_c(network)?;
        let addr_c = str_to_c(addr)?;

        let mut listener_fd: c_int = 0;
        let err = unsafe {
            libtailscale::tailscale_listen_tls(
                self.handle, network_c.as_ptr(), addr_c.as_ptr(), &mut listener_fd,
            )
        };
        if err != 0 {
            return Err(read_error_for(self.handle));
        }

        unsafe { set_nonblocking(listener_fd) }?;
        Ok(Listener::new(listener_fd)?)
    }

    // tailscale services — advertises as a named service, returns listener + fqdn
    pub fn listen_service(
        &self,
        service_name: &str,
        service_mode: &str,
        port: u16,
        https: bool,
        terminate_tls: bool,
    ) -> Result<(Listener, String), TsNetError> {
        let name_c = str_to_c(service_name)?;
        let mode_c = str_to_c(service_mode)?;

        let mut listener_fd: c_int = 0;
        let mut fqdn_buf = [0i8; 256];
        let err = unsafe {
            libtailscale::tailscale_listen_service(
                self.handle,
                name_c.as_ptr(),
                mode_c.as_ptr(),
                port as c_int,
                c_int::from(https),
                c_int::from(terminate_tls),
                &mut listener_fd,
                fqdn_buf.as_mut_ptr(),
                fqdn_buf.len(),
            )
        };
        if err != 0 {
            return Err(read_error_for(self.handle));
        }

        unsafe { set_nonblocking(listener_fd) }?;
        let fqdn = unsafe {
            std::ffi::CStr::from_ptr(fqdn_buf.as_ptr())
                .to_string_lossy()
                .into_owned()
        };
        Ok((Listener::new(listener_fd)?, fqdn))
    }

    pub fn dial(&self, network: &str, addr: &str) -> Result<TailscaleStream, TsNetError> {
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

        unsafe { set_nonblocking(conn_fd) }?;
        TailscaleStream::from_raw(conn_fd).map_err(Into::into)
    }
}

#[cfg(test)]
impl RawTsTcpServer {
    pub(crate) fn for_test(handle: c_int) -> Self {
        RawTsTcpServer { handle }
    }
}

impl Drop for RawTsTcpServer {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
