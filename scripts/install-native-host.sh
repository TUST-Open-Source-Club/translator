#!/usr/bin/env bash
# Install the Firefox native messaging host manifest (Linux/macOS).
# Usage: scripts/install-native-host.sh [path to the translate binary]
set -euo pipefail

HOST_NAME="qwen_translate"
EXT_ID="qwen-translate@local"

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ $# -ge 1 ]]; then
  BIN="$1"
elif [[ -x "$PROJECT_ROOT/target/release/translate" ]]; then
  BIN="$PROJECT_ROOT/target/release/translate"
elif command -v translate >/dev/null 2>&1; then
  BIN="$(command -v translate)"
elif [[ -x "$HOME/.local/bin/translate" ]]; then
  # installed by scripts/install.sh, but ~/.local/bin is not in PATH
  BIN="$HOME/.local/bin/translate"
elif [[ -x "$PROJECT_ROOT/translate" ]]; then
  # extracted release package
  BIN="$PROJECT_ROOT/translate"
else
  echo "translate binary not found; run cargo build --release first or pass the path" >&2
  exit 1
fi
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

if [[ "$(uname)" == "Darwin" ]]; then
  HOST_DIR="$HOME/Library/Application Support/Mozilla/NativeMessagingHosts"
  DATA_DIR="$HOME/Library/Application Support/qwen-translate"
else
  HOST_DIR="$HOME/.mozilla/native-messaging-hosts"
  DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/qwen-translate"
fi
mkdir -p "$HOST_DIR" "$DATA_DIR"

# The manifest "path" cannot carry arguments; use a wrapper to add --native-host
WRAPPER="$DATA_DIR/native-host.sh"
cat > "$WRAPPER" <<EOF
#!/bin/sh
exec "$BIN" --native-host "\$@"
EOF
chmod +x "$WRAPPER"

cat > "$HOST_DIR/$HOST_NAME.json" <<EOF
{
  "name": "$HOST_NAME",
  "description": "Local translation native messaging host (Qwen)",
  "path": "$WRAPPER",
  "type": "stdio",
  "allowed_extensions": ["$EXT_ID"]
}
EOF

echo "Installed: $HOST_DIR/$HOST_NAME.json"
echo "Wrapper: $WRAPPER -> $BIN --native-host"
echo "Next: open Firefox about:debugging#/runtime/this-firefox -> Load Temporary Add-on -> $PROJECT_ROOT/firefox/manifest.json"
