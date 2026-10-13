//! Shared HTTPS download: resumes only a known version of the same URL, retries
//! transient failures and draws a progress line on stderr.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use ureq::tls::{PemItem, RootCerts, TlsConfig};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{Connector, DefaultConnector};
use ureq::Agent;

/// Extra attempts after the first one fails (curl --retry 3)
const RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(2);

/// Download `url` into `out`, resuming what is already there. Returns whether the
/// file is complete; failures are reported on stderr (like curl) so the caller
/// can try the next mirror.
pub fn fetch(out: &Path, url: &str) -> Result<bool> {
    fetch_with(&agent(Agent::config_builder())?, out, url, RETRY_DELAY)
}

/// Agent with curl-like timeouts. The proxy comes from the usual environment
/// variables (HTTPS_PROXY, NO_PROXY, …); roots are Mozilla's bundled ones, or
/// those in $SSL_CERT_FILE when set (as with curl/OpenSSL).
fn agent(config: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>) -> Result<Agent> {
    agent_with_idle(config, Duration::from_secs(30))
}

fn agent_with_idle(
    config: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>,
    idle: Duration,
) -> Result<Agent> {
    let mut config = config
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_send_request(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("qwen-translate/", env!("CARGO_PKG_VERSION")));
    if let Some(file) = std::env::var_os("SSL_CERT_FILE") {
        let pem = std::fs::read(&file)
            .with_context(|| format!("failed to read SSL_CERT_FILE {}", file.to_string_lossy()))?;
        let certs: Vec<_> = ureq::tls::parse_pem(&pem)
            .filter_map(|item| match item {
                Ok(PemItem::Certificate(c)) => Some(c),
                _ => None,
            })
            .collect();
        config = config.tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::new_with_certs(&certs))
                .build(),
        );
    }
    Ok(Agent::with_parts(
        config.build(),
        DefaultConnector::default().chain(super::idle::IdleConnector(idle)),
        DefaultResolver::default(),
    ))
}

#[derive(Serialize, Deserialize)]
struct Resume {
    url: String,
    etag: String,
    total: Option<u64>,
}

pub(super) fn resume_path(out: &Path) -> PathBuf {
    out.with_extension("tmp.resume.json")
}

/// The caller that publishes a model must know the complete response length.
/// A sidecar is retained even without an ETag for this one-shot fact; its empty
/// validator deliberately never authorizes a later resume.
pub(super) fn known_total(out: &Path) -> Option<u64> {
    std::fs::read(resume_path(out))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Resume>(&bytes).ok())
        .and_then(|r| r.total)
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn discard_partial(out: &Path) -> Result<()> {
    // Remove identity first: a crash before removing bytes must not permit resume.
    remove_if_present(&resume_path(out))?;
    remove_if_present(out)
}

fn strong_etag(value: &str) -> bool {
    value.starts_with('"')
        && value.ends_with('"')
        && value.len() >= 2
        && value[1..value.len() - 1]
            .bytes()
            .all(|b| b == 0x21 || (0x23..=0x7e).contains(&b) || b >= 0x80)
}

fn save_resume(out: &Path, resume: &Resume) -> Result<()> {
    let mut file = File::create(resume_path(out))?;
    file.write_all(&serde_json::to_vec(resume)?)?;
    file.sync_all()?;
    Ok(())
}

/// Outcome of one request
enum Attempt {
    Done,
    /// A valid 206 supplied only part of the requested suffix. Continue from the
    /// new offset without spending the network-failure retry budget.
    Continue,
    /// Worth retrying (network error, 5xx, cut-off body, …)
    Retry(String),
    /// Retrying won't help (404 and other client errors)
    Fail(String),
}

fn fetch_with(agent: &Agent, out: &Path, url: &str, delay: Duration) -> Result<bool> {
    let mut attempt = 0;
    loop {
        match try_once(agent, out, url)? {
            Attempt::Done => return Ok(true),
            Attempt::Continue => attempt = 0,
            Attempt::Fail(e) => {
                eprintln!("error: {e}");
                return Ok(false);
            }
            Attempt::Retry(e) if attempt < RETRIES => {
                attempt += 1;
                eprintln!(
                    "warning: {e}; retrying in {}s ({attempt}/{RETRIES})",
                    delay.as_secs()
                );
                std::thread::sleep(delay);
            }
            Attempt::Retry(e) => {
                eprintln!("error: {e}");
                return Ok(false);
            }
        }
    }
}

fn try_once(agent: &Agent, out: &Path, url: &str) -> Result<Attempt> {
    let existing = std::fs::metadata(out).map_or(0, |m| m.len());
    let resume = std::fs::read(resume_path(out))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Resume>(&bytes).ok())
        .filter(|r| {
            r.url == url
                && strong_etag(&r.etag)
                && existing > 0
                && r.total.is_none_or(|total| existing <= total)
        });
    // Legacy partials, a new mirror and weak/no validators require a fresh body.
    let offset = if resume.is_some() { existing } else { 0 };
    let mut req = agent.get(url).header("Accept-Encoding", "identity");
    if offset > 0 {
        req = req
            .header("Range", format!("bytes={offset}-"))
            .header("If-Range", &resume.as_ref().unwrap().etag);
    }
    let resp = match req.call() {
        Ok(r) => r,
        // The partial file is no prefix of the remote one (e.g. longer): start over
        Err(ureq::Error::StatusCode(416)) if offset > 0 => {
            discard_partial(out)?;
            return Ok(Attempt::Retry("cannot resume the partial download".into()));
        }
        Err(ureq::Error::StatusCode(code)) if code == 408 || code == 429 || code >= 500 => {
            return Ok(Attempt::Retry(format!("HTTP {code} from {url}")));
        }
        Err(ureq::Error::StatusCode(code)) => {
            return Ok(Attempt::Fail(format!("HTTP {code} from {url}")))
        }
        Err(e) => return Ok(Attempt::Retry(e.to_string())),
    };

    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok());
    if header("content-encoding").is_some_and(|v| !v.eq_ignore_ascii_case("identity")) {
        return Ok(Attempt::Fail("unexpected encoded model response".into()));
    }
    let etag = header("etag").filter(|v| strong_etag(v)).map(str::to_owned);
    let (mut file, start, total, response_end) = match resp.status().as_u16() {
        206 => match header("content-range").and_then(parse_content_range) {
            Some((first, last, Some(total)))
                if first == offset
                    && offset > 0
                    && etag.as_deref() == resume.as_ref().map(|r| r.etag.as_str())
                    && resume.as_ref().unwrap().total.is_none_or(|t| t == total)
                    && header("content-length").is_none_or(|length| {
                        length.parse::<u64>().ok() == Some(last - first + 1)
                    }) =>
            {
                (
                    OpenOptions::new().append(true).open(out)?,
                    offset,
                    Some(total),
                    Some(last + 1),
                )
            }
            _ => {
                discard_partial(out)?;
                return Ok(Attempt::Retry("unexpected partial response".into()));
            }
        },
        // If-Range mismatch or ignored Range: replace, never append the new version.
        200 => {
            let total = header("content-length").and_then(|v| v.parse().ok());
            remove_if_present(&resume_path(out))?;
            let file = File::create(out)?;
            file.sync_all()?;
            (file, 0, total, total)
        }
        code => return Ok(Attempt::Fail(format!("unexpected HTTP {code} from {url}"))),
    };

    if let Some(etag) = etag {
        save_resume(
            out,
            &Resume {
                url: url.into(),
                etag,
                total,
            },
        )?;
    } else {
        save_resume(
            out,
            &Resume {
                url: url.into(),
                etag: String::new(),
                total,
            },
        )?;
    }

    let mut body = resp.into_body().into_reader();
    let mut progress = Progress::new(start, total);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                progress.finish();
                return Ok(Attempt::Retry(format!("download interrupted: {e}")));
            }
        };
        // The HTTP body framing and Content-Range are independent. A malformed
        // chunked/close-delimited 206 must not append bytes outside its range.
        if response_end.is_some_and(|end| progress.done.saturating_add(n as u64) > end) {
            drop(file);
            discard_partial(out)?;
            progress.finish();
            return Ok(Attempt::Retry("body exceeds the declared range".into()));
        }
        file.write_all(&buf[..n])
            .with_context(|| format!("failed to write {}", out.display()))?;
        progress.advance(n as u64);
    }
    file.sync_all()?;
    progress.finish();

    let len = file.metadata()?.len();
    if start > 0 && response_end == Some(len) && total.is_some_and(|t| len < t) {
        return Ok(Attempt::Continue);
    }
    match total {
        Some(t) if len != t => Ok(Attempt::Retry(format!(
            "download incomplete ({len} of {t} bytes)"
        ))),
        _ => Ok(Attempt::Done),
    }
}

/// "bytes 100-999/1000" -> (100, 999, Some(1000)). An unknown total can be
/// parsed, but cannot establish that a resumed model is complete.
fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let (range, total) = v.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = range.split_once('-')?;
    let first: u64 = first.parse().ok()?;
    let last: u64 = last.parse().ok()?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse::<u64>().ok()?)
    };
    if first > last || total.is_some_and(|n| last >= n) {
        return None;
    }
    Some((first, last, total))
}

/// One-line progress meter on stderr, redrawn in place with \r
struct Progress {
    done: u64,
    total: Option<u64>,
    started: Instant,
    received: u64,
    drawn: Option<Instant>,
}

impl Progress {
    const INTERVAL: Duration = Duration::from_millis(250);

    fn new(done: u64, total: Option<u64>) -> Self {
        Self {
            done,
            total,
            started: Instant::now(),
            received: 0,
            drawn: None,
        }
    }

    fn advance(&mut self, n: u64) {
        self.done += n;
        self.received += n;
        if self.drawn.is_none_or(|t| t.elapsed() >= Self::INTERVAL) {
            self.draw();
        }
    }

    fn draw(&mut self) {
        const MIB: f64 = 1024.0 * 1024.0;
        let secs = self.started.elapsed().as_secs_f64().max(0.001);
        let speed = self.received as f64 / MIB / secs;
        let done = self.done as f64 / MIB;
        let line = match self.total {
            Some(t) if t > 0 => format!(
                "{:5.1}%  {done:7.1} / {:.1} MiB  {speed:5.1} MiB/s",
                self.done as f64 * 100.0 / t as f64,
                t as f64 / MIB
            ),
            _ => format!("{done:7.1} MiB  {speed:5.1} MiB/s"),
        };
        // Trailing spaces wipe leftovers of a longer previous line
        eprint!("\r{line}    ");
        self.drawn = Some(Instant::now());
    }

    fn finish(&mut self) {
        if self.drawn.is_some() {
            self.draw();
            eprintln!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const DATA: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

    /// What the test server does with a request carrying `range` (first byte asked for)
    type Handler = dyn Fn(usize, Option<u64>) -> Vec<u8> + Send + Sync;

    #[derive(Default)]
    struct Request {
        range: Option<u64>,
        if_range: Option<String>,
        encoding: Option<String>,
    }

    type StreamHandler = dyn Fn(usize, Request, TcpStream) + Send + Sync;

    /// Serves one response per connection, then closes it. Returns the base URL
    /// and the number of requests seen.
    fn server(handler: Box<Handler>) -> (String, Arc<AtomicUsize>) {
        stream_server(Box::new(move |n, request, mut stream| {
            let _ = stream.write_all(&handler(n, request.range));
        }))
    }

    fn stream_server(handler: Box<StreamHandler>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model.gguf", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let handler: Arc<StreamHandler> = handler.into();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = io::BufReader::new(stream.try_clone().unwrap());
                let mut request = Request::default();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("range: bytes=") {
                        request.range = v.trim().trim_end_matches('-').parse().ok();
                    } else if lower.starts_with("if-range:") {
                        request.if_range = Some(line.split_once(':').unwrap().1.trim().into());
                    } else if let Some(v) = lower.strip_prefix("accept-encoding:") {
                        request.encoding = Some(v.trim().into());
                    }
                }
                let n = seen.fetch_add(1, Ordering::SeqCst);
                let handler = handler.clone();
                std::thread::spawn(move || handler(n, request, stream));
            }
        });
        (url, count)
    }

    fn full() -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n",
            DATA.len()
        )
        .into_bytes();
        r.extend_from_slice(DATA);
        r
    }

    fn partial(from: u64) -> Vec<u8> {
        let from = from as usize;
        let mut r = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\
             Content-Range: bytes {from}-{}/{}\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n",
            DATA.len() - from,
            DATA.len() - 1,
            DATA.len()
        )
        .into_bytes();
        r.extend_from_slice(&DATA[from..]);
        r
    }

    fn status(code: u16) -> Vec<u8> {
        format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
    }

    fn run(handler: Box<Handler>, existing: Option<&[u8]>) -> (bool, Vec<u8>, usize) {
        let (url, count) = server(handler);
        run_at(&url, count, existing, |out, url| {
            if existing.is_some() {
                save_resume(
                    out,
                    &Resume {
                        url: url.into(),
                        etag: "\"fixture-v1\"".into(),
                        total: None,
                    },
                )
                .unwrap();
            }
        })
    }

    fn run_at(
        url: &str,
        count: Arc<AtomicUsize>,
        existing: Option<&[u8]>,
        prepare: impl FnOnce(&Path, &str),
    ) -> (bool, Vec<u8>, usize) {
        let dir = std::env::temp_dir().join(format!(
            "translate-https-{}-{}",
            std::process::id(),
            url.rsplit(':').next().unwrap().replace('/', "_")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("model.gguf.tmp");
        let _ = std::fs::remove_file(&out);
        if let Some(bytes) = existing {
            std::fs::write(&out, bytes).unwrap();
        }
        prepare(&out, url);
        let agent = agent(Agent::config_builder().proxy(None)).unwrap();
        let ok = fetch_with(&agent, &out, url, Duration::ZERO).unwrap();
        let got = std::fs::read(&out).unwrap_or_default();
        std::fs::remove_dir_all(&dir).unwrap();
        (ok, got, count.load(Ordering::SeqCst))
    }

    #[test]
    fn downloads_whole_file() {
        let (ok, got, requests) = run(Box::new(|_, _| full()), None);
        assert!(ok);
        assert_eq!(got, DATA);
        assert_eq!(requests, 1);
    }

    #[test]
    fn resumes_partial_file() {
        let handler = |_, range: Option<u64>| partial(range.expect("no Range header"));
        let (ok, got, _) = run(Box::new(handler), Some(&DATA[..10]));
        assert!(ok);
        assert_eq!(got, DATA);
    }

    #[test]
    fn restarts_when_range_is_ignored() {
        let (ok, got, _) = run(Box::new(|_, _| full()), Some(b"stale bytes"));
        assert!(ok);
        assert_eq!(got, DATA);
    }

    #[test]
    fn resumes_after_connection_drop() {
        let handler = |n, range: Option<u64>| match (n, range) {
            // Promise the whole file, deliver half of it, hang up
            (0, _) => full()[..full().len() - DATA.len() / 2].to_vec(),
            (_, Some(from)) => partial(from),
            _ => status(500),
        };
        let (ok, got, requests) = run(Box::new(handler), None);
        assert!(ok);
        assert_eq!(got, DATA);
        assert_eq!(requests, 2);
    }

    #[test]
    fn starts_over_when_partial_cannot_be_resumed() {
        let handler = |_, range: Option<u64>| match range {
            Some(_) => status(416),
            None => full(),
        };
        let (ok, got, requests) = run(Box::new(handler), Some(&[b'x'; 64]));
        assert!(ok);
        assert_eq!(got, DATA);
        assert_eq!(requests, 2);
    }

    #[test]
    fn client_errors_are_not_retried() {
        let (ok, _, requests) = run(Box::new(|_, _| status(404)), None);
        assert!(!ok);
        assert_eq!(requests, 1);
    }

    #[test]
    fn server_errors_are_retried() {
        let (ok, _, requests) = run(Box::new(|_, _| status(503)), None);
        assert!(!ok);
        assert_eq!(requests, 1 + RETRIES as usize);
    }

    #[test]
    fn changed_version_replaces_old_prefix_and_sends_if_range() {
        let (url, count) = stream_server(Box::new(|_, request, mut stream| {
            assert_eq!(request.range, Some(10));
            assert_eq!(request.if_range.as_deref(), Some("\"old-version\""));
            assert_eq!(request.encoding.as_deref(), Some("identity"));
            stream.write_all(&full()).unwrap();
        }));
        let (ok, got, requests) = run_at(&url, count, Some(b"old prefix"), |out, url| {
            save_resume(
                out,
                &Resume {
                    url: url.into(),
                    etag: "\"old-version\"".into(),
                    total: Some(DATA.len() as u64),
                },
            )
            .unwrap();
        });
        assert!(ok);
        assert_eq!(got, DATA);
        assert_eq!(requests, 1);
    }

    #[test]
    fn unverified_partials_are_never_resumed() {
        // Legacy partial, weak validator, changed mirror, corrupt/truncated
        // sidecar and impossible length all require a full response.
        for case in 0..5 {
            let (url, count) = stream_server(Box::new(|_, request, mut stream| {
                assert_eq!(request.range, None);
                assert_eq!(request.if_range, None);
                stream.write_all(&full()).unwrap();
            }));
            let (ok, got, requests) =
                run_at(&url, count, Some(b"old prefix"), |out, url| match case {
                    0 => {}
                    1 | 2 | 4 => save_resume(
                        out,
                        &Resume {
                            url: if case == 2 {
                                format!("{url}?different-mirror")
                            } else {
                                url.into()
                            },
                            etag: if case == 1 {
                                "W/\"fixture-v1\""
                            } else {
                                "\"fixture-v1\""
                            }
                            .into(),
                            total: if case == 4 { Some(1) } else { None },
                        },
                    )
                    .unwrap(),
                    _ => std::fs::write(resume_path(out), b"{\"url\":").unwrap(),
                });
            assert!(ok, "case {case}");
            assert_eq!(got, DATA, "case {case}");
            assert_eq!(requests, 1);
        }
    }

    #[test]
    fn missing_or_weak_response_validator_forces_full_retry() {
        for validator in ["", "ETag: W/\"fixture-v1\"\r\n"] {
            let handler = move |n, range| {
                assert_eq!(range, None, "cannot resume an unvalidated response");
                if n == 0 {
                    let mut response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{validator}Connection: close\r\n\r\n", DATA.len()).into_bytes();
                    response.extend_from_slice(&DATA[..10]);
                    response
                } else {
                    full()
                }
            };
            let (ok, got, requests) = run(Box::new(handler), None);
            assert!(ok);
            assert_eq!(got, DATA);
            assert_eq!(requests, 2);
        }
    }

    #[test]
    fn invalid_partial_responses_restart_without_mixing_bytes() {
        for headers in [
            "Content-Range: bytes 10-35/36\r\nETag: \"changed-version\"\r\n",
            "Content-Range: bytes 10-35/36\r\nETag: W/\"fixture-v1\"\r\n",
            "Content-Range: bytes 10-35/36\r\n",
            "Content-Range: bytes 11-35/36\r\nETag: \"fixture-v1\"\r\n",
            "Content-Range: bytes 10-35/*\r\nETag: \"fixture-v1\"\r\n",
            "Content-Range: bytes 10-35/35\r\nETag: \"fixture-v1\"\r\n",
            "Content-Range: bytes 10-9/36\r\nETag: \"fixture-v1\"\r\n",
            "Content-Range: bytes 10-35/36\r\nETag: \"fixture-v1\"\r\nContent-Length: 25\r\n",
            // No Content-Length: the stream itself overflows the declared span.
            "Content-Range: bytes 10-19/36\r\nETag: \"fixture-v1\"\r\n",
        ] {
            let handler = move |n, range| {
                if n == 0 {
                    assert_eq!(range, Some(10));
                    let mut response = format!(
                        "HTTP/1.1 206 Partial Content\r\n{headers}Connection: close\r\n\r\n"
                    )
                    .into_bytes();
                    response.extend_from_slice(&DATA[10..]);
                    response
                } else {
                    assert_eq!(range, None);
                    full()
                }
            };
            let (ok, got, requests) = run(Box::new(handler), Some(&DATA[..10]));
            assert!(ok, "{headers}");
            assert_eq!(got, DATA, "{headers}");
            assert_eq!(requests, 2);
        }
    }

    #[test]
    fn inconsistent_complete_length_restarts_even_with_same_etag() {
        let (url, count) = server(Box::new(|n, range| {
            if n == 0 {
                assert_eq!(range, Some(10));
                partial(10)
            } else {
                assert_eq!(range, None);
                full()
            }
        }));
        let (ok, got, requests) = run_at(&url, count, Some(&DATA[..10]), |out, url| {
            save_resume(
                out,
                &Resume {
                    url: url.into(),
                    etag: "\"fixture-v1\"".into(),
                    total: Some(40),
                },
            )
            .unwrap();
        });
        assert!(ok);
        assert_eq!(got, DATA);
        assert_eq!(requests, 2);
    }

    #[test]
    fn server_may_deliver_a_shorter_valid_range() {
        let handler = |n, range: Option<u64>| {
            let start = range.unwrap() as usize;
            assert_eq!(start, 10 + n * 4);
            let end = (start + 4).min(DATA.len());
            let mut response = format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n", end - 1, DATA.len(), end - start).into_bytes();
            response.extend_from_slice(&DATA[start..end]);
            response
        };
        let (ok, got, requests) = run(Box::new(handler), Some(&DATA[..10]));
        assert!(ok);
        assert_eq!(got, DATA);
        assert_eq!(
            requests, 7,
            "successful partial responses must not consume the retry budget"
        );
    }

    fn temporary_output(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "translate-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("model.gguf")
    }

    #[test]
    fn stalled_body_times_out_and_retries() {
        let (url, count) = stream_server(Box::new(|_, _, mut stream| {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\na")
                .unwrap();
            std::thread::sleep(Duration::from_secs(2));
        }));
        let out = temporary_output("idle");
        let agent = agent_with_idle(
            Agent::config_builder().proxy(None),
            Duration::from_millis(100),
        )
        .unwrap();
        let start = Instant::now();
        assert!(!fetch_with(&agent, &out, &url, Duration::ZERO).unwrap());
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(count.load(Ordering::SeqCst), 1 + RETRIES as usize);
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn active_transfer_may_last_longer_than_idle_timeout() {
        let (url, count) = stream_server(Box::new(|_, _, mut stream| {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                DATA.len()
            )
            .unwrap();
            for chunk in DATA.chunks(3) {
                stream.write_all(chunk).unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }
        }));
        let out = temporary_output("slow");
        let idle = Duration::from_millis(300);
        let agent = agent_with_idle(Agent::config_builder().proxy(None), idle).unwrap();
        let start = Instant::now();
        assert!(fetch_with(&agent, &out, &url, Duration::ZERO).unwrap());
        assert!(start.elapsed() > idle);
        assert_eq!(std::fs::read(&out).unwrap(), DATA);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn concurrent_downloads_publish_only_one_complete_file() {
        let model = b"GGUF\x03\0\0\0fixture bytes";
        let (url, count) = stream_server(Box::new(move |_, request, mut stream| {
            assert_eq!(request.range, None);
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n", model.len()).unwrap();
            stream.write_all(&model[..8]).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            stream.write_all(&model[8..]).unwrap();
        }));
        let out = temporary_output("locked");
        let start = Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let (start, out, url) = (start.clone(), out.clone(), url.clone());
                std::thread::spawn(move || {
                    let agent = agent(Agent::config_builder().proxy(None)).unwrap();
                    start.wait();
                    super::super::download_urls_with(&out, &[url], |out, url| {
                        fetch_with(&agent, out, url, Duration::ZERO)
                    })
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        assert_eq!(std::fs::read(&out).unwrap(), model);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(!out.with_extension("gguf.tmp").exists());
        assert!(!resume_path(&out.with_extension("gguf.tmp")).exists());
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn mirror_fallback_does_not_resume_another_urls_partial() {
        let (first, first_count) = server(Box::new(|n, _| {
            if n == 0 {
                full()[..full().len() - 10].to_vec()
            } else {
                status(404)
            }
        }));
        let model = b"GGUF\x03\0\0\0fresh mirror bytes";
        let (second, second_count) = stream_server(Box::new(move |_, request, mut stream| {
            assert_eq!(request.range, None);
            assert_eq!(request.if_range, None);
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n", model.len()).unwrap();
            stream.write_all(model).unwrap();
        }));
        let out = temporary_output("mirror");
        let agent = agent(Agent::config_builder().proxy(None)).unwrap();
        super::super::download_urls_with(&out, &[first, second], |out, url| {
            fetch_with(&agent, out, url, Duration::ZERO)
        })
        .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), model);
        assert_eq!(first_count.load(Ordering::SeqCst), 2);
        assert_eq!(second_count.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn invalid_model_body_is_never_published() {
        let (url, count) = server(Box::new(|_, _| full()));
        let out = temporary_output("invalid");
        let agent = agent(Agent::config_builder().proxy(None)).unwrap();
        assert!(
            super::super::download_urls_with(&out, &[url], |out, url| fetch_with(
                &agent,
                out,
                url,
                Duration::ZERO
            ))
            .is_err()
        );
        assert!(!out.exists());
        assert!(!out.with_extension("gguf.tmp").exists());
        assert!(!resume_path(&out.with_extension("gguf.tmp")).exists());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn publication_rejects_unknown_length_gguf_prefix() {
        for etag in ["", "ETag: \"fixture-v1\"\r\n"] {
            let (url, count) = server(Box::new(move |_, _| {
                let mut response =
                    format!("HTTP/1.1 200 OK\r\n{etag}Connection: close\r\n\r\n").into_bytes();
                response.extend_from_slice(b"GGUF\x03\0\0\0possibly truncated model");
                response
            }));
            let out = temporary_output("unknown-length");
            let agent = agent(Agent::config_builder().proxy(None)).unwrap();
            assert!(super::super::download_urls_with(&out, &[url], |out, url| {
                fetch_with(&agent, out, url, Duration::ZERO)
            })
            .is_err());
            assert_eq!(count.load(Ordering::SeqCst), 1);
            assert!(!out.exists());
            assert!(!out.with_extension("gguf.tmp").exists());
            assert!(!resume_path(&out.with_extension("gguf.tmp")).exists());
            std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn publication_accepts_known_length_without_etag() {
        let model = b"GGUF\x03\0\0\0complete unversioned model";
        let (url, count) = server(Box::new(move |_, _| {
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                model.len()
            )
            .into_bytes();
            response.extend_from_slice(model);
            response
        }));
        let out = temporary_output("known-unversioned");
        let agent = agent(Agent::config_builder().proxy(None)).unwrap();
        super::super::download_urls_with(&out, &[url], |out, url| {
            let completed = fetch_with(&agent, out, url, Duration::ZERO)?;
            assert_eq!(known_total(out), Some(model.len() as u64));
            let metadata: Resume =
                serde_json::from_slice(&std::fs::read(resume_path(out))?).unwrap();
            assert!(
                !strong_etag(&metadata.etag),
                "length alone must never authorize resume"
            );
            Ok(completed)
        })
        .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), model);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(!out.with_extension("gguf.tmp").exists());
        assert!(!resume_path(&out.with_extension("gguf.tmp")).exists());
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn publication_falls_back_from_unknown_length_to_complete_mirror() {
        let (first, first_count) = server(Box::new(|_, _| {
            b"HTTP/1.1 200 OK\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\nGGUF\x03\0\0\0incomplete prefix".to_vec()
        }));
        let model = b"GGUF\x03\0\0\0complete replacement model";
        let (second, second_count) = stream_server(Box::new(move |_, request, mut stream| {
            assert_eq!(request.range, None);
            assert_eq!(request.if_range, None);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                model.len()
            )
            .unwrap();
            stream.write_all(model).unwrap();
        }));
        let out = temporary_output("unknown-mirror");
        let agent = agent(Agent::config_builder().proxy(None)).unwrap();
        super::super::download_urls_with(&out, &[first, second], |out, url| {
            fetch_with(&agent, out, url, Duration::ZERO)
        })
        .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), model);
        assert_eq!(first_count.load(Ordering::SeqCst), 1);
        assert_eq!(second_count.load(Ordering::SeqCst), 1);
        assert!(!out.with_extension("gguf.tmp").exists());
        assert!(!resume_path(&out.with_extension("gguf.tmp")).exists());
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn content_range_parsing() {
        assert_eq!(
            parse_content_range("bytes 100-999/1000"),
            Some((100, 999, Some(1000)))
        );
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(parse_content_range("items 1-2/3"), None);
    }

    /// Real mirrors over TLS (through $HTTPS_PROXY if set):
    /// `cargo test -- --ignored real_mirrors`
    #[test]
    #[ignore]
    fn real_mirrors() {
        let dir = std::env::temp_dir().join(format!("translate-https-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for url in [
            "https://modelscope.cn/models/unsloth/Qwen3.5-0.8B-GGUF/resolve/master/README.md",
            "https://hf-mirror.com/unsloth/Qwen3.5-0.8B-GGUF/resolve/main/README.md",
            "https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF/resolve/main/README.md",
        ] {
            let out = dir.join("README.md");
            let _ = std::fs::remove_file(&out);
            assert!(fetch(&out, url).unwrap(), "{url}");
            assert!(std::fs::metadata(&out).unwrap().len() > 0, "{url}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
