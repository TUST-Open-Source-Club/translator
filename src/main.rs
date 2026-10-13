mod download;
mod err;
mod infer;
mod logging;
mod native_host;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use download::{ModelSource, ModelVariant, VARIANT_DEFAULT, VARIANT_FAST, VARIANT_QUALITY};
use err::AppError;
use infer::{default_target, Lang, SourceLang, Translator};

const ENV_MODEL: &str = "QWEN_TRANSLATE_MODEL";

#[derive(Parser)]
#[command(
    name = "translate",
    version,
    disable_help_flag = true,
    about = "Local offline translation with Qwen3.5 (text and images)",
    long_about = "Local offline translation with Qwen3.5 (text and images).\n\n\
        Input: command-line arguments take precedence, otherwise stdin is read (pipe-friendly).\n\
        Image translation: pass an image via --image; -x/-y/-w/-h crop the region to translate.\n\
        Models: 2B by default; --fast/--quality (or --model fast/quality) switch to 0.8B/4B.\n\
        Model lookup order: --model / env var > user data dir > /usr/local/share, /usr/share\n\
        (on macOS also the Homebrew prefix /opt/homebrew/share); auto-downloads to the user\n\
        data dir when not found anywhere.\n\
        stdout carries only results; logs and errors go to stderr.\n\n\
        Exit codes: 0 ok, 1 runtime, 2 usage, 3 model, 4 download, 5 unsupported.\n\n\
        Examples:\n  \
        translate \"I love programming\"\n  \
        echo 我喜欢编程 | translate\n  \
        translate --to ja \"你好\"\n  \
        translate --model fast \"quick answer\"\n  \
        translate --image shot.png -x 10 -y 20 -w 300 -h 200"
)]
struct Args {
    /// Path to a GGUF model file, or a variant keyword: default (2B) / fast (0.8B) / quality (4B)
    #[arg(long, env = ENV_MODEL)]
    model: Option<String>,

    /// Path to the mmproj projector weights (required for --image with a custom --model)
    #[arg(long)]
    mmproj: Option<PathBuf>,

    /// Use the default 2B model (same as --model default)
    #[arg(long)]
    default: bool,

    /// Use the fast 0.8B model (same as --model fast)
    #[arg(long, conflicts_with = "quality", conflicts_with = "default")]
    fast: bool,

    /// Use the high-quality 4B model (same as --model quality)
    #[arg(long, conflicts_with = "fast", conflicts_with = "default")]
    quality: bool,

    /// Source used when auto-downloading the model
    #[arg(long, value_enum)]
    model_source: Option<ModelSource>,

    /// Download models from HuggingFace (shortcut for --model-source huggingface)
    #[arg(long)]
    huggingface: bool,

    /// Only download model variant(s), then exit. All variants when combined with
    /// none of --default/--fast/--quality; otherwise just the selected one.
    /// Other options and environment variables are ignored.
    #[arg(long)]
    download_only: bool,

    /// Source language (default: auto-detect)
    #[arg(long, value_enum, default_value = "auto", env = "TRANSLATE_FROM")]
    from: SourceLang,

    /// Target language (default: Chinese<->English auto pair)
    #[arg(long, value_enum, env = "TRANSLATE_TO")]
    to: Option<Lang>,

    /// Layers to offload to the GPU (falls back to CPU when unavailable)
    #[arg(long, default_value_t = 999)]
    gpu_layers: u32,

    /// llama.cpp log level
    #[arg(long, value_enum, default_value = "warn")]
    log_level: logging::LogLevel,

    /// Image translation: path to the image (direct multimodal translation, not OCR)
    #[arg(long)]
    image: Option<PathBuf>,

    /// Left edge X of the region to translate (requires -y/-w/-h)
    #[arg(short = 'x', requires = "y", requires = "w", requires = "h")]
    x: Option<u32>,

    /// Top edge Y of the region to translate
    #[arg(short = 'y', requires = "x", requires = "w", requires = "h")]
    y: Option<u32>,

    /// Width of the region to translate
    #[arg(short = 'w', long = "width", requires = "x", requires = "y", requires = "h")]
    w: Option<u32>,

    /// Height of the region to translate
    #[arg(short = 'h', long = "height", requires = "x", requires = "y", requires = "w")]
    h: Option<u32>,

    /// Disable streaming output
    #[arg(long)]
    no_stream: bool,

    /// Run as a Firefox native messaging host (used by the browser extension)
    #[arg(long)]
    native_host: bool,

    /// Print help
    #[arg(long, action = clap::ArgAction::HelpLong)]
    help: Option<bool>,

    /// Text to translate (read from stdin when omitted)
    text: Vec<String>,
}

fn data_dir_path(file: &str) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qwen-translate")
        .join(file)
}

/// System-wide lookup paths (model-bundled system packages install here)
fn system_paths(file: &str) -> Vec<PathBuf> {
    let dirs: Vec<&str> = if cfg!(target_os = "macos") {
        // Apple Silicon Homebrew / Intel Homebrew & manual installs / system packages
        vec![
            "/opt/homebrew/share/qwen-translate",
            "/usr/local/share/qwen-translate",
            "/usr/share/qwen-translate",
        ]
    } else {
        vec![
            "/usr/local/share/qwen-translate",
            "/usr/share/qwen-translate",
        ]
    };
    dirs.into_iter().map(|d| PathBuf::from(d).join(file)).collect()
}

/// Lookup order: user data dir > system dirs; returns the user data dir path
/// when not found anywhere (as the auto-download destination)
fn resolve_named_file(file: &str) -> PathBuf {
    let user = data_dir_path(file);
    if user.is_file() {
        return user;
    }
    for p in system_paths(file) {
        if p.is_file() {
            return p;
        }
    }
    user
}

/// A model given as a file path is explicit: it is never auto-downloaded,
/// and image mode requires an explicit --mmproj.
struct ModelChoice {
    path: PathBuf,
    explicit: bool,
    variant: &'static ModelVariant,
}

fn resolve_model_choice(args: &Args) -> Result<ModelChoice> {
    let flag_variant = if args.fast {
        Some(&VARIANT_FAST)
    } else if args.quality {
        Some(&VARIANT_QUALITY)
    } else if args.default {
        Some(&VARIANT_DEFAULT)
    } else {
        None
    };
    let kw_variant = match args.model.as_deref() {
        Some("fast") => Some(&VARIANT_FAST),
        Some("quality") => Some(&VARIANT_QUALITY),
        Some("default") => Some(&VARIANT_DEFAULT),
        _ => None,
    };
    if let (Some(f), Some(k)) = (flag_variant, kw_variant) {
        if !std::ptr::eq(f, k) {
            return Err(err::usage(format!(
                "conflicting model variants: --{} vs --model {}",
                if args.fast {
                    "fast"
                } else if args.quality {
                    "quality"
                } else {
                    "default"
                },
                args.model.as_deref().unwrap()
            )));
        }
    }
    let variant: &ModelVariant = flag_variant.or(kw_variant).unwrap_or(&VARIANT_DEFAULT);
    let (path, explicit) = match &args.model {
        Some(p) if kw_variant.is_none() => (PathBuf::from(p), true),
        _ => (resolve_named_file(variant.model_file), false),
    };
    Ok(ModelChoice {
        path,
        explicit,
        variant,
    })
}

/// Auto-download when the model is missing; error out for explicit paths
fn ensure_model(choice: &ModelChoice, source: ModelSource) -> Result<()> {
    if choice.path.is_file() {
        return Ok(());
    }
    if choice.explicit {
        return Err(err::model(format!(
            "model file not found: {}\n(paths set via --model / {ENV_MODEL} are not auto-downloaded; please check the path)",
            choice.path.display()
        )));
    }
    download::download_model(choice.variant, &choice.path, source)
        .map_err(|e| err::or_kind(e, AppError::Download))
}

fn no_input() -> ! {
    Args::command()
        .error(
            clap::error::ErrorKind::InvalidValue,
            "no input text (pass it as arguments or via stdin)",
        )
        .exit()
}

fn print_result(out: &str, streamed: bool) {
    if !streamed {
        print!("{out}");
    }
    if !out.ends_with('\n') {
        println!();
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    logging::init(args.log_level);

    // --model-source wins over --huggingface; default is ModelScope
    let model_source = args.model_source.unwrap_or(if args.huggingface {
        ModelSource::HuggingFace
    } else {
        ModelSource::ModelScope
    });

    if args.download_only {
        // Pure download mode: only the variant flags and download-source flags apply;
        // everything else (incl. --model/--mmproj and env vars) is ignored.
        let variants: Vec<&ModelVariant> = if args.fast {
            vec![&VARIANT_FAST]
        } else if args.quality {
            vec![&VARIANT_QUALITY]
        } else if args.default {
            vec![&VARIANT_DEFAULT]
        } else {
            vec![&VARIANT_DEFAULT, &VARIANT_FAST, &VARIANT_QUALITY]
        };
        for v in variants {
            let m = resolve_named_file(v.model_file);
            if !m.is_file() {
                download::download_model(v, &m, model_source)
                    .map_err(|e| err::or_kind(e, AppError::Download))?;
            }
            let mp = resolve_named_file(v.mmproj_file);
            if !mp.is_file() {
                download::download_mmproj(v, &mp, model_source)
                    .map_err(|e| err::or_kind(e, AppError::Download))?;
            }
            println!("{}", m.display());
            println!("{}", mp.display());
        }
        return Ok(());
    }

    let choice = resolve_model_choice(&args)?;
    let source = args.from.resolve();

    if args.native_host {
        return native_host::run(|| {
            ensure_model(&choice, model_source)?;
            Translator::load(&choice.path, args.gpu_layers)
                .map_err(|e| err::or_kind(e, AppError::Model))
        });
    }

    if let Some(image_path) = &args.image {
        if !args.text.is_empty() {
            return Err(err::usage(
                "image mode does not accept text input (the text comes from the image itself)",
            ));
        }
        ensure_model(&choice, model_source)?;
        let mmproj = match &args.mmproj {
            Some(p) => p.clone(),
            None => {
                if choice.explicit {
                    return Err(err::usage(
                        "--image with a custom --model also requires --mmproj \
                         (the projector weights must match the model)",
                    ));
                }
                resolve_named_file(choice.variant.mmproj_file)
            }
        };
        if !mmproj.is_file() {
            download::download_mmproj(choice.variant, &mmproj, model_source)
                .map_err(|e| err::or_kind(e, AppError::Download))?;
        }
        let mut translator = Translator::load_vision(&choice.path, &mmproj, args.gpu_layers)
            .map_err(|e| err::or_kind(e, AppError::Model))?;
        let region = match (args.x, args.y, args.w, args.h) {
            (Some(x), Some(y), Some(w), Some(h)) => Some((x, y, w, h)),
            _ => None,
        };
        let out = translator.translate_image(image_path, region, source, args.to, !args.no_stream)?;
        print_result(&out, !args.no_stream);
        return Ok(());
    }

    if args.x.is_some() || args.y.is_some() || args.w.is_some() || args.h.is_some() {
        return Err(err::usage("-x/-y/-w/-h can only be used together with --image"));
    }

    let text = if !args.text.is_empty() {
        args.text.join(" ")
    } else if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        no_input();
    };

    let text = text.trim();
    if text.is_empty() {
        no_input();
    }

    ensure_model(&choice, model_source)?;
    let translator = Translator::load(&choice.path, args.gpu_layers)
        .map_err(|e| err::or_kind(e, AppError::Model))?;
    let target = args
        .to
        .unwrap_or_else(|| default_target(source.unwrap_or_else(|| Lang::detect(text))));
    let out = translator.translate(text, source, target, !args.no_stream)?;
    print_result(&out, !args.no_stream);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(err::exit_code(&e))
        }
    }
}
