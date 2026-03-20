use std::io;
use std::sync::Arc;

use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use crate::glue::listener::Listener;
use crate::glue::stream::TailscaleStream;

pub struct TlsListener {
    inner: Listener,
    acceptor: TlsAcceptor,
}

impl TlsListener {
    pub fn new(listener: Listener, config: Arc<ServerConfig>) -> Self {
        Self {
            inner: listener,
            acceptor: TlsAcceptor::from(config),
        }
    }

    pub fn from_pem(listener: Listener, cert_pem: &[u8], key_pem: &[u8]) -> io::Result<Self> {
        let config = tls_config_from_pem(cert_pem, key_pem)?;
        Ok(Self::new(listener, config))
    }

    pub async fn accept(&self) -> io::Result<TlsStream<TailscaleStream>> {
        let stream = self.inner.accept().await?;
        self.acceptor.accept(stream).await
    }
}

pub fn tls_config_from_pem(cert_pem: &[u8], key_pem: &[u8]) -> io::Result<Arc<ServerConfig>> {
    let certs = rustls_pemfile::certs(&mut &*cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| io::Error::other(format!("bad cert pem: {e}")))?;

    let key = rustls_pemfile::private_key(&mut &*key_pem)
        .map_err(|e| io::Error::other(format!("bad key pem: {e}")))?
        .ok_or_else(|| io::Error::other("no private key found in pem"))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| io::Error::other(format!("tls config error: {e}")))?;

    Ok(Arc::new(config))
}
