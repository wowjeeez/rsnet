use std::io;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::glue::error::TsNetError;
use crate::glue::server::RawTsTcpServer;
use crate::glue::stream::TailscaleStream;

pub enum ServiceMode {
    Http { https: bool },
    Tcp { terminate_tls: bool },
}

pub struct ServiceBuilder<'a> {
    server: &'a RawTsTcpServer,
    name: String,
    ports: Vec<(ServiceMode, u16)>,
}

impl<'a> ServiceBuilder<'a> {
    pub(crate) fn new(server: &'a RawTsTcpServer, name: &str) -> Self {
        Self { server, name: name.to_string(), ports: Vec::new() }
    }

    pub fn http(mut self, port: u16) -> Self {
        self.ports.push((ServiceMode::Http { https: false }, port));
        self
    }

    pub fn https(mut self, port: u16) -> Self {
        self.ports.push((ServiceMode::Http { https: true }, port));
        self
    }

    pub fn tcp(mut self, port: u16) -> Self {
        self.ports.push((ServiceMode::Tcp { terminate_tls: false }, port));
        self
    }

    pub fn tcp_tls(mut self, port: u16) -> Self {
        self.ports.push((ServiceMode::Tcp { terminate_tls: true }, port));
        self
    }

    pub fn bind(self) -> Result<Service, TsNetError> {
        if self.ports.is_empty() {
            return Err(TsNetError::Tailscale("no ports configured".into()));
        }

        let mut listeners = Vec::new();
        let mut fqdn = String::new();

        for (mode, port) in &self.ports {
            let (mode_str, https, terminate_tls) = match mode {
                ServiceMode::Http { https } => ("http", *https, false),
                ServiceMode::Tcp { terminate_tls } => ("tcp", false, *terminate_tls),
            };
            let (listener, f) = self.server.listen_service(
                &self.name, mode_str, *port, https, terminate_tls,
            )?;
            fqdn = f;
            listeners.push((*port, Arc::new(listener)));
        }

        let (tx, rx) = mpsc::channel(64);

        for (port, listener) in &listeners {
            let tx = tx.clone();
            let port = *port;
            let listener = Arc::clone(listener);
            tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok(stream) => {
                            if tx.send((port, stream)).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => continue,
                    }
                }
            });
        }
        drop(tx);

        Ok(Service { name: self.name, fqdn, rx })
    }
}

pub struct Service {
    pub name: String,
    pub fqdn: String,
    rx: mpsc::Receiver<(u16, TailscaleStream)>,
}

impl Service {
    pub async fn accept(&mut self) -> io::Result<(u16, TailscaleStream)> {
        self.rx.recv().await
            .ok_or_else(|| io::Error::other("all service listeners closed"))
    }
}
