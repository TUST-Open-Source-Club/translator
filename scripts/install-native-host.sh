#!/usr/bin/env bash
# Install the native messaging host manifests (Linux/macOS) for Firefox and the
# Chromium family (Chrome, Chromium, Edge, Brave, Vivaldi).
# Usage: scripts/install-native-host.sh [path to the translate binary]
set -euo pipefail

HOST_NAME="qwen_translate"
FIREFOX_EXT_ID="qwen-translate@local"
# Fixed by the "key" in chrome/manifest.json
CHROME_EXT_ID="bfkmmaobjaldoelkdphajjagomjehbpc"

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
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
# Preserve a legal trailing newline when resolving the binary directory.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    BIN="$(cygpath -u -- "$BIN" && printf .)"
    BIN="${BIN%.}"
    # cygpath adds one record-terminating newline of its own.
    BIN="${BIN%$'\n'}"
    ;;
esac
BIN_NAME="${BIN##*/}"
BIN_DIR="${BIN%/*}"
[[ "$BIN_DIR" == "$BIN" ]] && BIN_DIR=.
[[ -n "$BIN_DIR" ]] || BIN_DIR=/
BIN_DIR="$(CDPATH= cd -- "$BIN_DIR" && printf '%s.' "$PWD")"
BIN="${BIN_DIR%.}/$BIN_NAME"

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
# Quote a path as literal POSIX shell data, including dollars and single quotes.
shell_quote() {
  local escaped="${1//\'/\'\\\'\'}"
  printf "'%s'" "$escaped"
}
{
  printf '#!/bin/sh\nexec '
  shell_quote "$BIN"
  printf ' --native-host "$@"\n'
} > "$WRAPPER"
chmod +x "$WRAPPER"

# Native Messaging manifests need JSON escaping independently of shell quoting.
json_quote() {
  local value="$1" char code i
  local LC_ALL=C
  printf '"'
  for ((i=0; i<${#value}; i++)); do
    char="${value:i:1}"
    case "$char" in
      '"') printf '\\"' ;;
      '\') printf '\\\\' ;;
      *)
        printf -v code '%d' "'$char"
        if ((code < 32)); then printf '\\u%04x' "$code"; else printf '%s' "$char"; fi
        ;;
    esac
  done
  printf '"'
}
WRAPPER_JSON="$(json_quote "$WRAPPER")"

# Firefox: allowed_extensions holds add-on IDs
mkdir -p "$FIREFOX_HOST_DIR"
cat > "$FIREFOX_HOST_DIR/$HOST_NAME.json" <<JSON
{
  "name": "$HOST_NAME",
  "description": "Local translation native messaging host (Qwen)",
  "path": $WRAPPER_JSON,
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
  "path": $WRAPPER_JSON,
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
