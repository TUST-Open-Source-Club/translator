mod download;
mod err;
mod infer;
mod input;
mod logging;
mod native_host;

use std::io::{BufRead, IsTerminal, Read, Write};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{parser::ValueSource, CommandFactory, FromArgMatches, Parser};
use download::{ModelSource, ModelVariant, VARIANT_DEFAULT, VARIANT_FAST, VARIANT_QUALITY};
use err::AppError;
use infer::{resolve_direction, Lang, SourceLang, Translator};

const ENV_MODEL: &str = "QWEN_TRANSLATE_MODEL";
/// Add-on ID of the Firefox extension (firefox/manifest.json)
const FIREFOX_ADDON_ID: &str = "qwen-translate@local";

fn parse_threads(value: &str) -> std::result::Result<NonZeroU32, String> {
    let threads = value
        .parse::<u32>()
        .map_err(|_| "threads must be a positive 32-bit integer".to_string())?;
    if threads == 0 || threads > i32::MAX as u32 {
        return Err(format!("threads must be between 1 and {}", i32::MAX));
    }
    // The inference bindings use i32 for the llama.cpp thread count.
    Ok(NonZeroU32::new(threads).expect("nonzero checked above"))
}

#[derive(Parser)]
#[command(
    name = "translate",
    // Not argv[0]: the Android app runs the binary as libtranslate.so
    bin_name = "translate",
    version,
    disable_help_flag = true,
    about = "Local offline translation with Qwen3.5 (text and images)",
    long_about = "Local offline translation with Qwen3.5 (text and images).\n\n\
        Input: command-line arguments take precedence, otherwise stdin is read (pipe-friendly);\n\
        run in a terminal without either, it translates each line typed (interactive mode).\n\
        Image translation: pass an image via --image; -x/-y/-w/-h crop the region to translate.\n\
        Models: 2B by default; --fast/--quality (or --model fast/quality) switch to 0.8B/4B.\n\
        Model lookup order: --model / env var > user data dir > model/ next to the executable\n\
        > /usr/local/share, /usr/share (macOS also /opt/homebrew/share; Windows %ProgramData%;\n\
        Android/Termux $PREFIX/share);\n\
        auto-downloads to the user data dir when not found anywhere.\n\
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

    /// Compatibility entry point for scripts/download-model.sh.
    #[arg(
        long,
        hide = true,
        conflicts_with_all = ["download_only", "native_host", "image", "text", "x", "y", "w", "h"]
    )]
    download_from_env: bool,

    /// Source language (default: auto-detect)
    #[arg(long, value_enum, default_value = "auto", env = "TRANSLATE_FROM")]
    from: SourceLang,

    /// Target language (default: Chinese<->English auto pair)
    #[arg(long, value_enum, env = "TRANSLATE_TO")]
    to: Option<Lang>,

    /// Layers to offload to the GPU (falls back to CPU when unavailable)
    #[arg(long, default_value_t = 999)]
    gpu_layers: u32,

    /// CPU threads for inference (default: one per core; half of them on Android)
    #[arg(long, env = "TRANSLATE_THREADS", value_parser = parse_threads)]
    threads: Option<NonZeroU32>,

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
    #[arg(
        short = 'w',
        long = "width",
        requires = "x",
        requires = "y",
        requires = "h"
    )]
    w: Option<u32>,

    /// Height of the region to translate
    #[arg(
        short = 'h',
        long = "height",
        requires = "x",
        requires = "y",
        requires = "w"
    )]
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

    /// Text to translate (read from stdin when omitted; interactive in a terminal)
    text: Vec<String>,
}

impl Args {
    fn command_for_args(raw: &[String]) -> clap::Command {
        let command = Self::command();
        if raw
            .iter()
            .take_while(|arg| arg.as_str() != "--")
            .any(|arg| arg == "--download-only" || arg == "--download-from-env")
        {
            // The old standalone download script never parsed translation
            // settings. A stale/invalid translation default must not prevent
            // model maintenance; explicit CLI values still get validated.
            ["model", "from", "to", "threads"]
                .into_iter()
                .fold(command, |cmd, id| {
                    cmd.mut_arg(id, |arg| arg.env(clap::builder::Resettable::Reset))
                })
        } else {
            command
        }
    }

    fn from_cli_matches(matches: &clap::ArgMatches) -> std::result::Result<Self, clap::Error> {
        let mut args = Self::from_arg_matches(matches)?;
        // Variant flags are aliases for --model <variant>, so they override an
        // environment default too. Keep explicit --model values for the usual
        // conflict check (or an explicitly supplied custom model path).
        if (args.default || args.fast || args.quality)
            && matches.value_source("model") != Some(ValueSource::CommandLine)
        {
            args.model = None;
        }
        Ok(args)
    }
}

fn data_dir_path(file: &str) -> PathBuf {
    // Local (non-roaming) app data: %LOCALAPPDATA% on Windows, so gigabytes of model
    // files don't travel with roaming profiles; the same as data_dir() elsewhere
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qwen-translate")
        .join(file)
}

/// Portable and system-wide lookup paths (model-bundled packages install here)
fn system_paths(file: &str) -> Vec<PathBuf> {
    // Portable layout: model/ next to the executable (Windows zip, extracted tarballs)
    let mut dirs: Vec<PathBuf> = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("model")))
        .into_iter()
        .collect();
    if cfg!(windows) {
        dirs.extend(
            std::env::var_os("ProgramData").map(|d| PathBuf::from(d).join("qwen-translate")),
        );
    } else if cfg!(target_os = "android") {
        // Termux keeps its "/usr" under the app's private data dir
        let prefix = std::env::var_os("PREFIX").map_or_else(
            || PathBuf::from("/data/data/com.termux/files/usr"),
            PathBuf::from,
        );
        dirs.push(prefix.join("share/qwen-translate"));
    } else if cfg!(target_os = "macos") {
        // Apple Silicon Homebrew / Intel Homebrew & manual installs / system packages
        dirs.extend(
            [
                "/opt/homebrew/share/qwen-translate",
                "/usr/local/share/qwen-translate",
                "/usr/share/qwen-translate",
            ]
            .map(PathBuf::from),
        );
    } else {
        dirs.extend(
            [
                "/usr/local/share/qwen-translate",
                "/usr/share/qwen-translate",
            ]
            .map(PathBuf::from),
        );
    }
    dirs.into_iter().map(|d| d.join(file)).collect()
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
        if f.model_file != k.model_file {
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

/// Explicit projectors must already exist: a custom model path tells us nothing
/// about the compatible projector, even when a built-in variant is also selected.
fn resolve_mmproj(args: &Args, choice: &ModelChoice) -> Result<PathBuf> {
    if let Some(path) = &args.mmproj {
        if !path.is_file() {
            return Err(err::model(format!(
                "mmproj file not found: {}\n(paths set via --mmproj are not auto-downloaded; please check the path)",
                path.display()
            )));
        }
        return Ok(path.clone());
    }
    if choice.explicit {
        return Err(err::usage(
            "--image with a custom --model also requires --mmproj \
             (the projector weights must match the model)",
        ));
    }
    Ok(resolve_named_file(choice.variant.mmproj_file))
}

fn no_input() -> ! {
    Args::command()
        .error(
            clap::error::ErrorKind::InvalidValue,
            "no input text (pass it as arguments or via stdin)",
        )
        .exit()
}

/// Write the result to stdout (only the final newline if it was streamed already).
/// A reader that went away early (e.g. `| head -c0`) is normal for a filter: stop
/// quietly instead of panicking like print! does.
fn print_result(out: &str, streamed: bool) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    let body: &[u8] = if streamed { b"" } else { out.as_bytes() };
    let newline: &[u8] = if out.ends_with('\n') { b"" } else { b"\n" };
    let res = stdout
        .write_all(body)
        .and_then(|()| stdout.write_all(newline))
        .and_then(|()| stdout.flush());
    match res {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        res => Ok(res?),
    }
}

/// Whether a browser started us as its native messaging host. Host manifests can
/// only name an executable, and on Windows there is no wrapper script to add
/// --native-host, so recognise the arguments browsers pass instead: Chromium
/// browsers pass the caller's origin (plus --parent-window=N on Windows), Firefox
/// passes the host manifest path and the add-on ID.
fn is_browser_launch(args: &[String]) -> bool {
    let args: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--native-host")
        .collect();
    match args.as_slice() {
        [origin, rest @ ..] if origin.starts_with("chrome-extension://") => {
            rest.iter().all(|a| a.starts_with("--parent-window="))
        }
        [manifest, id, ..] => manifest.ends_with(".json") && *id == FIREFOX_ADDON_ID,
        _ => false,
    }
}

fn run() -> Result<()> {
    let raw: Vec<String> = std::env::args_os()
        .skip(1)
        .filter_map(|a| a.into_string().ok())
        .collect();
    let matches = if is_browser_launch(&raw) {
        // Browser-supplied arguments are not ours; options come from env vars
        Args::command().get_matches_from(["translate", "--native-host"])
    } else {
        Args::command_for_args(&raw).get_matches()
    };
    let args = Args::from_cli_matches(&matches).unwrap_or_else(|e| e.exit());
    logging::init(args.log_level);

    // --model-source wins over --huggingface; default is ModelScope
    let model_source = args.model_source.unwrap_or(if args.huggingface {
        ModelSource::HuggingFace
    } else {
        ModelSource::ModelScope
    });

    if args.download_from_env {
        return download::download_from_env(model_source);
    }

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
            // Informational only: a closed stdout must not abort the downloads
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{}\n{}", m.display(), mp.display());
        }
        return Ok(());
    }

    let choice = resolve_model_choice(&args)?;
    let source = args.from.resolve();

    if args.native_host {
        return native_host::run(|| {
            ensure_model(&choice, model_source)?;
            Translator::load(&choice.path, args.gpu_layers, args.threads)
                .map_err(|e| err::or_kind(e, AppError::Model))
        });
    }

    if let Some(image_path) = &args.image {
        if !args.text.is_empty() {
            return Err(err::usage(
                "image mode does not accept text input (the text comes from the image itself)",
            ));
        }
        let mmproj = resolve_mmproj(&args, &choice)?;
        ensure_model(&choice, model_source)?;
        if args.mmproj.is_none() && !mmproj.is_file() {
            download::download_mmproj(choice.variant, &mmproj, model_source)
                .map_err(|e| err::or_kind(e, AppError::Download))?;
        }
        let mut translator =
            Translator::load_vision(&choice.path, &mmproj, args.gpu_layers, args.threads)
                .map_err(|e| err::or_kind(e, AppError::Model))?;
        let region = match (args.x, args.y, args.w, args.h) {
            (Some(x), Some(y), Some(w), Some(h)) => Some((x, y, w, h)),
            _ => None,
        };
        let out =
            translator.translate_image(image_path, region, source, args.to, !args.no_stream)?;
        return print_result(&out, !args.no_stream);
    }

    if args.x.is_some() || args.y.is_some() || args.w.is_some() || args.h.is_some() {
        return Err(err::usage(
            "-x/-y/-w/-h can only be used together with --image",
        ));
    }

    let text = if !args.text.is_empty() {
        args.text.join(" ")
    } else if !std::io::stdin().is_terminal() {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf)?;
        input::decode(buf)
    } else {
        return interactive(&args, &choice, model_source, source);
    };

    let text = text.trim();
    if text.is_empty() {
        no_input();
    }

    ensure_model(&choice, model_source)?;
    let translator = Translator::load(&choice.path, args.gpu_layers, args.threads)
        .map_err(|e| err::or_kind(e, AppError::Model))?;
    let (source, target) = resolve_direction(text, source, args.to);
    let out = translator.translate(text, source, target, !args.no_stream)?;
    print_result(&out, !args.no_stream)
}

/// `translate` alone in a terminal: translate each line typed until end of input,
/// with the model loaded once (a REPL, like the Android app's terminal starts).
/// Prompts and hints go to stderr, so stdout still carries translations only.
fn interactive(
    args: &Args,
    choice: &ModelChoice,
    model_source: ModelSource,
    source: Option<Lang>,
) -> Result<()> {
    ensure_model(choice, model_source)?;
    eprintln!("Loading {}…", choice.path.display());
    let translator = Translator::load(&choice.path, args.gpu_layers, args.threads)
        .map_err(|e| err::or_kind(e, AppError::Model))?;
    let mut session = translator.text_session()?;
    let eof = if cfg!(windows) {
        "Ctrl+Z, Enter"
    } else {
        "Ctrl+D"
    };
    eprintln!("Type text and press Enter to translate it; {eof} to quit.");
    let mut stdin = std::io::stdin().lock();
    let mut line = Vec::new();
    loop {
        eprint!("> ");
        line.clear();
        if stdin.read_until(b'\n', &mut line)? == 0 {
            eprintln!();
            return Ok(());
        }
        let text = input::decode(std::mem::take(&mut line));
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let (from, to) = resolve_direction(text, source, args.to);
        match session.translate(text, from, to, !args.no_stream) {
            Ok(out) => print_result(&out, !args.no_stream)?,
            Err(e) if is_broken_pipe(&e) => return Err(e),
            // One bad line shouldn't end the session
            Err(e) => eprintln!("error: {e:#}"),
        }
    }
}

/// The reader of streamed output went away (e.g. `| head`): normal for a filter
fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe)
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if is_broken_pipe(&e) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(err::exit_code(&e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_count_stays_representable_for_llama() {
        assert_eq!(parse_threads("1").unwrap().get(), 1);
        assert_eq!(
            parse_threads(i32::MAX.to_string().as_str()).unwrap().get(),
            i32::MAX as u32
        );
        assert!(parse_threads("0").is_err());
        assert!(parse_threads((i32::MAX as u64 + 1).to_string().as_str()).is_err());
        assert!(parse_threads("not-a-number").is_err());
    }

    fn args_with_model_default(args: &[&str], model: Option<&'static str>) -> Args {
        let mut cmd = Args::command();
        // Command defaults exercise the same lower-priority branch as env values
        // without mutating the process environment of concurrent tests.
        for id in ["model", "from", "to", "threads"] {
            cmd = cmd.mut_arg(id, |a| a.env(None::<&str>));
        }
        if let Some(model) = model {
            cmd = cmd.mut_arg("model", |a| a.default_value(model));
        }
        let matches = cmd
            .try_get_matches_from(std::iter::once("translate").chain(args.iter().copied()))
            .unwrap();
        Args::from_cli_matches(&matches).unwrap()
    }

    #[test]
    fn variant_flags_override_model_defaults() {
        for model in ["fast", "/missing/custom.gguf"] {
            for (flag, expected) in [
                ("--default", &VARIANT_DEFAULT),
                ("--fast", &VARIANT_FAST),
                ("--quality", &VARIANT_QUALITY),
            ] {
                let args = args_with_model_default(&[flag, "hello"], Some(model));
                let choice = resolve_model_choice(&args).unwrap();
                assert!(!choice.explicit);
                assert_eq!(choice.variant.model_file, expected.model_file);
            }
        }
    }

    #[test]
    fn explicit_model_choices_keep_their_precedence_and_conflicts() {
        let args = args_with_model_default(&["--model", "quality"], Some("fast"));
        let choice = resolve_model_choice(&args).unwrap();
        assert_eq!(choice.variant.model_file, VARIANT_QUALITY.model_file);

        let args = args_with_model_default(&["--quality", "--model", "fast"], None);
        assert!(resolve_model_choice(&args).is_err());
        let args = args_with_model_default(&["--fast", "--model", "fast"], None);
        assert!(resolve_model_choice(&args).is_ok());

        let args = args_with_model_default(&["--fast", "--model", "custom.gguf"], Some("quality"));
        let choice = resolve_model_choice(&args).unwrap();
        assert!(choice.explicit);
        assert_eq!(choice.path, PathBuf::from("custom.gguf"));

        let args = args_with_model_default(&[], Some("quality"));
        assert_eq!(
            resolve_model_choice(&args).unwrap().variant.model_file,
            VARIANT_QUALITY.model_file
        );
    }

    #[test]
    fn explicit_projector_must_exist_and_custom_models_require_one() {
        let mut args = args_with_model_default(&["--model", "custom.gguf"], None);
        let choice = resolve_model_choice(&args).unwrap();
        assert_eq!(
            err::exit_code(&resolve_mmproj(&args, &choice).unwrap_err()),
            2
        );

        let missing = std::env::temp_dir().join(format!(
            "translate-missing-projector-{}-{}.gguf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        args.mmproj = Some(missing.clone());
        assert_eq!(
            err::exit_code(&resolve_mmproj(&args, &choice).unwrap_err()),
            3
        );
        assert!(!missing.exists());

        // Resolution checks existence; the actual model loader checks its format.
        let existing = std::env::current_exe().unwrap();
        args.mmproj = Some(existing.clone());
        assert_eq!(resolve_mmproj(&args, &choice).unwrap(), existing);

        let mut args = args_with_model_default(&["--fast"], None);
        let choice = resolve_model_choice(&args).unwrap();
        assert!(resolve_mmproj(&args, &choice)
            .unwrap()
            .ends_with(VARIANT_FAST.mmproj_file));
        args.mmproj = Some(missing);
        assert_eq!(
            err::exit_code(&resolve_mmproj(&args, &choice).unwrap_err()),
            3
        );
    }

    fn launch(args: &[&str]) -> bool {
        is_browser_launch(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn browser_launches_are_recognised() {
        let chrome = "chrome-extension://bfkmmaobjaldoelkdphajjagomjehbpc/";
        assert!(launch(&[chrome]));
        assert!(launch(&[chrome, "--parent-window=0"]));
        assert!(launch(&["--native-host", chrome]));
        let manifest = r"C:\Users\me\AppData\Local\qwen-translate\native-host-firefox.json";
        assert!(launch(&[manifest, "qwen-translate@local"]));
    }

    #[test]
    fn normal_invocations_are_not_browser_launches() {
        assert!(!launch(&[]));
        assert!(!launch(&["hello", "world"]));
        assert!(!launch(&["--to", "ja", "chrome-extension://x/"]));
        assert!(!launch(&["chrome-extension://x/", "and more text"]));
        assert!(!launch(&["config.json", "qwen-translate@local!"]));
    }
}
