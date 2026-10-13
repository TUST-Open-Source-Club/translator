#!/usr/bin/env bash
# Download the GGUF model + mmproj projector weights into the platform data dir.
# Downloads from ModelScope by default; MODEL_SOURCE=huggingface for HuggingFace.
# VARIANT=default (2B) | fast (0.8B) | quality (4B); default is "default".
set -euo pipefail

VARIANT="${VARIANT:-default}"
case "$VARIANT" in
  default)
    REPO="unsloth/Qwen3.5-2B-GGUF"
    MODEL_NAME="Qwen3.5-2B-Q4_K_M.gguf"
    MMPROJ_LOCAL="Qwen3.5-2B-mmproj-F16.gguf"
    ;;
  fast)
    REPO="unsloth/Qwen3.5-0.8B-GGUF"
    MODEL_NAME="Qwen3.5-0.8B-Q4_K_M.gguf"
    MMPROJ_LOCAL="mmproj-F16.gguf"
    ;;
  quality)
    REPO="unsloth/Qwen3.5-4B-GGUF"
    MODEL_NAME="Qwen3.5-4B-Q4_K_M.gguf"
    MMPROJ_LOCAL="Qwen3.5-4B-mmproj-F16.gguf"
    ;;
  *)
    echo "VARIANT must be default, fast or quality" >&2
    exit 1
    ;;
esac
REPO="${MODEL_REPO:-$REPO}"
SOURCE="${MODEL_SOURCE:-modelscope}"
MMPROJ_REMOTE="mmproj-F16.gguf"

if [[ "$(uname)" == "Darwin" ]]; then
  DEST_DIR="$HOME/Library/Application Support/qwen-translate"
else
  DEST_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/qwen-translate"
fi
mkdir -p "$DEST_DIR"

download_one() {
  local remote="$1" local_name="$2"
  local dest="$DEST_DIR/$local_name"
  if [[ -f "$dest" ]]; then
    echo "Already exists: $dest" >&2
    return 0
  fi

  local urls=()
  case "$SOURCE" in
    modelscope)
      urls=("https://modelscope.cn/models/$REPO/resolve/master/$remote")
      ;;
    huggingface|hf)
      urls=(
        "https://hf-mirror.com/$REPO/resolve/main/$remote"
        "https://huggingface.co/$REPO/resolve/main/$remote"
      )
      ;;
    *)
      echo "MODEL_SOURCE must be modelscope or huggingface" >&2
      exit 1
      ;;
  esac

  # A partial download is kept in $dest.tmp and resumed (-C -) on the next attempt
  local url rc
  for url in "${urls[@]}"; do
    echo "Downloading: $url" >&2
    rc=0
    curl -fL --connect-timeout 15 --retry 3 --retry-delay 2 -C - -o "$dest.tmp" "$url" || rc=$?
    if [[ $rc -eq 33 ]]; then
      # Server ignores Range requests: start over
      rm -f "$dest.tmp"
      rc=0
      curl -fL --connect-timeout 15 --retry 3 --retry-delay 2 -o "$dest.tmp" "$url" || rc=$?
    fi
    if [[ $rc -eq 0 ]]; then
      mv "$dest.tmp" "$dest"
      echo "Done: $dest" >&2
      return 0
    fi
  done

  echo "Download failed: $remote (rerun to resume the partial download)" >&2
  return 1
}

download_one "$MODEL_NAME" "$MODEL_NAME"
download_one "$MMPROJ_REMOTE" "$MMPROJ_LOCAL"
