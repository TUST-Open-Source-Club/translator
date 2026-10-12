use std::io::{self, Read, Write};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::infer::{default_target, Lang, Translator};

const MAX_MSG_LEN: u32 = 8 * 1024 * 1024;

#[derive(Deserialize)]
struct Request {
    text: String,
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

/// Answer `first` and every following request with the loaded model
fn serve(
    translator: &Translator,
    first: &[u8],
    input: &mut impl Read,
    output: &mut impl Write,
) -> Result<()> {
    write_message(output, &handle(translator, first))?;
    while let Some(msg) = read_message(input)? {
        let response = match msg {
            Ok(body) => handle(translator, &body),
            Err(e) => error(e),
        };
        write_message(output, &response)?;
    }
    Ok(())
}

fn error(message: impl Into<String>) -> serde_json::Value {
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

fn write_message(output: &mut impl Write, msg: &serde_json::Value) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    output.write_all(&(payload.len() as u32).to_ne_bytes())?;
    output.write_all(&payload)?;
    output.flush()?;
    Ok(())
}

fn handle(translator: &Translator, buf: &[u8]) -> serde_json::Value {
    let req = match serde_json::from_slice::<Request>(buf) {
        Ok(r) => r,
        Err(e) => return error(format!("invalid request: {e}")),
    };
    let text = req.text.trim();
    if text.is_empty() {
        return error("empty text");
    }
    let detected = Lang::detect(text);
    match translator.translate(text, None, default_target(detected), false) {
        Ok(t) => json!({ "translation": t }),
        Err(e) => error(format!("{e:#}")),
    }
}
