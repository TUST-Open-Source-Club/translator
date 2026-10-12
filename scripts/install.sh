#!/usr/bin/env bash
# Installer for the translate release tarball:
# installs the binary into ~/.local/bin ($PREFIX/bin on Android/Termux) and the
# bundled model (if any) into the data dir.
# Usage: scripts/install.sh   (run from anywhere inside the extracted package)
set -euo pipefail

# This script lives in <package>/scripts/
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

TERMUX=false
if [[ -n "${TERMUX_VERSION:-}" || "${PREFIX:-}" == */com.termux/* ]]; then
  TERMUX=true
fi

if [[ "$(uname)" == "Darwin" ]]; then
  DATA_DIR="$HOME/Library/Application Support/qwen-translate"
else
  DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/qwen-translate"
fi
if $TERMUX; then
  # Android (Termux): $PREFIX/bin is already on PATH
  BIN_DIR="$PREFIX/bin"
else
  BIN_DIR="$HOME/.local/bin"
fi

mkdir -p "$BIN_DIR" "$DATA_DIR"
install -m755 "$ROOT/translate" "$BIN_DIR/translate"
echo "Installed: $BIN_DIR/translate"

# The -with-model packages carry both the model and the mmproj projector
shopt -s nullglob
models=("$ROOT"/model/*.gguf)
shopt -u nullglob
if (( ${#models[@]} )); then
  for m in "${models[@]}"; do
    cp "$m" "$DATA_DIR/"
    echo "Installed model: $DATA_DIR/$(basename "$m")"
  done
else
  echo "This package has no bundled model; it will be auto-downloaded on first run (via ModelScope)."
  if ! command -v curl >/dev/null 2>&1; then
    if $TERMUX; then
      echo "Note: the download needs curl; install it with: pkg install curl"
    else
      echo "Note: the download needs curl; install it with your package manager"
    fi
  fi
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "Note: $BIN_DIR is not in PATH; add it in your shell config (e.g. ~/.bashrc or ~/.zshrc)" ;;
esac

if ! $TERMUX; then
  echo "Browser extension: run $ROOT/scripts/install-native-host.sh, then load $ROOT/firefox (Firefox) or $ROOT/chrome (Chrome/Edge); see README"
fi
