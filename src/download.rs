use std::fs::{File, TryLockError};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};

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
                vec![format!("https://modelscope.cn/models/{repo}/resolve/master/{file}")]
            }
            // China-friendly mirror first, official site as fallback
            Self::HuggingFace => vec![
                format!("https://hf-mirror.com/{repo}/resolve/main/{file}"),
                format!("https://huggingface.co/{repo}/resolve/main/{file}"),
            ],
        }
    }
}

/// Download the model via the system curl.
pub fn download_model(v: &ModelVariant, dest: &Path, source: ModelSource) -> Result<()> {
    download_file(dest, source, v.repo, v.model_file)
}

/// Download the variant's mmproj projector weights (for image translation)
pub fn download_mmproj(v: &ModelVariant, dest: &Path, source: ModelSource) -> Result<()> {
    download_file(dest, source, v.repo, MMPROJ_REMOTE_FILE)
}

fn download_file(dest: &Path, source: ModelSource, repo: &str, file: &str) -> Result<()> {
    let dir = dest.parent().expect("model path has no parent directory");
    std::fs::create_dir_all(dir)?;

    // The CLI and the browser's native host may fetch the same file at once; two
    // curls resuming into one partial file would corrupt it. Take turns (the lock
    // is released when the process exits, so it can't go stale).
    let lock = File::create(dest.with_extension("gguf.lock"))?;
    if let Err(TryLockError::WouldBlock) = lock.try_lock() {
        eprintln!(
            "Waiting for another download of {} to finish…",
            dest.display()
        );
    }
    lock.lock().context("failed to lock the download")?;
    if dest.is_file() {
        // finished by the other process while we waited
        return Ok(());
    }

    // A partial download is kept here and resumed on the next attempt / run
    let tmp = dest.with_extension("gguf.tmp");
    discard_stale(&tmp);

    let mut tried = Vec::new();
    for url in source.urls(repo, file) {
        eprintln!("Downloading: {url}");
        let mut status = curl(&tmp, &url, true)?;
        // curl exit 33: server ignores Range; start over from scratch
        if status.code() == Some(33) {
            let _ = std::fs::remove_file(&tmp);
            status = curl(&tmp, &url, false)?;
        }
        if status.success() {
            std::fs::rename(&tmp, dest)?;
            eprintln!("Downloaded: {}", dest.display());
            return Ok(());
        }
        tried.push(url);
    }
    bail!(
        "download failed (tried: {}); rerun to resume the partial download",
        tried.join(", ")
    )
}

/// A partial file left for more than a day may stem from an older upload of the
/// model; resuming it would splice two versions together, so start over instead
fn discard_stale(tmp: &Path) {
    const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
    let stale = std::fs::metadata(tmp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > MAX_AGE);
    if stale {
        let _ = std::fs::remove_file(tmp);
    }
}

/// Run the system curl (progress bar on stderr, stdout stays clean). Transient
/// network errors are retried, and an existing partial file is resumed.
fn curl(out: &Path, url: &str, resume: bool) -> Result<std::process::ExitStatus> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fL", "--connect-timeout", "15"])
        .args(["--retry", "3", "--retry-delay", "2"]);
    if resume {
        cmd.args(["-C", "-"]);
    }
    // stdout may be the native messaging pipe: nothing else must write to it
    cmd.stdout(Stdio::null());
    #[cfg(windows)]
    {
        use std::io::IsTerminal;
        use std::os::windows::process::CommandExt;
        // Without a console (started by a browser) Windows would pop up a new
        // console window for curl.exe
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if !std::io::stderr().is_terminal() {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
    }
    cmd.arg("-o")
        .arg(out)
        .arg(url)
        .status()
        .context("failed to execute curl (please install curl first)")
}
