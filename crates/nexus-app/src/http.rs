//! The real HTTP transport, with `reqwest`. The core checks each plugin's network allowlist
//! before a request gets here.

use async_trait::async_trait;
use futures::StreamExt;
use nexus_api::{HttpRequest, HttpResponse, Method, PluginError, PluginResult};
use nexus_core::{Body, HttpTransport, Opened};

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("launcher/", env!("CARGO_PKG_VERSION")))
            // The core follows redirects itself, checking each hop against the allowlist.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
}

fn network(e: reqwest::Error) -> PluginError {
    PluginError::Network(e.to_string())
}

/// The whole file's length from `Content-Range: bytes 100-199/200`.
fn range_total(response: &reqwest::Response) -> Option<u64> {
    let header = response.headers().get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    header.rsplit('/').next()?.parse().ok()
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> PluginResult<HttpResponse> {
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            Method::Head => reqwest::Method::HEAD,
            Method::Post => reqwest::Method::POST,
        };
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let response = builder.send().await.map_err(network)?;
        let status = response.status().as_u16();
        let headers =
            response.headers().iter().filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_owned()))).collect();
        let body = response.bytes().await.map_err(network)?.to_vec();
        Ok(HttpResponse { status, headers, body })
    }

    async fn open(&self, url: &str, offset: u64) -> PluginResult<Opened> {
        let mut builder = self.client.get(url);
        if offset > 0 {
            builder = builder.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let response = builder.send().await.map_err(network)?;
        if response.status().is_redirection() {
            let location = response.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok());
            let location =
                location.ok_or_else(|| PluginError::Network(format!("a redirect from {url} has no Location")))?;
            return Ok(Opened::Redirect(location.to_owned()));
        }
        // 416: the range starts at the end, so the part already has every byte.
        if offset > 0 && response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            return Ok(Opened::Body(Body { resumed: true, total: None, chunks: futures::stream::empty().boxed() }));
        }
        let response = response.error_for_status().map_err(network)?;
        let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let total = if resumed { range_total(&response) } else { response.content_length() };
        let chunks = response.bytes_stream().map(|chunk| chunk.map(|b| b.to_vec()).map_err(network)).boxed();
        Ok(Opened::Body(Body { resumed, total, chunks }))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    use super::*;

    const FILE: &[u8] = b"0123456789";

    /// A one-file web server on a free port: `/file` honours `Range`, `/moved` redirects to
    /// `/file`. One request a connection, and each answer says so, so the client doesn't send
    /// the next on a closed one. Returns its base URL.
    fn serve() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut range = None;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = value.trim().trim_end_matches('-').parse::<usize>().ok();
                    }
                }
                let reply = if first.starts_with("GET /moved ") {
                    "HTTP/1.1 302 Found\r\nLocation: /file\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                        .as_bytes()
                        .to_vec()
                } else {
                    match range {
                        Some(start) if start >= FILE.len() => {
                            b"HTTP/1.1 416 Range Not Satisfiable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                                .to_vec()
                        }
                        Some(start) => {
                            let body = &FILE[start..];
                            let head = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                                FILE.len() - 1,
                                FILE.len(),
                                body.len()
                            );
                            [head.as_bytes(), body].concat()
                        }
                        None => {
                            let head = format!(
                                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                                FILE.len()
                            );
                            [head.as_bytes(), FILE].concat()
                        }
                    }
                };
                let _ = stream.write_all(&reply);
            }
        });
        base
    }

    async fn body(opened: Opened) -> (bool, Option<u64>, Vec<u8>) {
        let Opened::Body(body) = opened else { panic!("expected a body") };
        let chunks: Vec<_> = body.chunks.collect().await;
        (body.resumed, body.total, chunks.into_iter().flat_map(|c| c.unwrap()).collect())
    }

    #[tokio::test]
    async fn a_range_resumes_a_416_means_complete_and_a_redirect_comes_back_unfollowed() {
        let base = serve();
        let http = ReqwestTransport::new().unwrap();

        assert_eq!(body(http.open(&format!("{base}/file"), 0).await.unwrap()).await, (false, Some(10), FILE.to_vec()));
        assert_eq!(
            body(http.open(&format!("{base}/file"), 4).await.unwrap()).await,
            (true, Some(10), b"456789".to_vec())
        );
        assert_eq!(body(http.open(&format!("{base}/file"), 10).await.unwrap()).await, (true, None, Vec::new()));
        assert!(matches!(http.open(&format!("{base}/moved"), 0).await.unwrap(), Opened::Redirect(to) if to == "/file"));
        let sent = http.send(HttpRequest::get(format!("{base}/moved"))).await.unwrap();
        assert_eq!(sent.status, 302, "send doesn't follow either");
    }
}
