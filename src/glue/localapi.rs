use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use base64::Engine as _;

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
             Sec-Tailscale: localapi\r\n\
             Authorization: {}\r\n\
             Connection: close\r\n\
             \r\n",
            self.addr, self.auth_header,
        );
        stream.write_all(req.as_bytes()).await?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;

        let raw_str = String::from_utf8_lossy(&raw);
        let (head, body) = raw_str
            .split_once("\r\n\r\n")
            .unwrap_or((&raw_str, ""));

        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        // find the body bytes in the original raw response
        let header_len = head.len() + 4; // +4 for \r\n\r\n
        let body_bytes = if header_len <= raw.len() {
            raw[header_len..].to_vec()
        } else {
            body.as_bytes().to_vec()
        };

        Ok((status, body_bytes))
    }

    pub async fn get(&self, path: &str) -> io::Result<(u16, Vec<u8>)> {
        self.request("GET", path).await
    }

    pub async fn post(&self, path: &str) -> io::Result<(u16, Vec<u8>)> {
        self.request("POST", path).await
    }

    pub async fn status(&self) -> io::Result<Vec<u8>> {
        let (code, body) = self.get("/localapi/v0/status").await?;
        if code != 200 {
            return Err(io::Error::other(format!("status returned {code}")));
        }
        Ok(body)
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

    pub async fn whoami(&self) -> io::Result<String> {
        let status = self.status().await?;
        let json = String::from_utf8_lossy(&status);

        // DISGUSTING string parsing to avoid a serde dep — just extract the "Self":{...} block
        let self_start = json.find("\"Self\":{")
            .ok_or_else(|| io::Error::other("no Self key in status response"))?;
        let obj_start = self_start + "\"Self\":".len();
        let mut depth = 0;
        let mut end = obj_start;
        for (i, c) in json[obj_start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = obj_start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }

        Ok(json[obj_start..end].to_string())
    }

    pub async fn whois(&self, addr: &str) -> io::Result<Vec<u8>> {
        let (code, body) = self.get(&format!("/localapi/v0/whois?addr={addr}")).await?;
        if code != 200 {
            return Err(io::Error::other(format!("whois returned {code}")));
        }
        Ok(body)
    }
}
