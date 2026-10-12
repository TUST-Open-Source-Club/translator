use std::io::Write;
use std::num::NonZeroU32;
use std::path::Path;

use anyhow::{Context, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::mtmd::{
    mtmd_default_marker, MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText,
};
use llama_cpp_2::sampling::LlamaSampler;

const N_CTX: u32 = 4096;
/// Larger context for vision: image token count grows with resolution
const N_CTX_VISION: u32 = 16384;
const MAX_PROMPT_TOKENS: usize = 3700;

/// Supported languages
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum Lang {
    Zh,
    En,
    Ja,
    Ko,
    Fr,
    De,
    Ru,
    Es,
    Pt,
}

impl Lang {
    pub fn name(self) -> &'static str {
        match self {
            Lang::Zh => "Simplified Chinese",
            Lang::En => "English",
            Lang::Ja => "Japanese",
            Lang::Ko => "Korean",
            Lang::Fr => "French",
            Lang::De => "German",
            Lang::Ru => "Russian",
            Lang::Es => "Spanish",
            Lang::Pt => "Portuguese",
        }
    }

    /// Rough language detection of the text (for picking the default target)
    pub fn detect(text: &str) -> Lang {
        let (mut han, mut kana, mut hangul, mut cyrillic) = (0usize, 0usize, 0usize, 0usize);
        for c in text.chars() {
            match c as u32 {
                // Hiragana / Katakana / phonetic extensions / half-width Katakana
                0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => kana += 1,
                0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => han += 1,
                // Hangul syllables / Jamo / compatibility Jamo
                0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => hangul += 1,
                0x0400..=0x04FF => cyrillic += 1,
                _ => {}
            }
        }
        // Japanese mixes kanji with kana, so kana must win over han; Chinese text
        // may still borrow the odd kana (e.g. "の"), hence the ratio rather than any().
        if kana > 0 && kana * 10 >= han {
            Lang::Ja
        } else if hangul > 0 && hangul >= han {
            Lang::Ko
        } else if han > 0 {
            Lang::Zh
        } else if cyrillic > 0 {
            Lang::Ru
        } else {
            Lang::En
        }
    }
}

/// Source language (auto = detect)
#[derive(Clone, Copy, Default, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum SourceLang {
    #[default]
    Auto,
    Zh,
    En,
    Ja,
    Ko,
    Fr,
    De,
    Ru,
    Es,
    Pt,
}

impl SourceLang {
    pub fn resolve(self) -> Option<Lang> {
        match self {
            SourceLang::Auto => None,
            SourceLang::Zh => Some(Lang::Zh),
            SourceLang::En => Some(Lang::En),
            SourceLang::Ja => Some(Lang::Ja),
            SourceLang::Ko => Some(Lang::Ko),
            SourceLang::Fr => Some(Lang::Fr),
            SourceLang::De => Some(Lang::De),
            SourceLang::Ru => Some(Lang::Ru),
            SourceLang::Es => Some(Lang::Es),
            SourceLang::Pt => Some(Lang::Pt),
        }
    }
}

/// Default target direction (Chinese <-> English): Chinese source -> English, otherwise -> Chinese
pub fn default_target(source: Lang) -> Lang {
    match source {
        Lang::Zh => Lang::En,
        _ => Lang::Zh,
    }
}

pub struct Translator {
    backend: LlamaBackend,
    model: LlamaModel,
    mtmd: Option<MtmdContext>,
}

impl Translator {
    pub fn load(model_path: &Path, gpu_layers: u32) -> Result<Self> {
        let backend = LlamaBackend::init()?;
        let params = LlamaModelParams::default().with_n_gpu_layers(gpu_layers);
        let model = LlamaModel::load_from_file(&backend, model_path, &params)
            .with_context(|| format!("failed to load model: {}", model_path.display()))?;
        Ok(Self {
            backend,
            model,
            mtmd: None,
        })
    }

    /// Text + image translation (additionally loads the mmproj projector)
    pub fn load_vision(model_path: &Path, mmproj_path: &Path, gpu_layers: u32) -> Result<Self> {
        let mut t = Self::load(model_path, gpu_layers)?;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4);
        let params = MtmdContextParams {
            use_gpu: gpu_layers > 0,
            n_threads: threads,
            ..Default::default()
        };
        let mmproj_str = mmproj_path
            .to_str()
            .context("mmproj path contains invalid characters")?;
        let mtmd = MtmdContext::init_from_file(mmproj_str, &t.model, &params)
            .with_context(|| format!("failed to load mmproj: {}", mmproj_path.display()))?;
        if !mtmd.support_vision() {
            return Err(crate::err::unsupported(
                "this model/mmproj combination does not support image input",
            ));
        }
        t.mtmd = Some(mtmd);
        Ok(t)
    }

    fn ctx_params(&self, n_ctx: u32) -> LlamaContextParams {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4);
        LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(512)
            .with_n_threads(threads)
            .with_n_threads_batch(threads)
    }

    /// Greedy decode loop (deterministic output); `stream` prints pieces to stdout as generated
    fn generate(
        &self,
        ctx: &mut LlamaContext,
        start_pos: i32,
        max_tokens: usize,
        stream: bool,
    ) -> Result<String> {
        let vocab = self.model.vocab();
        let mut sampler = LlamaSampler::chain_simple([LlamaSampler::greedy()]);
        let mut batch = LlamaBatch::new(512, 1);
        let mut out_bytes: Vec<u8> = Vec::new();
        let mut pos = start_pos;
        let stdout = std::io::stdout();
        for _ in 0..max_tokens {
            let token = sampler.sample(ctx, -1);
            sampler.accept(token);
            if vocab.is_eog(token) {
                break;
            }
            let piece = vocab.token_to_piece(token, false, None);
            if stream {
                let mut out = stdout.lock();
                let _ = out.write_all(&piece);
                let _ = out.flush();
            }
            out_bytes.extend_from_slice(&piece);
            batch.clear();
            batch.add(token, pos, &[0], true)?;
            pos += 1;
            ctx.decode(&mut batch).context("decode failed")?;
        }
        let raw = String::from_utf8_lossy(&out_bytes);
        Ok(strip_think(&raw).to_string())
    }

    pub fn translate(
        &self,
        text: &str,
        source: Option<Lang>,
        target: Lang,
        stream: bool,
    ) -> Result<String> {
        let from = match source {
            Some(s) => format!("from {} ", s.name()),
            None => String::new(),
        };
        let prompt = format!(
            "<|im_start|>system\n\
             You are a professional translator. Translate the user's text {from}into {}. \
             Preserve the original meaning, tone and formatting (including line breaks). \
             Output only the translation, without any explanation.<|im_end|>\n\
             <|im_start|>user\n{}<|im_end|>\n\
             <|im_start|>assistant\n<think>\n\n</think>\n\n",
            target.name(),
            text
        );

        let vocab = self.model.vocab();
        let tokens = vocab.tokenize(prompt.as_bytes(), true, true);
        if tokens.len() > MAX_PROMPT_TOKENS {
            return Err(crate::err::usage(format!(
                "text too long ({} tokens, limit {MAX_PROMPT_TOKENS})",
                tokens.len()
            )));
        }

        let mut ctx = self.model.new_context(&self.backend, self.ctx_params(N_CTX))?;
        // Chunked prefill: a single decode batch may not exceed n_batch,
        // and positions must be absolute across chunks.
        let n_batch = 512usize;
        let mut batch = LlamaBatch::new(n_batch, 1);
        let mut pos = 0usize;
        while pos < tokens.len() {
            let end = (pos + n_batch).min(tokens.len());
            batch.clear();
            for (i, t) in tokens[pos..end].iter().enumerate() {
                let p = pos + i;
                batch.add(*t, p as i32, &[0], p == tokens.len() - 1)?;
            }
            ctx.decode(&mut batch).context("failed to process prompt")?;
            pos = end;
        }

        let max_out = (tokens.len() * 2 + 128).min(2048);
        self.generate(&mut ctx, tokens.len() as i32, max_out, stream)
    }

    /// Image translation (direct multimodal translation, NOT OCR). region = (x, y, w, h), None = whole image.
    pub fn translate_image(
        &mut self,
        image_path: &Path,
        region: Option<(u32, u32, u32, u32)>,
        source: Option<Lang>,
        target: Option<Lang>,
        stream: bool,
    ) -> Result<String> {
        let img = image::open(image_path)
            .with_context(|| format!("failed to read image: {}", image_path.display()))?;
        let img = match region {
            Some((x, y, w, h)) => {
                if w == 0 || h == 0 || x + w > img.width() || y + h > img.height() {
                    return Err(crate::err::usage(format!(
                        "region out of bounds: image is {}x{}, requested x={x} y={y} w={w} h={h}",
                        img.width(),
                        img.height()
                    )));
                }
                img.crop_imm(x, y, w, h)
            }
            None => img,
        };
        let rgb = img.to_rgb8();
        let (w, h) = rgb.dimensions();
        let bitmap = MtmdBitmap::from_image_data(w, h, rgb.as_raw())
            .map_err(|e| anyhow::anyhow!("failed to create bitmap: {e}"))?;

        let from = match source {
            Some(s) => format!("from {} ", s.name()),
            None => String::new(),
        };
        let target = match target {
            Some(t) => t,
            // Auto direction: pass 1 detects whether the image text is Chinese,
            // pass 2 translates with an explicit target (small models cannot handle
            // conditional direction in a single prompt).
            None => {
                let probe = format!(
                    "<|im_start|>user\n{}\nIs the text in this image in Chinese? Answer only yes or no.<|im_end|>\n\
                     <|im_start|>assistant\n<think>\n\n</think>\n\n",
                    mtmd_default_marker()
                );
                let answer = self.run_vision_prompt(&bitmap, probe, 8, false)?;
                if answer.to_lowercase().contains("yes") {
                    Lang::En
                } else {
                    Lang::Zh
                }
            }
        };

        let prompt = format!(
            "<|im_start|>system\n\
             You are a professional translator. Output only the translation, without any explanation.<|im_end|>\n\
             <|im_start|>user\n{}\nRead the text in this image and translate it {from}into {}. \
             Output only the translation.<|im_end|>\n\
             <|im_start|>assistant\n<think>\n\n</think>\n\n",
            mtmd_default_marker(),
            target.name()
        );
        self.run_vision_prompt(&bitmap, prompt, 2048, stream)
    }

    /// Run one vision prompt: tokenize (text + image) -> encode -> greedy decode
    fn run_vision_prompt(
        &mut self,
        bitmap: &MtmdBitmap,
        prompt: String,
        max_tokens: usize,
        stream: bool,
    ) -> Result<String> {
        let input = MtmdInputText {
            text: prompt,
            add_special: true,
            parse_special: true,
        };
        // tokenize only needs &MtmdContext
        let chunks = {
            let mtmd = self
                .mtmd
                .as_ref()
                .context("mmproj not loaded; image translation unavailable")?;
            mtmd.tokenize(input, &[bitmap])
                .map_err(|e| anyhow::anyhow!("failed to tokenize image: {e}"))?
        };

        let mut ctx = self
            .model
            .new_context(&self.backend, self.ctx_params(N_CTX_VISION))?;
        // eval_chunks needs &mut MtmdContext; keep the borrow short
        let n_past = {
            let mtmd = self
                .mtmd
                .as_mut()
                .context("mmproj not loaded; image translation unavailable")?;
            chunks
                .eval_chunks(mtmd, &mut ctx, 0, 0, 512, true)
                .map_err(|e| anyhow::anyhow!("failed to encode image: {e}"))?
        };

        self.generate(&mut ctx, n_past, max_tokens, stream)
    }
}

/// Fallback: strip <think>...</think> blocks
fn strip_think(s: &str) -> &str {
    let s = s.trim();
    match s.rfind("</think>") {
        Some(end) => s[end + "</think>".len()..].trim(),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_languages() {
        assert!(Lang::detect("今天天气很好") == Lang::Zh);
        assert!(Lang::detect("I love programming") == Lang::En);
        assert!(Lang::detect("Привет, мир") == Lang::Ru);
        assert!(Lang::detect("안녕하세요") == Lang::Ko);
        assert!(Lang::detect("ありがとう") == Lang::Ja);
    }

    #[test]
    fn detect_japanese_with_kanji() {
        // Kanji + kana used to be detected as Chinese (and translated into English)
        assert!(Lang::detect("日本語を勉強しています") == Lang::Ja);
        assert!(Lang::detect("東京都に住んでいます。") == Lang::Ja);
    }

    #[test]
    fn detect_chinese_with_stray_kana() {
        assert!(Lang::detect("这是我们学校の图书馆，里面有很多很多的书") == Lang::Zh);
    }

    #[test]
    fn detect_korean_with_hanja() {
        assert!(Lang::detect("大韓民國의 수도는 서울입니다") == Lang::Ko);
    }
}
