use std::io::Write;
use std::num::NonZeroU32;
use std::path::Path;

use anyhow::{bail, Context, Result};
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
use llama_cpp_2::token::LlamaToken;

const N_CTX: u32 = 8192;
/// Larger context for vision: image token count grows with resolution
const N_CTX_VISION: u32 = 16384;
/// Max tokens per decode batch
const N_BATCH: usize = 512;
/// Long input is translated in chunks of at most this many source tokens, so that
/// prompt + translation (up to ~2x the source) always fit in N_CTX
const CHUNK_TOKENS: usize = 2048;

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
        let (mut han, mut kana, mut hangul, mut cyrillic, mut latin) = (0usize, 0, 0, 0, 0);
        let mut zh_only = 0usize;
        for c in text.chars() {
            match c as u32 {
                // Hiragana / Katakana / phonetic extensions / half-width Katakana
                0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => kana += 1,
                0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => {
                    han += 1;
                    zh_only += usize::from(is_simplified_only(c));
                }
                // Hangul syllables / Jamo / compatibility Jamo
                0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => hangul += 1,
                0x0400..=0x04FF => cyrillic += 1,
                _ if c.is_alphabetic() => latin += 1,
                _ => {}
            }
        }
        // One CJK character carries about as much as a short word, so weigh CJK
        // characters against letters: an English text with a few Chinese characters
        // (e.g. a "[简体中文]" link) is English, not Chinese.
        let cjk = han + kana + hangul;
        if cjk == 0 || cjk * 3 < latin + cyrillic {
            return if cyrillic > latin { Lang::Ru } else { Lang::En };
        }
        // Japanese writes kanji together with plenty of kana and never uses simplified
        // Chinese characters; Chinese text may still quote a Japanese phrase.
        if kana > 0 && kana * 4 >= han && kana > zh_only * 2 {
            Lang::Ja
        } else if hangul > 0 && hangul >= han {
            Lang::Ko
        } else {
            Lang::Zh
        }
    }
}

/// Frequent simplified Chinese characters that Japanese never uses
fn is_simplified_only(c: char) -> bool {
    "这们说时个么还对让给过你她吗呢吧没从现发经长开关间问题语话见车东认识样习学术后".contains(c)
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

/// Source and target for a request. An explicit source wins; otherwise Japanese and
/// Korean, recognised by scripts unique to them (kana, hangul), are named in the
/// prompt as well: left open, the models tend to echo short kana/hangul text instead
/// of translating it. Latin-script text stays open (en/fr/de/es/pt look alike).
pub fn resolve_direction(
    text: &str,
    source: Option<Lang>,
    target: Option<Lang>,
) -> (Option<Lang>, Lang) {
    let detected = Lang::detect(text);
    let source = source.or_else(|| matches!(detected, Lang::Ja | Lang::Ko).then_some(detected));
    let target = target.unwrap_or_else(|| default_target(source.unwrap_or(detected)));
    (source, target)
}

pub struct Translator {
    backend: LlamaBackend,
    model: LlamaModel,
    mtmd: Option<MtmdContext>,
    threads: i32,
}

/// Default inference threads: one per available core. Phones use big.LITTLE SoCs
/// where threads on the little cores hold the big ones back, so Android uses half
/// of the cores (typically the big cluster).
fn default_threads() -> i32 {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get() as i32);
    if cfg!(target_os = "android") {
        (cores / 2).max(1)
    } else {
        cores
    }
}

impl Translator {
    /// `threads`: CPU threads for inference (None = [`default_threads`])
    pub fn load(model_path: &Path, gpu_layers: u32, threads: Option<NonZeroU32>) -> Result<Self> {
        let backend = LlamaBackend::init()?;
        let params = LlamaModelParams::default().with_n_gpu_layers(gpu_layers);
        let model = LlamaModel::load_from_file(&backend, model_path, &params)
            .with_context(|| format!("failed to load model: {}", model_path.display()))?;
        Ok(Self {
            backend,
            model,
            mtmd: None,
            threads: threads.map_or_else(default_threads, |n| n.get() as i32),
        })
    }

    /// Text + image translation (additionally loads the mmproj projector)
    pub fn load_vision(
        model_path: &Path,
        mmproj_path: &Path,
        gpu_layers: u32,
        threads: Option<NonZeroU32>,
    ) -> Result<Self> {
        let mut t = Self::load(model_path, gpu_layers, threads)?;
        let params = MtmdContextParams {
            use_gpu: gpu_layers > 0,
            n_threads: t.threads,
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
        LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(N_BATCH as u32)
            .with_n_threads(self.threads)
            .with_n_threads_batch(self.threads)
    }

    /// Greedy decode loop (deterministic output); `stream` prints pieces to stdout as
    /// generated. `pos` is the next free position and is advanced past every
    /// generated token, so a conversation can continue.
    fn generate(
        &self,
        ctx: &mut LlamaContext,
        pos: &mut i32,
        max_tokens: usize,
        stream: bool,
    ) -> Result<String> {
        let vocab = self.model.vocab();
        let mut sampler = LlamaSampler::chain_simple([LlamaSampler::greedy()]);
        let mut batch = LlamaBatch::new(1, 1);
        let mut out_bytes: Vec<u8> = Vec::new();
        let stdout = std::io::stdout();
        for _ in 0..max_tokens {
            let token = sampler.sample(ctx, -1);
            sampler.accept(token);
            if vocab.is_eog(token) {
                break;
            }
            let piece = vocab.token_to_piece(token, false, None);
            out_bytes.extend_from_slice(&piece);
            if stream {
                let mut out = stdout.lock();
                if let Err(e) = out.write_all(&piece).and_then(|()| out.flush()) {
                    // The reader went away (e.g. `| head`): stop the whole translation
                    // instead of generating (and prefilling further chunks) for nobody
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        return Err(e.into());
                    }
                }
            }
            batch.clear();
            batch.add(token, *pos, &[0], true)?;
            *pos += 1;
            ctx.decode(&mut batch).context("decode failed")?;
        }
        let raw = String::from_utf8_lossy(&out_bytes);
        Ok(strip_think(&raw).to_string())
    }

    /// Append chat-template text (special tokens parsed) at `pos`, logits on the last token
    fn eval_text(&self, ctx: &mut LlamaContext, pos: &mut i32, text: &str) -> Result<()> {
        let tokens = self.model.vocab().tokenize(text.as_bytes(), false, true);
        let mut batch = LlamaBatch::new(N_BATCH, 1);
        for (n, piece) in tokens.chunks(N_BATCH).enumerate() {
            batch.clear();
            let last_piece = (n + 1) * N_BATCH >= tokens.len();
            for (i, t) in piece.iter().enumerate() {
                batch.add(*t, *pos, &[0], last_piece && i == piece.len() - 1)?;
                *pos += 1;
            }
            ctx.decode(&mut batch).context("failed to process prompt")?;
        }
        Ok(())
    }

    /// One-shot convenience wrapper around [`TextSession::translate`]
    pub fn translate(
        &self,
        text: &str,
        source: Option<Lang>,
        target: Lang,
        stream: bool,
    ) -> Result<String> {
        self.text_session()?.translate(text, source, target, stream)
    }

    /// A text-translation context that can be reused across requests
    pub fn text_session(&self) -> Result<TextSession<'_>> {
        let ctx = self
            .model
            .new_context(&self.backend, self.ctx_params(N_CTX))?;
        Ok(TextSession {
            translator: self,
            ctx,
        })
    }

    /// Chat-template tokens for one translation request
    fn prompt_tokens(&self, text: &str, source: Option<Lang>, target: Lang) -> Vec<LlamaToken> {
        let from = match source {
            Some(s) => format!("from {} ", s.name()),
            None => String::new(),
        };
        let head = format!(
            "<|im_start|>system\n\
             You are a professional translator. Translate the user's text {from}into {}. \
             Preserve the original meaning, tone and formatting (including line breaks). \
             Output only the translation, without any explanation.<|im_end|>\n\
             <|im_start|>user\n",
            target.name()
        );
        let tail = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n";

        // Only the template parses special tokens: markers such as "<|im_end|>" inside
        // the user's text must stay plain text, or they end the turn early and let
        // the text hijack the prompt.
        let vocab = self.model.vocab();
        let mut tokens = vocab.tokenize(head.as_bytes(), true, true);
        tokens.extend(vocab.tokenize(text.as_bytes(), false, false));
        tokens.extend(vocab.tokenize(tail.as_bytes(), false, true));
        tokens
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
                // checked_add: x + w can overflow u32 and slip past the bounds check
                let fits = |start: u32, len: u32, max: u32| {
                    len > 0 && start.checked_add(len).is_some_and(|end| end <= max)
                };
                if !fits(x, w, img.width()) || !fits(y, h, img.height()) {
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
        let marker = mtmd_default_marker();
        let assistant = "<|im_start|>assistant\n<think>\n\n</think>\n\n";
        let instruction = |target: Lang| {
            format!(
                "Read the text in this image and translate it {from}into {}. \
                 Output only the translation.<|im_end|>\n",
                target.name()
            )
        };
        let system = "<|im_start|>system\n\
             You are a professional translator. Output only the translation, without any explanation.<|im_end|>\n";

        let mut ctx = self
            .model
            .new_context(&self.backend, self.ctx_params(N_CTX_VISION))?;
        let mtmd = self
            .mtmd
            .as_mut()
            .context("mmproj not loaded; image translation unavailable")?;
        let mut pos = match target {
            Some(target) => {
                let prompt = format!(
                    "{system}<|im_start|>user\n{marker}\n{}{assistant}",
                    instruction(target)
                );
                eval_vision_prompt(mtmd, &mut ctx, &bitmap, prompt)?
            }
            // Auto direction: turn 1 asks whether the image text is Chinese, turn 2
            // translates with an explicit target (small models cannot handle a
            // conditional direction in a single prompt). Both turns share one
            // context, so the image is encoded only once.
            None => {
                let probe = format!(
                    "{system}<|im_start|>user\n{marker}\n\
                     Is the text in this image in Chinese? Answer only yes or no.<|im_end|>\n\
                     {assistant}"
                );
                let mut pos = eval_vision_prompt(mtmd, &mut ctx, &bitmap, probe)?;
                let answer = self.generate(&mut ctx, &mut pos, 8, false)?;
                let target = if answer.to_lowercase().contains("yes") {
                    Lang::En
                } else {
                    Lang::Zh
                };
                let follow_up = format!(
                    "<|im_end|>\n<|im_start|>user\n{}{assistant}",
                    instruction(target)
                );
                self.eval_text(&mut ctx, &mut pos, &follow_up)?;
                pos
            }
        };
        self.generate(&mut ctx, &mut pos, 2048, stream)
    }
}

/// Tokenize a prompt containing the image marker, then encode the image and
/// evaluate everything into `ctx`; returns the next free position
fn eval_vision_prompt(
    mtmd: &mut MtmdContext,
    ctx: &mut LlamaContext,
    bitmap: &MtmdBitmap,
    prompt: String,
) -> Result<i32> {
    let input = MtmdInputText {
        text: prompt,
        add_special: true,
        parse_special: true,
    };
    let chunks = mtmd
        .tokenize(input, &[bitmap])
        .map_err(|e| anyhow::anyhow!("failed to tokenize image: {e}"))?;
    chunks
        .eval_chunks(mtmd, ctx, 0, 0, N_BATCH as i32, true)
        .map_err(|e| anyhow::anyhow!("failed to encode image: {e}"))
}

/// Text translation context (KV cache) bound to a loaded model. Creating it
/// allocates the KV cache, so long-lived callers keep one and reuse it.
pub struct TextSession<'a> {
    translator: &'a Translator,
    ctx: LlamaContext<'a>,
}

impl TextSession<'_> {
    /// Number of tokens `text` takes up in a prompt
    pub fn token_count(&self, text: &str) -> usize {
        let vocab = self.translator.model.vocab();
        vocab.tokenize(text.as_bytes(), false, false).len()
    }

    /// Translate text of any length. Input longer than [`CHUNK_TOKENS`] is split at
    /// paragraph / line / sentence boundaries and translated chunk by chunk, keeping
    /// the whitespace between chunks, so prompt + output always fit in the context.
    /// `stream` prints the translation to stdout as it is generated.
    pub fn translate(
        &mut self,
        text: &str,
        source: Option<Lang>,
        target: Lang,
        stream: bool,
    ) -> Result<String> {
        let translator = self.translator;
        let vocab = translator.model.vocab();
        let count = |s: &str| vocab.tokenize(s.as_bytes(), false, false).len();
        // Whitespace between chunks is part of the output (streamed as well)
        let emit = |out: &mut String, s: &str| {
            out.push_str(s);
            if stream && !s.is_empty() {
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(s.as_bytes());
                let _ = stdout.flush();
            }
        };

        let mut out = String::with_capacity(text.len() * 2);
        for chunk in split_chunks(text, CHUNK_TOKENS, &count) {
            let body = chunk.trim();
            if body.is_empty() {
                emit(&mut out, chunk);
                continue;
            }
            let lead = chunk.len() - chunk.trim_start().len();
            emit(&mut out, &chunk[..lead]);
            out.push_str(&self.translate_chunk(body, source, target, stream)?);
            emit(&mut out, &chunk[lead + body.len()..]);
        }
        Ok(out)
    }

    fn translate_chunk(
        &mut self,
        text: &str,
        source: Option<Lang>,
        target: Lang,
        stream: bool,
    ) -> Result<String> {
        let translator = self.translator;
        let tokens = translator.prompt_tokens(text, source, target);
        let n_ctx = self.ctx.n_ctx() as usize;
        // split_chunks keeps chunks far below this; guard against overflowing anyway
        if tokens.len() + 128 > n_ctx {
            bail!("prompt too long ({} tokens, context {n_ctx})", tokens.len());
        }

        self.ctx.clear_kv_cache();
        // Chunked prefill: a single decode batch may not exceed n_batch,
        // and positions must be absolute across chunks.
        let mut batch = LlamaBatch::new(N_BATCH, 1);
        let mut pos = 0usize;
        while pos < tokens.len() {
            let end = (pos + N_BATCH).min(tokens.len());
            batch.clear();
            for (i, t) in tokens[pos..end].iter().enumerate() {
                let p = pos + i;
                batch.add(*t, p as i32, &[0], p == tokens.len() - 1)?;
            }
            self.ctx
                .decode(&mut batch)
                .context("failed to process prompt")?;
            pos = end;
        }

        // Output budget: generous for verbose target languages, but never past the
        // end of the context window (decoding there fails and loses everything).
        let max_out = (tokens.len() * 2 + 128).min(n_ctx - tokens.len());
        let mut pos = tokens.len() as i32;
        translator.generate(&mut self.ctx, &mut pos, max_out, stream)
    }
}

/// Split `text` into consecutive slices of at most `limit` tokens (as measured by
/// `count`), preferring blank lines (paragraphs), then line breaks, then sentence
/// ends, then the middle of the text. Concatenating the slices yields `text` exactly.
fn split_chunks<'t>(text: &'t str, limit: usize, count: &dyn Fn(&str) -> usize) -> Vec<&'t str> {
    let mut out = Vec::new();
    pack(text, 0, limit, count, &mut out);
    out
}

fn pack<'t>(
    text: &'t str,
    level: u8,
    limit: usize,
    count: &dyn Fn(&str) -> usize,
    out: &mut Vec<&'t str>,
) {
    if text.is_empty() {
        return;
    }
    if count(text) <= limit {
        out.push(text);
        return;
    }
    let pieces: Vec<&str> = match level {
        0 => text.split_inclusive("\n\n").collect(),
        1 => text.split_inclusive('\n').collect(),
        2 => sentences(text),
        _ => {
            // Last resort for a single huge "sentence": halve at a char boundary
            let mid = text
                .char_indices()
                .nth(text.chars().count() / 2)
                .map_or(text.len(), |(i, _)| i);
            pack(&text[..mid], level, limit, count, out);
            pack(&text[mid..], level, limit, count, out);
            return;
        }
    };

    // Greedily merge consecutive pieces into chunks of at most `limit` tokens;
    // pieces that are too big on their own are split at the next level.
    let (mut start, mut end, mut used) = (0usize, 0usize, 0usize);
    for piece in pieces {
        let n = count(piece);
        if n > limit {
            if end > start {
                out.push(&text[start..end]);
            }
            pack(&text[end..end + piece.len()], level + 1, limit, count, out);
            end += piece.len();
            (start, used) = (end, 0);
            continue;
        }
        if used + n > limit && end > start {
            out.push(&text[start..end]);
            (start, used) = (end, 0);
        }
        end += piece.len();
        used += n;
    }
    if end > start {
        out.push(&text[start..end]);
    }
}

/// Split after sentence-ending punctuation (CJK full-width marks, or ASCII marks
/// followed by whitespace so that "3.14" and "e.g." stay intact)
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let ends = matches!(c, '。' | '！' | '？' | '；')
            || (matches!(c, '.' | '!' | '?' | ';') && next.is_some_and(char::is_whitespace));
        if ends {
            let end = i + c.len_utf8();
            out.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Fallback: drop a reasoning block the model opened itself. The prompt pre-fills an
/// empty one, so a block can only appear at the very start; a "</think>" further on
/// belongs to the translation (e.g. a text about these tags) and is kept.
fn strip_think(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("<think>") {
        if let Some(end) = rest.find("</think>") {
            return rest[end + "</think>".len()..].trim();
        }
    }
    s
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
    fn detect_chinese_quoting_japanese() {
        let text = "他在日本留学时，最常说的一句话是「よろしくお願いします」，现在回国了还改不掉。";
        assert!(Lang::detect(text) == Lang::Zh);
    }

    #[test]
    fn detect_kanji_heavy_japanese() {
        assert!(Lang::detect("内閣総理大臣は記者会見で経済対策を発表した。") == Lang::Ja);
        assert!(Lang::detect("首相が来日") == Lang::Ja);
    }

    #[test]
    fn detect_by_dominant_script() {
        // A few CJK characters inside alphabetic text don't make it CJK
        let readme = "**English** | [简体中文](README.zh-CN.md)\n\nLocal, offline translation: \
                      Rust + llama.cpp running Qwen3.5, composed with other tools in pipelines.";
        assert!(Lang::detect(readme) == Lang::En);
        assert!(Lang::detect("In Japanese, people say ありがとう to thank someone.") == Lang::En);
        assert!(Lang::detect("Привет, это короткий тест with English") == Lang::Ru);
        // ... while Chinese with some English terms stays Chinese
        assert!(Lang::detect("使用 llama.cpp 和 Vulkan 加速推理") == Lang::Zh);
        assert!(Lang::detect("12345 !!!") == Lang::En);
    }

    /// Test tokenizer stand-in: one token per char
    fn chars(s: &str) -> usize {
        s.chars().count()
    }

    fn check_chunks(text: &str, limit: usize) -> Vec<&str> {
        let chunks = split_chunks(text, limit, &chars);
        assert_eq!(chunks.concat(), text);
        assert!(chunks.iter().all(|c| !c.is_empty() && chars(c) <= limit));
        chunks
    }

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(check_chunks("hello world", 100), vec!["hello world"]);
    }

    #[test]
    fn chunks_prefer_paragraphs() {
        let text = "aaaa\nbbbb\n\ncccc\ndd\n";
        assert_eq!(check_chunks(text, 12), vec!["aaaa\nbbbb\n\n", "cccc\ndd\n"]);
    }

    #[test]
    fn long_paragraph_splits_at_lines() {
        let text = "aaaa\nbbbb\ncccc\n";
        assert_eq!(check_chunks(text, 10), vec!["aaaa\nbbbb\n", "cccc\n"]);
    }

    #[test]
    fn long_line_splits_at_sentences() {
        let text = "One two. Three four! 五六。 Seven 3.14 eight";
        let chunks = check_chunks(text, 20);
        assert_eq!(
            chunks,
            vec!["One two. Three four!", " 五六。", " Seven 3.14 eight"]
        );
    }

    #[test]
    fn huge_sentence_is_halved() {
        let text = "x".repeat(100) + "中文".repeat(50).as_str();
        let chunks = check_chunks(&text, 16);
        assert!(chunks.len() >= 13);
    }

    #[test]
    fn direction_names_unambiguous_sources_only() {
        let resolve = |text| resolve_direction(text, None, None);
        assert!(resolve("日本語を勉強しています") == (Some(Lang::Ja), Lang::Zh));
        assert!(resolve("안녕하세요") == (Some(Lang::Ko), Lang::Zh));
        assert!(resolve("今天天气很好") == (None, Lang::En));
        assert!(resolve("Bonjour tout le monde") == (None, Lang::Zh));
        // explicit choices always win
        let explicit = resolve_direction("안녕하세요", Some(Lang::Ja), Some(Lang::En));
        assert!(explicit == (Some(Lang::Ja), Lang::En));
    }

    #[test]
    fn strip_think_only_drops_a_leading_block() {
        assert_eq!(strip_think("<think>\nhmm\n</think>\n\n你好"), "你好");
        assert_eq!(strip_think("  你好\n"), "你好");
        let about_tags = "模型把推理包裹在 <think> 和 </think> 标签中。";
        assert_eq!(strip_think(about_tags), about_tags);
    }

    #[test]
    fn detect_korean_with_hanja() {
        assert!(Lang::detect("大韓民國의 수도는 서울입니다") == Lang::Ko);
    }
}
