use std::path::Path;
use std::process::Command;

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
    // A partial download is kept here and resumed on the next attempt / run
    let tmp = dest.with_extension("gguf.tmp");

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

/// Run the system curl (progress bar on stderr, stdout stays clean). Transient
/// network errors are retried, and an existing partial file is resumed.
fn curl(out: &Path, url: &str, resume: bool) -> Result<std::process::ExitStatus> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fL", "--connect-timeout", "15"])
        .args(["--retry", "3", "--retry-delay", "2"]);
    if resume {
        cmd.args(["-C", "-"]);
    }
    cmd.arg("-o")
        .arg(out)
        .arg(url)
        .status()
        .context("failed to execute curl (please install curl first)")
}
