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
- Optional Firefox / Chrome extensions: right-click "Translate selection" backed by the same binary
- Linux, macOS, Windows and Android (a standalone app with a Termux-style terminal, or the CLI in Termux)

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
| `qwen-translate-*-windows-x86_64.zip` (and `-with-model`) | Windows 10/11 x64 (CPU; static CRT, no runtime to install) |
| `qwen-translate-*-android.apk` | Android app (Android 7+, arm64; terminal-style, no Termux needed; see below) |
| `qwen-translate-*-android-aarch64.tar.xz` (and `-with-model`) | Android aarch64 for Termux (CPU; see below) |
| `qwen-translate-extension-*-firefox.zip` / `-chrome.zip` / `-chrome.crx` | Browser extensions (see [Browser extensions](#browser-extensions-firefox--chrome)) |

For tar.xz, extract and run `scripts/install.sh` (installs to `~/.local/bin`). The macOS binary is unsigned; run `xattr -d com.apple.quarantine ~/.local/bin/translate` once.

On Windows, extract the zip and run `powershell -ExecutionPolicy Bypass -File scripts\install.ps1`: it installs `translate.exe` to `%LOCALAPPDATA%\Programs\qwen-translate` (added to the user PATH; no admin rights needed) and the bundled model, if any, to `%LOCALAPPDATA%\qwen-translate`. The extracted folder also works as-is (the model is found in `model\` next to the exe).

### Android app

`qwen-translate-*-android.apk` is a standalone app (Android 7+, arm64) whose terminal is built from [Termux](https://termux.dev)'s terminal emulator and view (`android/terminal-*`). It carries `translate`, so Termux itself isn't needed:

- It opens in `translate`'s interactive mode: type text and press Enter to translate it, with the model loaded once. ^D leaves for the shell (Android's `/system/bin/sh`, with `translate` on the `PATH`), where commands, pipes and options work as anywhere: `translate --to ja 你好`, `echo hi | translate`, `translate --help`. `models` lists the downloaded models; settings such as `export QWEN_TRANSLATE_MODEL=fast` go in `~/.mkshrc`.
- In any app, select text and choose **Qwen 翻译** / **Qwen Translate** in the selection menu, or share text or an image to the app: each runs in a session of its own, closed with Enter once done. The IMAGE key picks a picture to translate.
- The key row has ESC, ^C, ^D, TAB, the arrow keys, IMAGE and PASTE; the last key shows and switches sessions (long press: a new shell). Pinch to zoom; long press to select and copy text.
- Models download on first use (resumable) into the app's private storage: 2B (1.3 GB) by default, or 0.8B (0.5 GB) with `QWEN_TRANSLATE_MODEL=fast` on phones with little memory. Uninstalling the app removes them.
- As in Termux, a notification keeps the sessions alive in the background; it can also hold a wake lock (for a long download with the screen off) and exit all sessions. Android 12+ may still kill background processes that use a lot of CPU (`[Process completed (signal 9)]`); keep the app in the foreground for long translations.

### Android (Termux)

The CLI runs on Android (aarch64) inside [Termux](https://termux.dev), using the `android-aarch64` release asset:

```bash
tar xf qwen-translate-*-android-aarch64.tar.xz
bash qwen-translate-*-android-aarch64/scripts/install.sh   # installs to $PREFIX/bin
translate "I love programming"
termux-clipboard-get | translate | termux-clipboard-set   # translate the clipboard (Termux:API)
```

- CPU only. By default half of the cores are used (the big cores on typical big.LITTLE SoCs); tune with `--threads`.
- All builds use the same built-in HTTPS downloader (including `adb shell`), with no curl dependency. It honours `HTTPS_PROXY`/`NO_PROXY`, and `SSL_CERT_FILE` replaces the bundled root certificates. Downloads share a file lock across the CLI, browser host and pre-download script. Resuming requires a matching strong ETag from the same URL; legacy partials, changed models or mirrors without a usable validator restart safely. A stalled network read times out after 30 seconds and retries, without limiting the total duration of an active download.
- The default 2B model needs about 2 GB of free RAM; on low-memory phones use `--fast` (0.8B, about 1 GB).
- The browser extensions are desktop-only: mobile browsers don't support native messaging.

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

# Windows (CPU; or --features vulkan with the Vulkan SDK installed)
cargo build --release
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

Windows build deps: Visual Studio Build Tools (C++ workload), CMake and LLVM (`winget install Kitware.CMake LLVM.LLVM`).

Android is cross-compiled on Linux with the [NDK](https://developer.android.com/ndk/downloads) (libc++ is linked statically, so the binary needs nothing beyond Android itself):

```bash
rustup target add aarch64-linux-android
export ANDROID_NDK=/path/to/android-ndk ANDROID_API_LEVEL=24
TC=$ANDROID_NDK/toolchains/llvm/prebuilt/linux-x86_64/bin
export CC_aarch64_linux_android=$TC/aarch64-linux-android24-clang \
       CXX_aarch64_linux_android=$TC/aarch64-linux-android24-clang++ \
       AR_aarch64_linux_android=$TC/llvm-ar \
       CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$TC/aarch64-linux-android24-clang \
       BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--target=aarch64-linux-android24
cargo build --release --target aarch64-linux-android
```

For newer phones, `GGML_CPU_ARM_ARCH=armv8.2-a+dotprod` speeds up inference considerably, but the binary then no longer runs on older Cortex-A53/A73-class SoCs.

The Android app is built with `scripts/build-android-app.sh` (needs the NDK in `ANDROID_NDK`, the Android SDK in `ANDROID_HOME` and JDK 17+). It cross-compiles `translate` for the ABIs given (`arm64-v8a` by default, `x86_64` for emulators), packs it as the app's native library (`libtranslate.so`: Android only lets apps execute files from their native library directory) next to `libtermux.so` (the pseudo-terminal part of Termux's emulator), both linked with 16 KB page alignment for Android 15+ devices, and writes `android/app/build/outputs/apk/release/app-release.apk`. The APK is signed with the key in `ANDROID_KEYSTORE` (password `ANDROID_KEYSTORE_PASSWORD`, alias `ANDROID_KEY_ALIAS`, default `qwen-translate`), or else with the debug key.

## Usage

```bash
translate "I love programming"       # arguments -> Chinese
echo "我喜欢编程" | translate         # stdin pipe -> English
translate < file.txt                 # file redirect
cat file.txt | translate | less      # pipelines compose
translate                            # in a terminal: interactive, one translation per line

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

Arguments win over stdin. Run in a terminal with neither, `translate` is interactive: it translates each line you type, keeping the model loaded in between (Ctrl+D, or Ctrl+Z then Enter on Windows, to quit; prompts go to stderr). Decoding is greedy (deterministic).
stdin may be UTF-8 (with or without BOM) or UTF-16 with BOM; on Windows, input in the console code page (e.g. `echo 你好 | translate` in cmd.exe with GBK) is decoded too.
In Windows PowerShell 5.1, pipes to native programs are ASCII-encoded by default and captured output is decoded with the OEM code page; run `$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new()` first. PowerShell 7 pipes UTF-8 already but still decodes captured output with the console code page, so set `[Console]::OutputEncoding` there too. Output shown directly in the console is always correct.
The model runs in non-thinking mode (Qwen3.5-0.8B is non-thinking by default; `/no_think` is not supported).

Model lookup order (first hit wins):

1. `--model` / `QWEN_TRANSLATE_MODEL` — a variant keyword (`fast`/`default`/`quality`) or an explicit GGUF path (explicit paths are never auto-downloaded; a missing file is an error)
2. User data dir: `~/.local/share/qwen-translate/` (Linux), `~/Library/Application Support/qwen-translate/` (macOS), `%LOCALAPPDATA%\qwen-translate\` (Windows)
3. `model/` next to the executable (portable use of an extracted `-with-model` package)
4. System dirs: `/usr/local/share/qwen-translate/`, `/usr/share/qwen-translate/` (plus `/opt/homebrew/share/qwen-translate/` on macOS; `%ProgramData%\qwen-translate\` on Windows) — where model-bundled packages install
5. Otherwise auto-downloads to the user data dir (ModelScope by default)

Pre-download or switch variants with `scripts/download-model.sh` (`VARIANT=default/fast/quality`, `MODEL_SOURCE`, `MODEL_REPO`, and `MODEL_NAME` env overrides). It calls the packaged or locally built translator so all download entry points use the same locking and resume rules. In a source checkout without a binary it builds with Cargo; `TRANSLATE_BIN` selects a custom host binary.

## Recipes

The binary is a plain filter, so it composes with anything. A few setups worth stealing:

### Selection translation popup (KDE global shortcut)

Save as `/usr/local/bin/selection-translation` (or anywhere) and `chmod +x`:

```bash
#!/bin/bash
text=$(wl-paste --primary --no-newline 2>/dev/null)              # Wayland
[ -z "$text" ] && text=$(xsel --primary --output 2>/dev/null)    # X11 fallback
result=$(echo "$text" | translate 2>/tmp/translate.err)
if [ $? -ne 0 ]; then
    notify-send "Translation failed" "$(cat /tmp/translate.err)"
else
    notify-send "Translation" "$result" --expire-time=15000
fi
```

Bind it in KDE: System Settings → Shortcuts → Custom Shortcuts → New → Global Shortcut → Command/URL, trigger e.g. `Meta+T`, command `/usr/local/bin/selection-translation`. Select text anywhere, hit the shortcut, get the translation as a notification (on failure you get the stderr message instead). GNOME: Settings → Keyboard → Custom Shortcuts, same idea. On macOS swap `wl-paste`/`xsel` for `pbpaste`. Tip: add `--fast` if you want the popup quicker.

### Screenshot-to-clipboard image translation

Take a screenshot into the clipboard (Spectacle, `grim -g "$(slurp)" - | wl-copy`, …), then:

```bash
wl-paste > /tmp/shot.png && translate --image /tmp/shot.png
```

### Vim / Neovim: translate the visual selection

```
:'<,'>!translate
```

### Translate a file, in place or into a new one

```bash
translate < notes.md | sponge notes.md      # in place (sponge from moreutils)
translate < README.md > README.zh.md        # into a new file
```

### Translate other programs' output

```bash
./some-app --help | translate
make 2>&1 | translate                       # stderr included
```

### Auto-translate whatever you copy (Wayland)

```bash
wl-paste --watch sh -c 't=$(translate --fast 2>/dev/null) && [ -n "$t" ] && notify-send Translation "$t" --expire-time=8000'
```

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | runtime error (inference, I/O, …) |
| 2 | usage error (bad args, no input, region out of bounds, …) |
| 3 | model error (missing explicit path, load failure, …) |
| 4 | download failure |
| 5 | the model does not support the requested feature (e.g. images) |

## Browser extensions (Firefox / Chrome)

Right-click "翻译选中内容" (translate selection) on any page; a floating panel next to the selection shows the translation (copyable). Firefox uses `firefox/`, Chrome and other Chromium browsers (Edge, Brave, Vivaldi, …) use `chrome/`; both talk to the same binary.

1. Install the native messaging host manifests (Firefox + every Chromium browser found):

   ```bash
   scripts/install-native-host.sh
   ```

   On Windows (registers Firefox, Chrome, Edge, Chromium and Brave under HKCU; no admin rights needed):

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\install-native-host.ps1
   ```

2. Load the extension:
   - **Firefox**: `about:debugging#/runtime/this-firefox` → "Load Temporary Add-on" → pick `firefox/manifest.json`.
   - **Chrome / Edge / Brave**: `chrome://extensions` → enable "Developer mode" → "Load unpacked" → pick the `chrome/` directory. The extension ID is fixed (`bfkmmaobjaldoelkdphajjagomjehbpc`, from the `key` in its manifest), which the host manifest allows.

3. Select text on any page → right-click "翻译选中内容" → the panel shows the translation.

Packaged builds: releases ship `qwen-translate-extension-*-chrome.crx` / `-chrome.zip` and `-firefox.zip`. Chrome on Windows/macOS refuses to install `.crx` files from outside the Web Store, so there either use "Load unpacked" on the extracted zip or deploy the `.crx` through enterprise policy (`ExtensionInstallForcelist` with a self-hosted update URL); on Linux the `.crx` can be dragged onto `chrome://extensions` (developer mode). To pack one yourself:

```bash
scripts/pack-crx.py chrome path/to/key.pem qwen-translate.crx --zip qwen-translate.zip
```

The key must match the `key` in `chrome/manifest.json` (the script checks), otherwise the packed extension gets a different ID than the one the native host allows. If you use your own key, put its public part in the manifest `key` and update `CHROME_EXT_ID` in the host install scripts.

Notes:
- The extension talks to the binary over a native-messaging connection; the model stays loaded between translations and is released after 5 idle minutes.
- Selections over 4096 tokens are refused (a whole page would keep the translator busy for minutes); use the CLI for long documents.
- The panel script is injected only into the page (frame) you right-click, via `activeTab` — no access to all sites is requested.
- Temporarily loaded Firefox add-ons vanish on browser restart; reload via about:debugging, or use `web-ext run`.
- Restricted pages (`about:*`, `chrome://*`, extension stores, …) can't be scripted — a browser limitation.
- Firefox's built-in translation exposes no API for third-party engines, hence the context-menu + panel approach.
- Host protocol (usable from any native-messaging client): request `{"id": 1, "text": "…", "to": "ja", "from": "auto"}` (only `text` is required) → reply `{"id": 1, "translation": "…"}` or `{"id": 1, "error": "…"}`.

## CI/CD

- **CI** (push / PR): builds on Ubuntu (Vulkan), macOS (Metal) and Windows (CPU) + real-inference smoke tests (text and image); Android aarch64 cross-compile, APK build, app lint and unit tests (the APK is uploaded as a build artifact)
- **CD** (push a `v*` tag): builds release assets — per-platform tar.xz / zip (with/without model) + deb / rpm / pacman packages (with/without model) — and a single job creates a **draft** GitHub Release for manual review before publishing. The Chrome `.crx` is signed only when the repository secret `CRX_PRIVATE_KEY` (PEM key matching `chrome/manifest.json`'s `key`) is set; otherwise only the zip is published. The APK is signed with the release key from the secrets `ANDROID_KEYSTORE_BASE64` (the keystore, base64-encoded) and `ANDROID_KEYSTORE_PASSWORD` (plus `ANDROID_KEY_ALIAS` if not `qwen-translate`); without them it gets a throwaway debug key, and later versions can't be installed over it

## Uninstall

```bash
rm ~/.mozilla/native-messaging-hosts/qwen_translate.json         # Linux, Firefox
rm ~/.config/*/NativeMessagingHosts/qwen_translate.json           # Linux, Chrome/Chromium/Edge/Vivaldi
rm ~/.config/BraveSoftware/*/NativeMessagingHosts/qwen_translate.json  # Linux, Brave
rm -rf ~/.local/share/qwen-translate                              # model & wrapper
rm ~/.local/bin/translate
```

Windows (PowerShell):

```powershell
scripts\install-native-host.ps1 -Uninstall                  # browser registrations
Remove-Item -Recurse "$env:LOCALAPPDATA\Programs\qwen-translate", "$env:LOCALAPPDATA\qwen-translate"
# then remove %LOCALAPPDATA%\Programs\qwen-translate from the user PATH (Settings → Environment variables)
```

The Android app is uninstalled like any other app; its models go with it.

## Troubleshooting

| Problem | Fix |
| --- | --- |
| Auto-download fails | Rerun to resume the partial download; or run `scripts/download-model.sh`, or use `--model-source huggingface` |
| Panel says the translator can't start / "native messaging host not found" | Re-run `scripts/install-native-host.sh` (Windows: `scripts\install-native-host.ps1`); check the binary path; Chrome needs the extension ID `bfkmmaobjaldoelkdphajjagomjehbpc` (load `chrome/` unchanged) |
| Garbled Chinese in PowerShell | Set the console encodings to UTF-8, see "Usage" |
| Vulkan build: `SPIRV-Headers` missing | See "From source" above |
| GPU not used | Runtime CPU fallback is normal; check `vulkaninfo --summary`; CUDA builds need the toolkit |

## License

[GPLv3](LICENSE)
