use std::ffi::CStr;
use std::io::Write;
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

// ggml_log_level: NONE=0 DEBUG=1 INFO=2 WARN=3 ERROR=4 CONT=5（续行）
const WARN: i32 = 3;
const CONT: i32 = 5;

#[derive(Clone, Copy, Default, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    #[default]
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    fn threshold(self) -> i32 {
        match self {
            LogLevel::Off => i32::MAX,
            LogLevel::Error => 4,
            LogLevel::Warn => 3,
            LogLevel::Info => 2,
            LogLevel::Debug => 1,
        }
    }
}

static THRESHOLD: AtomicI32 = AtomicI32::new(3);
static LAST_PASSED: AtomicBool = AtomicBool::new(false);

/// 本模型（混合循环记忆 + M-RoPE 视觉）每次图片翻译都会触发的无害 WARN；
/// 仅在 info/debug 级别显示，默认 warn 下隐藏，保持 stderr 安静
const BENIGN_WARNINGS: &[&[u8]] = &[
    b"non-consecutive token position",
    b"require at minimum 1024 image tokens",
    b"--image-min-tokens",
    b"llama.cpp/issues/16842",
];

fn is_benign(bytes: &[u8]) -> bool {
    BENIGN_WARNINGS
        .iter()
        .any(|pat| bytes.windows(pat.len()).any(|w| w == *pat))
}

unsafe extern "C" fn log_callback(
    level: llama_cpp_sys_2::ggml_log_level,
    text: *const c_char,
    _user_data: *mut c_void,
) {
    if text.is_null() {
        return;
    }
    let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
    let level = level as i32;
    let pass = if level == CONT {
        LAST_PASSED.load(Ordering::Relaxed)
    } else {
        let threshold = THRESHOLD.load(Ordering::Relaxed);
        let p = level >= threshold && !(level == WARN && threshold >= WARN && is_benign(bytes));
        LAST_PASSED.store(p, Ordering::Relaxed);
        p
    };
    if pass {
        let _ = std::io::stderr().write_all(bytes);
    }
}

/// 接管 llama.cpp / ggml / mtmd（clip）的日志输出，按级别过滤（默认 warn，静默 info 噪音）
pub fn init(level: LogLevel) {
    THRESHOLD.store(level.threshold(), Ordering::Relaxed);
    unsafe {
        llama_cpp_sys_2::llama_log_set(Some(log_callback), std::ptr::null_mut());
        llama_cpp_sys_2::ggml_log_set(Some(log_callback), std::ptr::null_mut());
        // mtmd/clip 有独立的日志器，不接管的话图片翻译会把 info 日志全打到 stderr
        #[cfg(feature = "mtmd")]
        {
            llama_cpp_sys_2::mtmd_log_set(Some(log_callback), std::ptr::null_mut());
            llama_cpp_sys_2::mtmd_helper_log_set(Some(log_callback), std::ptr::null_mut());
        }
    }
}
