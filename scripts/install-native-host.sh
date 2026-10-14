#!/usr/bin/env bash
# Install the native messaging host manifests (Linux/macOS) for Firefox and the
# Chromium family (Chrome, Chromium, Edge, Brave, Vivaldi).
# Usage: scripts/install-native-host.sh [path to the translate binary]
set -euo pipefail

HOST_NAME="qwen_translate"
FIREFOX_EXT_ID="qwen-translate@local"
# Fixed by the "key" in chrome/manifest.json
CHROME_EXT_ID="bfkmmaobjaldoelkdphajjagomjehbpc"

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ $# -ge 1 ]]; then
  BIN="$1"
elif [[ -x "$PROJECT_ROOT/target/release/translate" ]]; then
  BIN="$PROJECT_ROOT/target/release/translate"
elif command -v translate >/dev/null 2>&1; then
  BIN="$(command -v translate)"
else
  echo "translate binary not found; run cargo build --release first or pass the path" >&2
  exit 1
fi
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

if [[ "$(uname)" == "Darwin" ]]; then
  SUPPORT="$HOME/Library/Application Support"
  DATA_DIR="$SUPPORT/qwen-translate"
  FIREFOX_HOST_DIR="$SUPPORT/Mozilla/NativeMessagingHosts"
  CHROMIUM_ROOTS=(
    "$SUPPORT/Google/Chrome"
    "$SUPPORT/Chromium"
    "$SUPPORT/Microsoft Edge"
    "$SUPPORT/BraveSoftware/Brave-Browser"
    "$SUPPORT/Vivaldi"
  )
else
  CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
  DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/qwen-translate"
  FIREFOX_HOST_DIR="$HOME/.mozilla/native-messaging-hosts"
  CHROMIUM_ROOTS=(
    "$CONFIG/google-chrome"
    "$CONFIG/chromium"
    "$CONFIG/microsoft-edge"
    "$CONFIG/BraveSoftware/Brave-Browser"
    "$CONFIG/vivaldi"
  )
fi
mkdir -p "$DATA_DIR"

# The manifest "path" cannot carry arguments; use a wrapper to add --native-host
WRAPPER="$DATA_DIR/native-host.sh"
cat > "$WRAPPER" <<WRAP
#!/bin/sh
exec "$BIN" --native-host "\$@"
WRAP
chmod +x "$WRAPPER"

# Firefox: allowed_extensions holds add-on IDs
mkdir -p "$FIREFOX_HOST_DIR"
cat > "$FIREFOX_HOST_DIR/$HOST_NAME.json" <<JSON
{
  "name": "$HOST_NAME",
  "description": "Local translation native messaging host (Qwen)",
  "path": "$WRAPPER",
  "type": "stdio",
  "allowed_extensions": ["$FIREFOX_EXT_ID"]
}
JSON
echo "Installed (Firefox): $FIREFOX_HOST_DIR/$HOST_NAME.json"

# Chromium family: allowed_origins holds extension origins. Only browsers with an
# existing profile directory are set up; Google Chrome is always set up.
for root in "${CHROMIUM_ROOTS[@]}"; do
  if [[ ! -d "$root" && "$root" != "${CHROMIUM_ROOTS[0]}" ]]; then
    continue
  fi
  mkdir -p "$root/NativeMessagingHosts"
  cat > "$root/NativeMessagingHosts/$HOST_NAME.json" <<JSON
{
  "name": "$HOST_NAME",
  "description": "Local translation native messaging host (Qwen)",
  "path": "$WRAPPER",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://$CHROME_EXT_ID/"]
}
JSON
  echo "Installed (Chromium family): $root/NativeMessagingHosts/$HOST_NAME.json"
done

echo "Wrapper: $WRAPPER -> $BIN --native-host"
echo "Next:"
echo "  Firefox: about:debugging#/runtime/this-firefox -> Load Temporary Add-on -> $PROJECT_ROOT/firefox/manifest.json"
echo "  Chrome/Edge: chrome://extensions -> Developer mode -> Load unpacked -> $PROJECT_ROOT/chrome"
