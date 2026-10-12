use std::io::{self, Read, Write};

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::infer::{default_target, Lang, TextSession, Translator};

const MAX_MSG_LEN: u32 = 8 * 1024 * 1024;

/// Request body. Every field but `text` is optional, so older extensions keep working.
#[derive(Deserialize)]
struct Request {
    text: String,
    /// Target language code (zh/en/ja/…); default: Chinese <-> English pair
    #[serde(default)]
    to: Option<String>,
    /// Source language code; default: auto-detect
    #[serde(default)]
    from: Option<String>,
}

/// A request as read from the browser: the raw JSON body, or why it was rejected
type Incoming = std::result::Result<Vec<u8>, String>;

/// Native messaging protocol (Firefox / Chrome): each message is a 4-byte length in
/// native byte order followed by UTF-8 JSON. Loops until the browser closes stdin.
///
/// The model is loaded on the first request rather than at startup, so a failure
/// (missing model, failed download) reaches the extension as an error reply instead
/// of a bare "native host exited"; loading is retried on the next request.
pub fn run(load: impl Fn() -> Result<Translator>) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();

    while let Some(msg) = read_message(&mut input)? {
        let body = match msg {
            Ok(body) => body,
            Err(e) => {
                write_message(&mut output, &error(e))?;
                continue;
            }
        };
        match load() {
            Ok(translator) => return serve(&translator, &body, &mut input, &mut output),
            Err(e) => write_message(&mut output, &error(format!("{e:#}")))?,
        }
    }
    Ok(())
}

/// Answer `first` and every following request with the loaded model. One inference
/// context (KV cache) is allocated up front and reused by every request.
fn serve(
    translator: &Translator,
    first: &[u8],
    input: &mut impl Read,
    output: &mut impl Write,
) -> Result<()> {
    let mut session = match translator.text_session() {
        Ok(s) => s,
        Err(e) => {
            write_message(output, &error(format!("{e:#}")))?;
            return Err(e);
        }
    };
    write_message(output, &handle(&mut session, first))?;
    while let Some(msg) = read_message(input)? {
        let response = match msg {
            Ok(body) => handle(&mut session, &body),
            Err(e) => error(e),
        };
        write_message(output, &response)?;
    }
    Ok(())
}

fn error(message: impl Into<String>) -> Value {
    json!({ "error": message.into() })
}

/// Next message, or None once the browser closes the pipe
fn read_message(input: &mut impl Read) -> Result<Option<Incoming>> {
    let mut len_buf = [0u8; 4];
    match input.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e).context("failed to read message length"),
    }
    let len = u32::from_ne_bytes(len_buf);
    if len > MAX_MSG_LEN {
        // Skip the body to stay in sync with the stream, and keep serving
        io::copy(&mut input.take(u64::from(len)), &mut io::sink())
            .context("failed to skip oversized message")?;
        return Ok(Some(Err(format!(
            "message too long: {len} bytes (limit {MAX_MSG_LEN})"
        ))));
    }
    let mut buf = vec![0u8; len as usize];
    input
        .read_exact(&mut buf)
        .context("failed to read message body")?;
    Ok(Some(Ok(buf)))
}

fn write_message(output: &mut impl Write, msg: &Value) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    output.write_all(&(payload.len() as u32).to_ne_bytes())?;
    output.write_all(&payload)?;
    output.flush()?;
    Ok(())
}

fn handle(session: &mut TextSession, buf: &[u8]) -> Value {
    let value: Value = match serde_json::from_slice(buf) {
        Ok(v) => v,
        Err(e) => return error(format!("invalid request: {e}")),
    };
    let mut response = match translate(session, &value) {
        Ok(t) => json!({ "translation": t }),
        Err(e) => error(e),
    };
    // Echo the caller's request id so replies can be matched to requests
    if let Some(id) = value.get("id") {
        response["id"] = id.clone();
    }
    response
}

fn translate(session: &mut TextSession, value: &Value) -> std::result::Result<String, String> {
    let req = Request::deserialize(value).map_err(|e| format!("invalid request: {e}"))?;
    let text = req.text.trim();
    if text.is_empty() {
        return Err("empty text".into());
    }
    let source = parse_lang(req.from.as_deref())?;
    let target = match parse_lang(req.to.as_deref())? {
        Some(t) => t,
        None => default_target(source.unwrap_or_else(|| Lang::detect(text))),
    };
    session
        .translate(text, source, target, false)
        .map_err(|e| format!("{e:#}"))
}

/// Language code from a request; absent or "auto" means automatic
fn parse_lang(code: Option<&str>) -> std::result::Result<Option<Lang>, String> {
    match code {
        None | Some("" | "auto") => Ok(None),
        Some(c) => <Lang as ValueEnum>::from_str(c, true)
            .map(Some)
            .map_err(|_| format!("unsupported language: {c}")),
    }
}
