use std::fs::{File, OpenOptions, TryLockError};
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

mod https;
mod idle;

/// A model variant: text model + its matching mmproj projector weights
pub struct ModelVariant {
    pub repo: &'static str,
    pub model_file: &'static str,
    /// Local file name for the mmproj (renamed per variant to avoid collisions)
    pub mmproj_file: &'static str,
}

/// Default variant (quality first)
pub const VARIANT_DEFAULT: ModelVariant = ModelVariant {
    repo: "unsloth/Qwen3.5-2B-GGUF",
    model_file: "Qwen3.5-2B-Q4_K_M.gguf",
    mmproj_file: "Qwen3.5-2B-mmproj-F16.gguf",
};

/// Fast variant (--fast)
pub const VARIANT_FAST: ModelVariant = ModelVariant {
    repo: "unsloth/Qwen3.5-0.8B-GGUF",
    model_file: "Qwen3.5-0.8B-Q4_K_M.gguf",
    // keep the original local name for backward compatibility with existing installs
    mmproj_file: "mmproj-F16.gguf",
};

/// Quality variant (--quality)
pub const VARIANT_QUALITY: ModelVariant = ModelVariant {
    repo: "unsloth/Qwen3.5-4B-GGUF",
    model_file: "Qwen3.5-4B-Q4_K_M.gguf",
    mmproj_file: "Qwen3.5-4B-mmproj-F16.gguf",
};

/// Remote file name of the mmproj inside each unsloth repo
const MMPROJ_REMOTE_FILE: &str = "mmproj-F16.gguf";

#[derive(Clone, Copy, Default, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum ModelSource {
    #[default]
    ModelScope,
    #[value(alias = "hf")]
    HuggingFace,
}

impl ModelSource {
    fn urls(self, repo: &str, file: &str) -> Vec<String> {
        match self {
            // ModelScope direct link, fast in China
            Self::ModelScope => {
                vec![format!(
                    "https://modelscope.cn/models/{repo}/resolve/master/{file}"
                )]
            }
            // China-friendly mirror first, official site as fallback
            Self::HuggingFace => vec![
                format!("https://hf-mirror.com/{repo}/resolve/main/{file}"),
                format!("https://huggingface.co/{repo}/resolve/main/{file}"),
            ],
        }
    }
}

/// Download the model with the shared, locked HTTPS downloader.
pub fn download_model(v: &ModelVariant, dest: &Path, source: ModelSource) -> Result<()> {
    download_file(dest, source, v.repo, v.model_file)
}

/// Download the variant's mmproj projector weights (for image translation)
pub fn download_mmproj(v: &ModelVariant, dest: &Path, source: ModelSource) -> Result<()> {
    download_file(dest, source, v.repo, MMPROJ_REMOTE_FILE)
}

fn download_file(dest: &Path, source: ModelSource, repo: &str, file: &str) -> Result<()> {
    download_urls(dest, &source.urls(repo, file))
}

fn download_urls(dest: &Path, urls: &[String]) -> Result<()> {
    download_urls_with(dest, urls, https::fetch)
}

fn download_urls_with(
    dest: &Path,
    urls: &[String],
    fetch: impl Fn(&Path, &str) -> Result<bool>,
) -> Result<()> {
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;

    // The CLI and the browser's native host may fetch the same file at once; two
    // writers resuming into one partial file would corrupt it. Take turns (the lock
    // is released when the process exits, so it can't go stale).
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dest.with_extension("gguf.lock"))?;
    // Block only when try_lock() failed: locking a handle that already holds the
    // lock deadlocks on Windows (LockFileEx is not reentrant)
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            eprintln!(
                "Waiting for another download of {} to finish…",
                dest.display()
            );
            lock.lock().context("failed to lock the download")?;
        }
        Err(TryLockError::Error(e)) => return Err(e).context("failed to lock the download"),
    }
    if dest.is_file() {
        // Finished by the other process while we waited. Validate the marker
        // before trusting it: an interrupted pre-atomic download from an older
        // version may have left a corrupt destination behind.
        if is_gguf(dest)? {
            return Ok(());
        }
        eprintln!("Discarding invalid model file: {}", dest.display());
        std::fs::remove_file(dest)?;
    }

    // A partial download is kept here and resumed on the next attempt / run
    let tmp = dest.with_extension("gguf.tmp");
    let mut tried = Vec::new();
    for url in urls {
        eprintln!("Downloading: {url}");
        if fetch(&tmp, url)? {
            // A close-delimited 200 response can end cleanly after a truncated
            // but still GGUF-looking prefix. Model mirrors must advertise a
            // complete length before publication; the HTTP layer still supports
            // unknown-length bodies for its own generic callers.
            if https::known_total(&tmp).is_none() || !is_gguf(&tmp)? {
                https::discard_partial(&tmp)?;
                tried.push(url.as_str());
                eprintln!("error: server did not return a complete, supported GGUF file");
                continue;
            }
            std::fs::rename(&tmp, dest)?;
            // The partial is gone, so a stale sidecar is harmless even if cleanup fails.
            let _ = std::fs::remove_file(https::resume_path(&tmp));
            eprintln!("Downloaded: {}", dest.display());
            return Ok(());
        }
        tried.push(url.as_str());
    }
    bail!(
        "download failed (tried: {}); rerun to resume the partial download",
        tried.join(", ")
    )
}

fn is_gguf(path: &Path) -> Result<bool> {
    let mut header = [0; 8];
    let mut file = File::open(path)?;
    Ok(file.read_exact(&mut header).is_ok()
        && &header[..4] == b"GGUF"
        && matches!(u32::from_le_bytes(header[4..].try_into().unwrap()), 2 | 3))
}

/// Compatibility entry point for scripts/download-model.sh. Use the same lock,
/// partial file and validators as automatic downloads, including on Windows.
pub fn download_from_env(source: ModelSource) -> Result<()> {
    let variant = match std::env::var("VARIANT").as_deref().unwrap_or("default") {
        "default" => &VARIANT_DEFAULT,
        "fast" => &VARIANT_FAST,
        "quality" => &VARIANT_QUALITY,
        _ => bail!("VARIANT must be default, fast or quality"),
    };
    let repo = std::env::var("MODEL_REPO").unwrap_or_else(|_| variant.repo.into());
    let model = std::env::var("MODEL_NAME").unwrap_or_else(|_| variant.model_file.into());
    // These are remote identifiers and local filenames, never arbitrary paths.
    anyhow::ensure!(
        !repo.is_empty() && repo.split('/').all(valid_component),
        "invalid MODEL_REPO"
    );
    anyhow::ensure!(valid_component(&model), "MODEL_NAME must be a filename");
    download_file(&crate::data_dir_path(&model), source, &repo, &model)?;
    download_file(
        &crate::data_dir_path(variant.mmproj_file),
        source,
        &repo,
        MMPROJ_REMOTE_FILE,
    )
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_existing_destination_is_replaced() {
        let dir = std::env::temp_dir().join(format!(
            "translate-download-dest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("model.gguf");
        std::fs::write(&dest, b"not a gguf").unwrap();
        let payload = b"GGUF\x03\0\0\0";
        download_urls_with(&dest, &["fixture://model".into()], |out, _| {
            std::fs::write(out, payload).unwrap();
            std::fs::write(
                https::resume_path(out),
                format!(
                    r#"{{"url":"fixture://model","etag":"","total":{}}}"#,
                    payload.len()
                ),
            )
            .unwrap();
            Ok(true)
        })
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), payload);
        let _ = std::fs::remove_dir_all(dir);
    }
}
