[English](README.md) | **简体中文**

# translator

本地离线翻译工具：Rust + llama.cpp 跑 Qwen3.5（Q4_K_M 量化）。GPLv3 开源。

模型变体：**默认 2B**，`--fast` 切 0.8B，`--quality` 切 4B；各自缺失时自动下载（默认 ModelScope，可切 HuggingFace）。

这是一个**非常 Unix** 的项目：只做一件事——把 stdin（或参数）的文本翻译到 stdout——并且为与其他工具管道组合而设计。文本进，译文出，stdout 上没有任何多余输出。

```bash
echo "我喜欢编程" | translate                        # -> I like programming.
cat README.md | translate | less                    # 管道随便组
translate --to ja "你好，世界"                       # -> こんにちは、世界です。
translate --image screenshot.png                    # 图片翻译（多模态直译，非 OCR）
```

- 首次运行自动下载模型（默认 ModelScope，可选 HuggingFace）
- 9 种语言：中/英/日/韩/法/德/俄/西/葡；`--from`（默认 auto）与 `--to` 可用 `TRANSLATE_FROM`/`TRANSLATE_TO` 设默认，命令行参数优先
- `--image` 图片翻译，支持 `-x/-y/-w/-h` 区域裁剪（长参数 `--width`/`--height`）
- 默认流式输出（`--no-stream` 关闭）
- 有 GPU 用 GPU（Vulkan / CUDA / Metal），没有自动回退 CPU
- stdout 只输出结果，日志/报错全走 stderr
- 可选 Firefox / Chrome 扩展：右键「翻译选中内容」，由同一个二进制驱动
- 支持 Linux、macOS、Windows 与 Android（Termux 风格终端的独立应用，或在 Termux 中使用 CLI）

## 安装

### 从 Release（推荐）

到 [Releases](https://github.com/TUST-Open-Source-Club/translator/releases) 下载：

| 资产 | 说明 |
| --- | --- |
| `qwen-translate-*-linux-x86_64.tar.xz` | Linux 二进制，模型首用自动下载 |
| `qwen-translate-*-linux-x86_64-with-model.tar.xz` | 同上但内置模型，离线可用 |
| `qwen-translate_*_amd64.deb` / `*-1.x86_64.rpm` / `*-1-x86_64.pkg.tar.zst` | Debian / Fedora / Arch 安装包（不含模型） |
| `qwen-translate-full_*` / `qwen-translate-full-*` | 上述三格式的**带模型**版本（模型装进 `/usr/share/qwen-translate/`，离线可用） |
| `qwen-translate-*-macos-arm64.tar.xz`（及 `-with-model`） | macOS（Apple Silicon，Metal 加速） |
| `qwen-translate-*-windows-x86_64.zip`（及 `-with-model`） | Windows 10/11 x64（CPU；静态链接运行库，无需安装 VC++ 运行时） |
| `qwen-translate-*-android.apk` | Android 应用（Android 7+，arm64；终端风格，无需 Termux；见下文） |
| `qwen-translate-*-android-aarch64.tar.xz`（及 `-with-model`） | Android aarch64，用于 Termux（CPU；见下文） |
| `qwen-translate-extension-*-firefox.zip` / `-chrome.zip` / `-chrome.crx` | 浏览器扩展（见「浏览器扩展」一节） |

tar.xz 解压后运行 `scripts/install.sh`（装到 `~/.local/bin`）。macOS 二进制未签名，首次运行前需 `xattr -d com.apple.quarantine ~/.local/bin/translate`。

Windows：解压 zip 后运行 `powershell -ExecutionPolicy Bypass -File scripts\install.ps1`，`translate.exe` 装到 `%LOCALAPPDATA%\Programs\qwen-translate` 并加入用户 PATH（无需管理员权限），包内模型（如有）装到 `%LOCALAPPDATA%\qwen-translate`。也可不安装，直接在解压目录使用（会自动找到 exe 同级 `model\` 中的模型）。

### Android 应用

`qwen-translate-*-android.apk` 是独立的 Android 应用（Android 7+，arm64），终端部分直接采用 [Termux](https://termux.dev) 的终端模拟器与终端视图（`android/terminal-*`），自带 `translate`，无需安装 Termux：

- 打开即进入 `translate` 的交互模式：输入文字、回车即翻译，模型只加载一次。^D 退出到 shell（Android 自带的 `/system/bin/sh`，`translate` 已在 `PATH` 中），命令、管道、选项与在其他终端中一样：`translate --to ja 你好`、`echo hi | translate`、`translate --help`。`models` 列出已下载的模型；`export QWEN_TRANSLATE_MODEL=fast` 之类的设置写进 `~/.mkshrc`。
- 在任意应用中选中文字后，点选择菜单里的 **Qwen 翻译**；也可把文字或图片分享到本应用：各在单独的会话中运行，完成后按回车关闭。「图片」键用于选一张图片翻译。
- 快捷键行：ESC、^C、^D、TAB、方向键、图片、粘贴；最后一个键显示并切换会话（长按新建 shell）。双指缩放字号，长按选择并复制文字。
- 首次使用时自动下载模型（支持续传）到应用私有目录：默认 2B（1.3GB），内存较小的手机可用 `QWEN_TRANSLATE_MODEL=fast` 改用 0.8B（0.5GB）。卸载应用时模型一并删除。
- 与 Termux 一样，通知栏常驻通知让会话在后台继续运行，还可从通知中保持唤醒（锁屏时长时间下载）或退出全部会话。Android 12+ 仍可能终止在后台占用大量 CPU 的进程（显示 `[Process completed (signal 9)]`），长文本翻译请保持应用在前台。

### Android（Termux）

CLI 可在 Android（aarch64）的 [Termux](https://termux.dev) 中运行，使用 Release 中的 `android-aarch64` 资产：

```bash
tar xf qwen-translate-*-android-aarch64.tar.xz
bash qwen-translate-*-android-aarch64/scripts/install.sh   # 安装到 $PREFIX/bin
translate "I love programming"
termux-clipboard-get | translate | termux-clipboard-set   # 翻译剪贴板（需 Termux:API）
```

- 仅 CPU 推理。默认使用一半核心（常见大小核 SoC 上即大核），可用 `--threads` 调整。
- 所有平台共用内置 HTTPS 下载器，无需 curl，在 `adb shell` 中也能用。支持 `HTTPS_PROXY`/`NO_PROXY`；设置 `SSL_CERT_FILE` 时以其中的根证书替代内置根证书。CLI、浏览器宿主和预下载脚本共用文件锁。仅在同一 URL 的强 ETag 匹配时续传；旧分片、模型版本变化或没有可用校验标识时安全重下。网络读取停滞 30 秒后超时重试，不限制持续传输的总时长。
- 默认 2B 模型约需 2GB 空闲内存，内存较小的手机可用 `--fast`（0.8B，约 1GB）。
- 浏览器扩展仅限桌面端：移动端浏览器不支持 Native Messaging。

### 从源码构建

```bash
# Linux（有 Vulkan，推荐；GPU 不可用时会自动回退 CPU）
cargo build --release --features vulkan

# 纯 CPU
cargo build --release

# Linux + NVIDIA CUDA（需要 cuda 工具链）
cargo build --release --features cuda

# macOS（Metal）
cargo build --release --features metal

# Windows（纯 CPU；装好 Vulkan SDK 后可加 --features vulkan）
cargo build --release
```

构建依赖：cmake、clang、C++ 编译器。Linux Vulkan 构建还需要 SPIRV-Headers：

```bash
# Debian/Ubuntu: sudo apt install spirv-headers libvulkan-dev glslang-tools glslc
# Arch:          sudo pacman -S spirv-headers
# 或无 sudo 本地安装：
git clone --depth 1 https://github.com/KhronosGroup/SPIRV-Headers /tmp/spirv-headers
cmake -S /tmp/spirv-headers -B /tmp/spirv-headers/build -DCMAKE_INSTALL_PREFIX=$HOME/.local
cmake --build /tmp/spirv-headers/build --target install
CMAKE_PREFIX_PATH=$HOME/.local CPATH=$HOME/.local/include cargo build --release --features vulkan
```

Windows 构建依赖：Visual Studio Build Tools（C++ 工作负载）、CMake、LLVM（`winget install Kitware.CMake LLVM.LLVM`）。

Android 在 Linux 上用 [NDK](https://developer.android.com/ndk/downloads) 交叉编译（libc++ 静态链接，二进制除 Android 系统外无其它依赖）：

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

较新的手机可加 `GGML_CPU_ARM_ARCH=armv8.2-a+dotprod`，推理明显更快，但生成的二进制无法在 Cortex-A53/A73 一代的老 SoC 上运行。

Android 应用用 `scripts/build-android-app.sh` 构建（需要 `ANDROID_NDK` 指向 NDK、`ANDROID_HOME` 指向 Android SDK，以及 JDK 17+）：按给定 ABI（默认 `arm64-v8a`，模拟器可用 `x86_64`）交叉编译 `translate`，作为应用的原生库 `libtranslate.so` 打包（Android 只允许应用执行原生库目录中的文件），连同 `libtermux.so`（Termux 终端模拟器的伪终端部分），两者都按 16KB 页对齐链接以支持 Android 15+ 设备，输出 `android/app/build/outputs/apk/release/app-release.apk`。设置了 `ANDROID_KEYSTORE`（密码 `ANDROID_KEYSTORE_PASSWORD`，别名 `ANDROID_KEY_ALIAS`，默认 `qwen-translate`）时用该密钥签名，否则用 debug 密钥。

## 使用

```bash
translate "I love programming"       # 参数传入 → 译成中文
echo "我喜欢编程" | translate         # stdin 管道 → 译成英文
translate < file.txt                 # 文件重定向
cat file.txt | translate | less      # 管道组合
translate                            # 在终端中直接运行：交互模式，逐行翻译

translate --to ja "你好"             # 指定目标语言
translate --from ru --to zh "Привет" # 指定源语言（默认 auto 自动检测）
TRANSLATE_TO=ja translate "你好"     # 环境变量设默认目标语言（命令行参数优先）

translate --image shot.png                           # 整图翻译
translate --image shot.png -x 10 -y 20 -w 300 -h 200 # 只翻译指定区域

translate --fast "…"                 # 等效 --model fast；--quality/--default 同理
translate --download-only            # 下载全部模型变体后退出
translate --download-only --fast     # 只下载 0.8B 变体
translate --download-only --huggingface # 从 HuggingFace 下载（默认 ModelScope）
translate --model /path/to.gguf "…"  # GGUF 路径或变体关键字（fast/default/quality）
translate --no-stream "…"            # 关闭流式输出
translate --gpu-layers 0 "…"         # 强制纯 CPU
translate --threads 4 "…"            # CPU 推理线程数（默认每核一个；或环境变量 TRANSLATE_THREADS）
translate --model-source huggingface "…" # 自动下载走 HuggingFace（默认 modelscope）
translate --log-level info "…"       # 显示 llama.cpp 内部日志（默认 warn，静默）
```

参数与 stdin 同时存在时优先用参数。两者都没有且在终端中运行时进入交互模式：输入一行翻译一行，模型只加载一次（Ctrl+D 退出，Windows 为 Ctrl+Z 后回车；提示符输出到 stderr）。解码为贪心策略（确定性输出）。
stdin 支持 UTF-8（可带 BOM）与带 BOM 的 UTF-16；Windows 下控制台代码页编码的输入（如 GBK 环境的 cmd 中 `echo 你好 | translate`）也能正确识别。
Windows PowerShell 5.1 默认以 ASCII 向外部程序传管道、以 OEM 代码页解码捕获的输出，使用前先执行 `$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new()`。PowerShell 7 管道默认已是 UTF-8，但捕获输出仍按控制台代码页解码，同样需要设置 `[Console]::OutputEncoding`。直接显示在控制台的输出始终正常。
模型运行在非思考模式（Qwen3.5 本身即默认非思考，不支持 /no_think 软开关）。

模型查找顺序（命中即用）：

1. `--model` / `QWEN_TRANSLATE_MODEL`（可为变体关键字 fast/default/quality，或显式 GGUF 路径；显式路径缺失则报错，不自动下载）
2. 用户数据目录：Linux `~/.local/share/qwen-translate/`，macOS `~/Library/Application Support/qwen-translate/`，Windows `%LOCALAPPDATA%\qwen-translate\`
3. 可执行文件同级的 `model/` 目录（解压 `-with-model` 包后免安装直接使用）
4. 系统目录：`/usr/local/share/qwen-translate/`、`/usr/share/qwen-translate/`（macOS 额外含 Homebrew 前缀 `/opt/homebrew/share/qwen-translate/`；Windows 为 `%ProgramData%\qwen-translate\`）——带模型的安装包就装在这里
5. 都没有 → 自动下载到用户数据目录（默认 ModelScope）

手动预下载/换变体：`scripts/download-model.sh`（支持 `VARIANT=default/fast/quality`、`MODEL_SOURCE`、`MODEL_REPO`、`MODEL_NAME` 环境变量）。脚本调用打包或本地构建的 translate，共用锁和续传规则；源码目录没有二进制时会通过 Cargo 构建，也可用 `TRANSLATE_BIN` 指定宿主机二进制。

## 玩法

二进制就是一个普通过滤器，和什么都能组合。几个值得抄的玩法：

### 划词翻译弹窗（KDE 全局快捷键）

保存为 `/usr/local/bin/selection-translation`（或任意位置）并 `chmod +x`：

```bash
#!/bin/bash
text=$(wl-paste --primary --no-newline 2>/dev/null)              # Wayland
[ -z "$text" ] && text=$(xsel --primary --output 2>/dev/null)    # X11 兜底
result=$(echo "$text" | translate 2>/tmp/translate.err)
if [ $? -ne 0 ]; then
    notify-send "翻译失败" "$(cat /tmp/translate.err)"
else
    notify-send "翻译" "$result" --expire-time=15000
fi
```

KDE 绑定：系统设置 → 快捷键 → 自定义快捷键 → 新建 → 全局快捷键 → 命令/URL，触发键比如 `Meta+T`，命令填 `/usr/local/bin/selection-translation`。之后在任何应用里选中文字、按快捷键，译文以通知弹出（失败则弹出 stderr 里的报错）。GNOME：设置 → 键盘 → 自定义快捷键，同理。macOS 上把 `wl-paste`/`xsel` 换成 `pbpaste`。提示：加 `--fast` 弹窗更快。

### 剪贴板截图翻译

先截图到剪贴板（Spectacle、`grim -g "$(slurp)" - | wl-copy` 等），然后：

```bash
wl-paste > /tmp/shot.png && translate --image /tmp/shot.png
```

### Vim / Neovim：翻译可视选区

```
:'<,'>!translate
```

### 翻译文件：原地或输出到新文件

```bash
translate < notes.md | sponge notes.md      # 原地翻译（sponge 来自 moreutils）
translate < README.md > README.zh.md        # 输出到新文件
```

### 翻译其他程序的输出

```bash
./some-app --help | translate
make 2>&1 | translate                       # 连 stderr 一起翻译
```

### 复制即翻译（Wayland）

```bash
wl-paste --watch sh -c 't=$(translate --fast 2>/dev/null) && [ -n "$t" ] && notify-send 翻译 "$t" --expire-time=8000'
```

## 退出码

| 码 | 含义 |
| --- | --- |
| 0 | 成功 |
| 1 | 运行时错误（推理、IO 等） |
| 2 | 用法错误（参数非法、无输入、裁剪区域越界等） |
| 3 | 模型错误（显式路径缺失、加载失败等） |
| 4 | 下载失败 |
| 5 | 模型不支持所请求的功能（如图片） |

## 浏览器扩展（Firefox / Chrome）

任意网页选中文字后右键「翻译选中内容」，选区旁浮层显示译文（可复制）。Firefox 用 `firefox/`，Chrome 及其它 Chromium 系浏览器（Edge、Brave、Vivaldi 等）用 `chrome/`，两者调用同一个翻译程序。

1. 安装原生消息宿主清单（告知浏览器翻译程序的位置；Firefox 与本机已安装的 Chromium 系浏览器一并配置）：

   ```bash
   scripts/install-native-host.sh
   ```

   Windows（在 HKCU 下为 Firefox、Chrome、Edge、Chromium、Brave 注册，无需管理员权限）：

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\install-native-host.ps1
   ```

2. 载入扩展：
   - **Firefox**：`about:debugging#/runtime/this-firefox` → 「临时载入附加组件」→ 选择 `firefox/manifest.json`。
   - **Chrome / Edge / Brave**：`chrome://extensions` → 打开「开发者模式」→「加载已解压的扩展程序」→ 选择 `chrome/` 目录。扩展 ID 由 manifest 中的 `key` 固定为 `bfkmmaobjaldoelkdphajjagomjehbpc`，宿主清单已放行该 ID。

3. 任意网页选中文字 → 右键「翻译选中内容」→ 选区旁浮层显示译文。

打包版本：Release 提供 `qwen-translate-extension-*-chrome.crx` / `-chrome.zip` 与 `-firefox.zip`。Windows/macOS 上的 Chrome 不允许安装 Web Store 以外的 `.crx`，可解压 zip 后「加载已解压的扩展程序」，或通过企业策略（`ExtensionInstallForcelist` + 自建更新地址）部署 `.crx`；Linux 上可在开发者模式下把 `.crx` 拖进 `chrome://extensions`。自行打包：

```bash
scripts/pack-crx.py chrome path/to/key.pem qwen-translate.crx --zip qwen-translate.zip
```

私钥必须与 `chrome/manifest.json` 中的 `key` 对应（脚本会校验），否则打包出的扩展 ID 与宿主清单放行的 ID 不一致。若换用自己的私钥，需把其公钥写入 manifest 的 `key`，并同步修改宿主安装脚本中的 `CHROME_EXT_ID`。

说明：
- 扩展与翻译程序之间走 Native Messaging 连接，模型在两次翻译之间常驻内存，空闲 5 分钟后自动释放。
- 选区超过 4096 token 会被拒绝（整页翻译会让翻译程序忙上好几分钟），长文档请用命令行。
- 浮层脚本只在右键点击时通过 `activeTab` 注入当前页面（所在 frame），不申请「读取和更改所有网站数据」权限。
- Firefox 重启后临时加载的扩展会消失，需重新在 about:debugging 载入；长期使用可打包签名或用 `web-ext run`。
- 受限页面（`about:*`、`chrome://*`、扩展商店等）无法注入浮层，属于浏览器限制。
- Firefox 内置翻译功能不开放第三方翻译引擎 API，故采用右键菜单 + 浮层方案。
- 宿主协议（任何 native messaging 客户端都可调用）：请求 `{"id": 1, "text": "…", "to": "ja", "from": "auto"}`（仅 `text` 必填）→ 回复 `{"id": 1, "translation": "…"}` 或 `{"id": 1, "error": "…"}`。

## CI/CD

- **CI**（push / PR）：Ubuntu（Vulkan）+ macOS（Metal）+ Windows（CPU）构建 + 真实推理冒烟测试（文本 + 图片）；Android aarch64 交叉编译、APK 构建、应用 lint 与单元测试（APK 作为构件上传）
- **CD**（推送 `v*` tag 触发）：构建 Release 资产——各平台 tar.xz / zip（含/不含模型）+ deb / rpm / pacman 安装包（各含/不含模型两种），由单独作业汇总后以**草稿**形式创建 Release，人工确认后发布。仓库配置了 `CRX_PRIVATE_KEY` 密钥（与 `chrome/manifest.json` 中 `key` 对应的 PEM 私钥）时才会签名产出 Chrome `.crx`，否则只发布 zip。APK 用 secrets 中的发布密钥签名：`ANDROID_KEYSTORE_BASE64`（keystore 的 base64）和 `ANDROID_KEYSTORE_PASSWORD`（别名不是 `qwen-translate` 时另设 `ANDROID_KEY_ALIAS`）；未配置时使用一次性的 debug 密钥，之后的版本无法覆盖安装

## 卸载

```bash
rm ~/.mozilla/native-messaging-hosts/qwen_translate.json         # Linux，Firefox
rm ~/.config/*/NativeMessagingHosts/qwen_translate.json           # Linux，Chrome/Chromium/Edge/Vivaldi
rm ~/.config/BraveSoftware/*/NativeMessagingHosts/qwen_translate.json  # Linux，Brave
rm -rf ~/.local/share/qwen-translate                              # 模型与 wrapper
rm ~/.local/bin/translate
```

Windows（PowerShell）：

```powershell
scripts\install-native-host.ps1 -Uninstall                  # 浏览器注册信息
Remove-Item -Recurse "$env:LOCALAPPDATA\Programs\qwen-translate", "$env:LOCALAPPDATA\qwen-translate"
# 再从用户 PATH 中删除 %LOCALAPPDATA%\Programs\qwen-translate（设置 → 环境变量）
```

Android 应用像其他应用一样卸载即可，模型随之删除。

## 故障排查

| 问题 | 处理 |
| --- | --- |
| 自动下载失败 | 重新运行即可续传已下载部分；或手动 `scripts/download-model.sh`，或 `--model-source huggingface` 换源 |
| 浮层提示「无法启动翻译程序」/「native messaging host not found」 | 重新运行 `scripts/install-native-host.sh`（Windows：`scripts\install-native-host.ps1`），确认二进制路径存在；Chrome 需保持扩展 ID 为 `bfkmmaobjaldoelkdphajjagomjehbpc`（直接加载未修改的 `chrome/`） |
| PowerShell 中中文乱码 | 将控制台编码设为 UTF-8，见「使用」 |
| Vulkan 构建报 `SPIRV-Headers` 缺失 | 见上文「从源码构建」一节 |
| GPU 没生效 | 运行时会自动回退 CPU，属正常；检查 `vulkaninfo --summary`；CUDA 构建需另行安装 cuda 工具链 |

## License

[GPLv3](LICENSE)
