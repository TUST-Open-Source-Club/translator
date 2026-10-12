# translator

**English** | [简体中文](README.zh-CN.md)

Local, offline translation: Rust + llama.cpp running Qwen3.5 (Q4_K_M). GPLv3.

Model variants: **2B by default**, `--fast` switches to 0.8B, `--quality` to 4B. Each auto-downloads on first use (ModelScope by default, HuggingFace optional).

A **very Unix** project: it does one thing — translate text from stdin (or arguments) to stdout — and is designed to be composed with other tools in pipelines. Text in, translation out, nothing else on stdout.

```bash
echo "我喜欢编程" | translate                        # -> I like programming.
cat README.md | translate | less                    # pipe away
translate --to ja "你好，世界"                       # -> こんにちは、世界です。
translate --image screenshot.png                    # image translation (multimodal, not OCR)
```

- Auto-downloads the model on first run (ModelScope by default, HuggingFace optional)
- 9 languages: zh / en / ja / ko / fr / de / ru / es / pt; `--from` (default `auto`) and `--to`, overridable via `TRANSLATE_FROM` / `TRANSLATE_TO`
- Image translation via `--image` with optional region cropping (`-x/-y/-w/-h`, long forms `--width`/`--height`)
- Streaming output by default (`--no-stream` to disable)
- GPU when available (Vulkan / CUDA / Metal), silent CPU fallback otherwise
- stdout carries results only; logs and errors go to stderr
- Optional Firefox extension: right-click "Translate selection" backed by the same binary

## Install

### From releases

Grab an asset from [Releases](https://github.com/TUST-Open-Source-Club/translator/releases):

| Asset | Notes |
| --- | --- |
| `qwen-translate-*-linux-x86_64.tar.xz` | Linux binary; model auto-downloads on first run |
| `qwen-translate-*-linux-x86_64-with-model.tar.xz` | Same, with the model bundled (works offline) |
| `qwen-translate_*_amd64.deb` / `*-1.x86_64.rpm` / `*-1-x86_64.pkg.tar.zst` | Debian / Fedora / Arch packages (no model) |
| `qwen-translate-full_*` / `qwen-translate-full-*` | Same three formats **with the model** (installed to `/usr/share/qwen-translate/`, offline-ready) |
| `qwen-translate-*-macos-arm64.tar.xz` (and `-with-model`) | macOS (Apple Silicon, Metal-accelerated) |

For tar.xz, extract and run `scripts/install.sh` (installs to `~/.local/bin`). The macOS binary is unsigned; run `xattr -d com.apple.quarantine ~/.local/bin/translate` once.

### From source

```bash
# Linux with Vulkan (recommended; falls back to CPU at runtime if no GPU)
cargo build --release --features vulkan

# CPU only
cargo build --release

# Linux with NVIDIA CUDA (needs the CUDA toolkit)
cargo build --release --features cuda

# macOS (Metal)
cargo build --release --features metal
```

Build deps: cmake, clang, a C++ compiler. Linux Vulkan builds also need SPIRV-Headers:

```bash
# Debian/Ubuntu: sudo apt install spirv-headers libvulkan-dev glslang-tools glslc
# Arch:          sudo pacman -S spirv-headers
# or a sudo-less local install:
git clone --depth 1 https://github.com/KhronosGroup/SPIRV-Headers /tmp/spirv-headers
cmake -S /tmp/spirv-headers -B /tmp/spirv-headers/build -DCMAKE_INSTALL_PREFIX=$HOME/.local
cmake --build /tmp/spirv-headers/build --target install
CMAKE_PREFIX_PATH=$HOME/.local CPATH=$HOME/.local/include cargo build --release --features vulkan
```

## Usage

```bash
translate "I love programming"       # arguments -> Chinese
echo "我喜欢编程" | translate         # stdin pipe -> English
translate < file.txt                 # file redirect
cat file.txt | translate | less      # pipelines compose

translate --to ja "你好"             # pick a target language
translate --from ru --to zh "Привет" # pick a source language (default: auto)
TRANSLATE_TO=ja translate "你好"     # env var default; CLI flags win

translate --image shot.png                               # translate a whole image
translate --image shot.png -x 10 -y 20 -w 300 -h 200     # translate a region only

translate --model /path/to.gguf "…"  # GGUF path or variant keyword (fast/default/quality)
translate --fast "…"                 # same as --model fast; --quality/--default likewise
translate --download-only            # download all model variants, then exit
translate --download-only --fast     # download only the 0.8B variant
translate --download-only --huggingface # download via HuggingFace (default: ModelScope)
translate --no-stream "…"            # disable streaming output
translate --gpu-layers 0 "…"         # force CPU
translate --threads 4 "…"            # CPU threads (default: one per core; or TRANSLATE_THREADS)
translate --model-source huggingface "…" # download via HuggingFace (default: modelscope)
translate --log-level info "…"       # llama.cpp internals (default: warn, quiet)
```

Arguments win over stdin; with neither, you get the help text. Decoding is greedy (deterministic).
The model runs in non-thinking mode (Qwen3.5-0.8B is non-thinking by default; `/no_think` is not supported).

Model lookup order (first hit wins):

1. `--model` / `QWEN_TRANSLATE_MODEL` — a variant keyword (`fast`/`default`/`quality`) or an explicit GGUF path (explicit paths are never auto-downloaded; a missing file is an error)
2. User data dir: `~/.local/share/qwen-translate/` (Linux), `~/Library/Application Support/qwen-translate/` (macOS)
3. System dirs: `/usr/local/share/qwen-translate/`, `/usr/share/qwen-translate/` (plus `/opt/homebrew/share/qwen-translate/` on macOS) — where model-bundled packages install
4. Otherwise auto-downloads to the user data dir (ModelScope by default)

Pre-download or switch quants with `scripts/download-model.sh` (`MODEL_SOURCE` / `MODEL_REPO` / `MODEL_NAME` env overrides).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | runtime error (inference, I/O, …) |
| 2 | usage error (bad args, no input, text too long, …) |
| 3 | model error (missing explicit path, load failure, …) |
| 4 | download failure |
| 5 | the model does not support the requested feature (e.g. images) |

## Firefox extension

1. Install the native messaging host manifest:

   ```bash
   scripts/install-native-host.sh
   ```

2. Firefox → `about:debugging#/runtime/this-firefox` → "Load Temporary Add-on" → pick `firefox/manifest.json`.

3. Select text on any page → right-click "翻译选中内容" → a floating panel shows the translation (copyable).

Notes:
- The extension talks to the binary over a long-lived native-messaging connection; the model stays loaded, so only the first translation pays the load cost.
- Temporarily loaded extensions vanish on browser restart; reload via about:debugging, or use `web-ext run`.
- Restricted pages (`about:*`, `addons.mozilla.org`, …) block content scripts — a browser limitation.
- Firefox's built-in translation exposes no API for third-party engines, hence the context-menu + panel approach.

## CI/CD

- **CI** (push / PR): builds on Ubuntu (Vulkan) and macOS (Metal) + real-inference smoke tests (text and image)
- **CD** (push a `v*` tag): builds release assets — per-platform tar.xz (with/without model) + deb / rpm / pacman packages (with/without model) — and creates a **draft** GitHub Release for manual review before publishing

## Uninstall

```bash
rm ~/.mozilla/native-messaging-hosts/qwen_translate.json   # Linux
rm -rf ~/.local/share/qwen-translate                        # model & wrapper
rm ~/.local/bin/translate
```

## Troubleshooting

| Problem | Fix |
| --- | --- |
| Auto-download fails | Run `scripts/download-model.sh` manually, or `--model-source huggingface` |
| Panel says the translator can't start | Re-run `scripts/install-native-host.sh`; check the binary path |
| Vulkan build: `SPIRV-Headers` missing | See "From source" above |
| GPU not used | Runtime CPU fallback is normal; check `vulkaninfo --summary`; CUDA builds need the toolkit |

## License

[GPLv3](LICENSE)
