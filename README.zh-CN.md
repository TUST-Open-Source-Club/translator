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
| `qwen-translate-*-android-aarch64.tar.xz`（及 `-with-model`） | Android aarch64，用于 Termux（CPU；见下文） |
| `qwen-translate-extension-*-firefox.zip` / `-chrome.zip` / `-chrome.crx` | 浏览器扩展（见「浏览器扩展」一节） |

tar.xz 解压后运行 `scripts/install.sh`（装到 `~/.local/bin`）。macOS 二进制未签名，首次运行前需 `xattr -d com.apple.quarantine ~/.local/bin/translate`。

Windows：解压 zip 后运行 `powershell -ExecutionPolicy Bypass -File scripts\install.ps1`，`translate.exe` 装到 `%LOCALAPPDATA%\Programs\qwen-translate` 并加入用户 PATH（无需管理员权限），包内模型（如有）装到 `%LOCALAPPDATA%\qwen-translate`。也可不安装，直接在解压目录使用（会自动找到 exe 同级 `model\` 中的模型）。

### Android（Termux）

CLI 可在 Android（aarch64）的 [Termux](https://termux.dev) 中运行，使用 Release 中的 `android-aarch64` 资产：

```bash
pkg install curl                                      # 自动下载模型需要 curl
tar xf qwen-translate-*-android-aarch64.tar.xz
bash qwen-translate-*-android-aarch64/scripts/install.sh   # 安装到 $PREFIX/bin
translate "I love programming"
termux-clipboard-get | translate | termux-clipboard-set   # 翻译剪贴板（需 Termux:API）
```

- 仅 CPU 推理。默认使用一半核心（常见大小核 SoC 上即大核），可用 `--threads` 调整。
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
       CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$TC/aarch64-linux-android24-clang
cargo build --release --target aarch64-linux-android
```

较新的手机可加 `GGML_CPU_ARM_ARCH=armv8.2-a+dotprod`，推理明显更快，但生成的二进制无法在 Cortex-A53/A73 一代的老 SoC 上运行。

## 使用

```bash
translate "I love programming"       # 参数传入 → 译成中文
echo "我喜欢编程" | translate         # stdin 管道 → 译成英文
translate < file.txt                 # 文件重定向
cat file.txt | translate | less      # 管道组合

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

参数与 stdin 同时存在时优先用参数；两者都没有时打印帮助。解码为贪心策略（确定性输出）。
stdin 支持 UTF-8（可带 BOM）与带 BOM 的 UTF-16；Windows 下控制台代码页编码的输入（如 GBK 环境的 cmd 中 `echo 你好 | translate`）也能正确识别。
Windows PowerShell 5.1 默认以 ASCII 向外部程序传管道、以 OEM 代码页解码捕获的输出，使用前先执行 `$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new()`。PowerShell 7 管道默认已是 UTF-8，但捕获输出仍按控制台代码页解码，同样需要设置 `[Console]::OutputEncoding`。直接显示在控制台的输出始终正常。
模型运行在非思考模式（Qwen3.5 本身即默认非思考，不支持 /no_think 软开关）。

模型查找顺序（命中即用）：

1. `--model` / `QWEN_TRANSLATE_MODEL`（可为变体关键字 fast/default/quality，或显式 GGUF 路径；显式路径缺失则报错，不自动下载）
2. 用户数据目录：Linux `~/.local/share/qwen-translate/`，macOS `~/Library/Application Support/qwen-translate/`，Windows `%LOCALAPPDATA%\qwen-translate\`
3. 可执行文件同级的 `model/` 目录（解压 `-with-model` 包后免安装直接使用）
4. 系统目录：`/usr/local/share/qwen-translate/`、`/usr/share/qwen-translate/`（macOS 额外含 Homebrew 前缀 `/opt/homebrew/share/qwen-translate/`；Windows 为 `%ProgramData%\qwen-translate\`）——带模型的安装包就装在这里
5. 都没有 → 自动下载到用户数据目录（默认 ModelScope）

手动预下载/换变体：`scripts/download-model.sh`（支持 `VARIANT=default/fast/quality`、`MODEL_SOURCE` 环境变量）。

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

- **CI**（push / PR）：Ubuntu（Vulkan）+ macOS（Metal）+ Windows（CPU）构建 + 真实推理冒烟测试（文本 + 图片）；Android aarch64 交叉编译检查
- **CD**（推送 `v*` tag 触发）：构建 Release 资产——各平台 tar.xz / zip（含/不含模型）+ deb / rpm / pacman 安装包（各含/不含模型两种），由单独作业汇总后以**草稿**形式创建 Release，人工确认后发布。仓库配置了 `CRX_PRIVATE_KEY` 密钥（与 `chrome/manifest.json` 中 `key` 对应的 PEM 私钥）时才会签名产出 Chrome `.crx`，否则只发布 zip

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
