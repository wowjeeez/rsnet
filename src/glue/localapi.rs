use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use base64::Engine as _;

#[cfg(feature = "localapi-serde-json")]
use crate::glue::types::*;

pub struct LocalClient {
    addr: String,
    auth_header: String,
}

impl LocalClient {
    pub fn new(addr: String, local_api_cred: String) -> Self {
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!(":{local_api_cred}"));
        Self {
            addr,
            auth_header: format!("Basic {encoded}"),
        }
    }

    async fn request(&self, method: &str, path: &str) -> io::Result<(u16, Vec<u8>)> {
        let mut stream = TcpStream::connect(&self.addr).await?;

        let req = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: {}\r\n\
             t\r\n\
             Authorization: {}\r\n\
             Connection: close\r\n\
             \r\n",
            self.addr, self.auth_header,
        );
        stream.write_all(req.as_bytes()).await?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;

        let header_end = find_header_end(&raw)
            .ok_or_else(|| io::Error::other("no header/body separator in response"))?;
        let head = &raw[..header_end];

        let status = std::str::from_utf8(head)
            .ok()
            .and_then(|s| s.lines().next())
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        let body_start = header_end + 4;
        let raw_body = if body_start <= raw.len() { &raw[body_start..] } else { &[] as &[u8] };

        let is_chunked = std::str::from_utf8(head)
            .ok()
            .map(|h| h.to_ascii_lowercase().contains("transfer-encoding: chunked"))
            .unwrap_or(false);

        let body = if is_chunked {
            decode_chunked(raw_body)?
        } else {
            raw_body.to_vec()
        };

        Ok((status, body))
    }

    async fn post_body(&self, path: &str, body: &[u8]) -> io::Result<(u16, Vec<u8>)> {
        let mut stream = TcpStream::connect(&self.addr).await?;

        let req = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: {}\r\n\
             Sec-Tailscale: localapi\r\n\
             Authorization: {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            self.addr, self.auth_header, body.len(),
        );
        stream.write_all(req.as_bytes()).await?;
        stream.write_all(body).await?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;

        let header_end = find_header_end(&raw)
            .ok_or_else(|| io::Error::other("no header/body separator"))?;
        let head = &raw[..header_end];

        let status = std::str::from_utf8(head)
            .ok()
            .and_then(|s| s.lines().next())
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        let body_start = header_end + 4;
        let raw_body = if body_start <= raw.len() { &raw[body_start..] } else { &[] as &[u8] };

        let is_chunked = std::str::from_utf8(head)
            .ok()
            .map(|h| h.to_ascii_lowercase().contains("transfer-encoding: chunked"))
            .unwrap_or(false);

        let resp_body = if is_chunked { decode_chunked(raw_body)? } else { raw_body.to_vec() };
        Ok((status, resp_body))
    }

    pub async fn get(&self, path: &str) -> io::Result<(u16, Vec<u8>)> {
        self.request("GET", path).await
    }

    pub async fn post(&self, path: &str) -> io::Result<(u16, Vec<u8>)> {
        self.request("POST", path).await
    }

    pub async fn status_raw(&self) -> io::Result<Vec<u8>> {
        let (code, body) = self.get("/localapi/v0/status").await?;
        if code != 200 {
            return Err(io::Error::other(format!("status returned {code}")));
        }
        Ok(body)
    }

    #[cfg(feature = "localapi-serde-json")]
    pub async fn status(&self) -> io::Result<Status> {
        let body = self.status_raw().await?;
        serde_json::from_slice(&body).map_err(|e| io::Error::other(e.to_string()))
    }

    #[cfg(feature = "localapi-serde-json")]
    pub async fn whoami(&self) -> io::Result<PeerStatus> {
        let status = self.status().await?;
        Ok(status.self_node)
    }

    #[cfg(feature = "localapi-serde-json")]
    pub async fn fqdn(&self) -> io::Result<String> {
        let me = self.whoami().await?;
        me.dns_name
            .map(|s| s.trim_end_matches('.').to_string())
            .ok_or_else(|| io::Error::other("no DNSName in self node"))
    }

    #[cfg(feature = "localapi-serde-json")]
    pub async fn whois(&self, addr: &str) -> io::Result<WhoIsResponse> {
        let (code, body) = self.get(&format!("/localapi/v0/whois?addr={addr}")).await?;
        if code != 200 {
            return Err(io::Error::other(format!("whois returned {code}")));
        }
        serde_json::from_slice(&body).map_err(|e| io::Error::other(e.to_string()))
    }

    #[cfg(feature = "localapi-serde-json")]
    pub async fn prefs(&self) -> io::Result<Prefs> {
        let (code, body) = self.get("/localapi/v0/prefs").await?;
        if code != 200 {
            return Err(io::Error::other(format!("prefs returned {code}")));
        }
        serde_json::from_slice(&body).map_err(|e| io::Error::other(e.to_string()))
    }

    pub async fn advertise_exit_node(&self) -> io::Result<()> {
        self.advertise_routes(&["0.0.0.0/0", "::/0"]).await
    }

    pub async fn stop_advertising_exit_node(&self) -> io::Result<()> {
        self.advertise_routes(&[]).await
    }

    pub async fn advertise_routes(&self, routes: &[&str]) -> io::Result<()> {
        let routes_json: Vec<String> = routes.iter().map(|r| format!("\"{r}\"")).collect();
        let body = format!(
            "{{\"AdvertiseRoutes\":[{}]}}",
            routes_json.join(",")
        );
        let (code, resp) = self.post_body("/localapi/v0/prefs", body.as_bytes()).await?;
        if code != 200 {
            return Err(io::Error::other(
                format!("prefs returned {code}: {}", String::from_utf8_lossy(&resp)),
            ));
        }
        Ok(())
    }

    pub async fn use_exit_node(&self, ip: &str) -> io::Result<()> {
        let (code, resp) = self.post(
            &format!("/localapi/v0/set-use-exit-node-enabled?enabled=true&exit_node_ip={ip}")
        ).await?;
        if code != 200 {
            return Err(io::Error::other(
                format!("set-use-exit-node returned {code}: {}", String::from_utf8_lossy(&resp)),
            ));
        }
        Ok(())
    }

    pub async fn stop_using_exit_node(&self) -> io::Result<()> {
        let (code, resp) = self.post("/localapi/v0/set-use-exit-node-enabled?enabled=false").await?;
        if code != 200 {
            return Err(io::Error::other(
                format!("set-use-exit-node returned {code}: {}", String::from_utf8_lossy(&resp)),
            ));
        }
        Ok(())
    }

    pub async fn cert(&self, domain: &str) -> io::Result<Vec<u8>> {
        let (code, body) = self.get(&format!("/localapi/v0/cert/{domain}")).await?;
        if code != 200 {
            return Err(io::Error::other(
                format!("cert returned {code}: {}", String::from_utf8_lossy(&body)),
            ));
        }
        Ok(body)
    }

    pub async fn cert_key(&self, domain: &str) -> io::Result<Vec<u8>> {
        let (code, body) = self.get(&format!("/localapi/v0/cert/{domain}?type=key")).await?;
        if code != 200 {
            return Err(io::Error::other(
                format!("cert key returned {code}: {}", String::from_utf8_lossy(&body)),
            ));
        }
        Ok(body)
    }

    pub async fn cert_pair(&self, domain: &str) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let cert = self.cert(domain).await?;
        let key = self.cert_key(domain).await?;
        Ok((cert, key))
    }
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|w| w == b"\r\n\r\n")
}

fn decode_chunked(mut data: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n")
            .ok_or_else(|| io::Error::other("malformed chunk: no CRLF after size"))?;
        let size_str = std::str::from_utf8(&data[..line_end])
            .map_err(|e| io::Error::other(e.to_string()))?
            .trim();
        let chunk_size = usize::from_str_radix(size_str, 16)
            .map_err(|e| io::Error::other(format!("bad chunk size '{size_str}': {e}")))?;
        if chunk_size == 0 {
            break;
        }
        let chunk_start = line_end + 2;
        let chunk_end = chunk_start + chunk_size;
        if chunk_end > data.len() {
            return Err(io::Error::other("chunk size exceeds available data"));
        }
        out.extend_from_slice(&data[chunk_start..chunk_end]);
        data = &data[chunk_end + 2..];
    }
    Ok(out)
}
