use std::collections::{HashMap, VecDeque};
use std::net::TcpListener;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

#[derive(Clone)]
struct StubKnowledgeHttpResponse {
    status: u16,
    content_type: &'static str,
    body: String,
}

impl StubKnowledgeHttpResponse {
    fn json_ok(body: String) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body,
        }
    }
}

pub struct KnowledgeStubServer {
    base_url: String,
    responses: Arc<std::sync::Mutex<HashMap<String, VecDeque<StubKnowledgeHttpResponse>>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for KnowledgeStubServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnowledgeStubServer")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl KnowledgeStubServer {
    pub fn start() -> anyhow::Result<Self> {
        let listener =
            TcpListener::bind("127.0.0.1:0").context("binding knowledge stub server")?;
        let local_addr = listener.local_addr().context("reading knowledge stub server addr")?;
        let base_url = format!("http://127.0.0.1:{port}", port = local_addr.port());
        let responses = Arc::new(std::sync::Mutex::new(HashMap::<
            String,
            VecDeque<StubKnowledgeHttpResponse>,
        >::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let responses_for_thread = Arc::clone(&responses);
        let shutdown_for_thread = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            serve_knowledge_stub(listener, responses_for_thread, shutdown_for_thread);
        });

        Ok(Self {
            base_url,
            responses,
            shutdown,
            handle: Some(handle),
        })
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    pub fn enqueue_json(&self, request_target: impl Into<String>, body: serde_json::Value) {
        let mut responses = self.responses.lock().expect("knowledge stub queue lock");
        responses
            .entry(request_target.into())
            .or_default()
            .push_back(StubKnowledgeHttpResponse::json_ok(body.to_string()));
    }
}

impl Drop for KnowledgeStubServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(
            self.base_url
                .strip_prefix("http://")
                .unwrap_or(self.base_url.as_str()),
        );
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve_knowledge_stub(
    listener: TcpListener,
    responses: Arc<std::sync::Mutex<HashMap<String, VecDeque<StubKnowledgeHttpResponse>>>>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(err) = respond_to_stub_request(&mut stream, &responses) {
                    eprintln!("knowledge stub request handling failed: {err:#}");
                }
            }
            Err(_) => break,
        }
    }
}

fn respond_to_stub_request(
    stream: &mut std::net::TcpStream,
    responses: &Arc<std::sync::Mutex<HashMap<String, VecDeque<StubKnowledgeHttpResponse>>>>,
) -> anyhow::Result<()> {
    let request_target = read_request_target(stream)?;
    let response = {
        let mut guard = responses.lock().expect("knowledge stub response lock");
        match guard
            .get_mut(&request_target)
            .and_then(|queue| queue.pop_front())
        {
            Some(response) => response,
            None => StubKnowledgeHttpResponse {
                status: 404,
                content_type: "text/plain; charset=utf-8",
                body: format!("no stub response queued for {request_target}"),
            },
        }
    };
    write_http_response(stream, &response)
}

fn read_request_target(stream: &mut std::net::TcpStream) -> anyhow::Result<String> {
    let mut buffer = Vec::with_capacity(8192);
    let mut chunk = [0u8; 1024];
    loop {
        let bytes_read =
            std::io::Read::read(stream, &mut chunk).context("reading knowledge stub request")?;
        if bytes_read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() >= 8192 {
            anyhow::bail!("knowledge stub request exceeded 8192 bytes before header completion");
        }
    }
    if buffer.is_empty() {
        anyhow::bail!("knowledge stub received empty request");
    }
    let request = String::from_utf8_lossy(&buffer);
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("knowledge stub missing request line"))?;
    let mut parts = first_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("knowledge stub missing request method"))?;
    let target = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("knowledge stub missing request target"))?;
    if method != "GET" {
        anyhow::bail!("knowledge stub only supports GET requests, got `{method}`");
    }
    Ok(target.to_string())
}

fn write_http_response(
    stream: &mut std::net::TcpStream,
    response: &StubKnowledgeHttpResponse,
) -> anyhow::Result<()> {
    let reason = match response.status {
        200 => "OK",
        404 => "Not Found",
        other => {
            return Err(anyhow::anyhow!(
                "knowledge stub encountered unsupported status code `{other}`"
            ));
        }
    };
    let payload = response.body.as_bytes();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        payload.len()
    );
    std::io::Write::write_all(stream, head.as_bytes())
        .context("writing knowledge stub response head")?;
    std::io::Write::write_all(stream, payload)
        .context("writing knowledge stub response body")?;
    std::io::Write::flush(stream).context("flushing knowledge stub response")?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .context("shutting down knowledge stub response stream")
}

#[cfg(test)]
mod knowledge_stub_tests {
    use super::KnowledgeStubServer;
    use anyhow::{Context, Result};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn stub_handles_request_headers_arriving_in_multiple_chunks() -> Result<()> {
        let server = KnowledgeStubServer::start()?;
        server.enqueue_json(
            "/wiki/rest/api/content/1002?expand=body.storage,version",
            serde_json::json!({
                "title": "Beta knowledge page",
                "version": {
                    "when": "2026-04-17T08:00:00Z",
                    "by": { "displayName": "QAT Docs" }
                },
                "body": {
                    "storage": {
                        "value": "<p>Beta reference content</p>"
                    }
                }
            }),
        );

        let mut stream = TcpStream::connect(
            server
                .base_url()
                .strip_prefix("http://")
                .context("knowledge stub base_url should be http")?,
        )?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(b"GET ")?;
        stream.flush()?;
        thread::sleep(Duration::from_millis(25));
        stream.write_all(
            b"/wiki/rest/api/content/1002?expand=body.storage,version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )?;
        stream.flush()?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;

        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "expected a successful stub response, got: {response}"
        );
        assert!(
            response.contains("Beta knowledge page"),
            "expected queued JSON body in response, got: {response}"
        );
        Ok(())
    }
}
