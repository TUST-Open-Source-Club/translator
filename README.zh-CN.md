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
- 可选 Firefox 扩展：右键「翻译选中内容」，由同一个二进制驱动

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

tar.xz 解压后运行 `scripts/install.sh`（装到 `~/.local/bin`）。macOS 二进制未签名，首次运行前需 `xattr -d com.apple.quarantine ~/.local/bin/translate`。

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
模型运行在非思考模式（Qwen3.5 本身即默认非思考，不支持 /no_think 软开关）。

模型查找顺序（命中即用）：

1. `--model` / `QWEN_TRANSLATE_MODEL`（可为变体关键字 fast/default/quality，或显式 GGUF 路径；显式路径缺失则报错，不自动下载）
2. 用户数据目录：Linux `~/.local/share/qwen-translate/`，macOS `~/Library/Application Support/qwen-translate/`
3. 系统目录：`/usr/local/share/qwen-translate/`、`/usr/share/qwen-translate/`（macOS 额外含 Homebrew 前缀 `/opt/homebrew/share/qwen-translate/`）——带模型的安装包就装在这里
4. 都没有 → 自动下载到用户数据目录（默认 ModelScope）

手动预下载/换变体：`scripts/download-model.sh`（支持 `VARIANT=default/fast/quality`、`MODEL_SOURCE` 环境变量）。

## 退出码

| 码 | 含义 |
| --- | --- |
| 0 | 成功 |
| 1 | 运行时错误（推理、IO 等） |
| 2 | 用法错误（参数非法、无输入、文本过长等） |
| 3 | 模型错误（显式路径缺失、加载失败等） |
| 4 | 下载失败 |
| 5 | 模型不支持所请求的功能（如图片） |

## Firefox 扩展

1. 安装原生消息宿主清单（告知 Firefox 翻译程序的位置）：

   ```bash
   scripts/install-native-host.sh
   ```

2. 打开 Firefox → `about:debugging#/runtime/this-firefox` → 「临时载入附加组件」→ 选择 `firefox/manifest.json`。

3. 任意网页选中文字 → 右键「翻译选中内容」→ 选区旁浮层显示译文（可复制）。

说明：
- 扩展与翻译程序之间走 Native Messaging 长连接，模型常驻内存，只有第一次翻译需要加载模型。
- 浏览器重启后临时加载的扩展会消失，需重新在 about:debugging 载入；长期使用可打包签名或用 `web-ext run`。
- 受限页面（`about:*`、`addons.mozilla.org` 等）无法注入浮层，属于浏览器限制。
- Firefox 内置翻译功能不开放第三方翻译引擎 API，故采用右键菜单 + 浮层方案。

## CI/CD

- **CI**（push / PR）：Ubuntu（Vulkan）+ macOS（Metal）构建 + 真实推理冒烟测试（文本 + 图片）
- **CD**（推送 `v*` tag 触发）：构建 Release 资产——双平台 tar.xz（含/不含模型）+ deb / rpm / pacman 安装包（各含/不含模型两种），以**草稿**形式创建 Release，人工确认后发布

## 卸载

```bash
rm ~/.mozilla/native-messaging-hosts/qwen_translate.json   # Linux
rm -rf ~/.local/share/qwen-translate                        # 模型与 wrapper
rm ~/.local/bin/translate
```

## 故障排查

| 问题 | 处理 |
| --- | --- |
| 自动下载失败 | 手动 `scripts/download-model.sh`，或 `--model-source huggingface` 换源 |
| 浮层提示「无法启动翻译程序」 | 确认 `scripts/install-native-host.sh` 已运行且二进制路径存在 |
| Vulkan 构建报 `SPIRV-Headers` 缺失 | 见上文「从源码构建」一节 |
| GPU 没生效 | 运行时会自动回退 CPU，属正常；检查 `vulkaninfo --summary`；CUDA 构建需另行安装 cuda 工具链 |

## License

[GPLv3](LICENSE)
