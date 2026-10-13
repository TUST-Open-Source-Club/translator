#!/usr/bin/env bash
# Share automatic downloads' file lock, HTTP validators and retry policy.
# VARIANT=default|fast|quality; MODEL_SOURCE=modelscope|huggingface;
# MODEL_REPO and MODEL_NAME retain the pre-download override interface.
set -euo pipefail
ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
SOURCE="${MODEL_SOURCE:-modelscope}"
case "$SOURCE" in
  modelscope|huggingface|hf) ;;
  *) echo "MODEL_SOURCE must be modelscope or huggingface" >&2; exit 1 ;;
esac
# TRANSLATE_BIN also supports custom CARGO_TARGET_DIR / cross-build host tools.
if [[ -n "${TRANSLATE_BIN:-}" ]]; then
  exec "$TRANSLATE_BIN" --download-from-env --model-source "$SOURCE"
fi
for bin in "$ROOT/translate" "$ROOT/translate.exe" \
           "$ROOT/target/release/translate" "$ROOT/target/release/translate.exe"; do
  # Source updates may leave an older release binary behind. Check support
  # without downloading anything before handing control to it.
  if [[ -x "$bin" ]] && "$bin" --download-from-env --help >/dev/null 2>&1; then
    exec "$bin" --download-from-env --model-source "$SOURCE"
  fi
done
if [[ -f "$ROOT/Cargo.toml" ]] && command -v cargo >/dev/null 2>&1; then
  exec cargo run --locked --release --manifest-path "$ROOT/Cargo.toml" -- \
    --download-from-env --model-source "$SOURCE"
fi
if command -v translate >/dev/null 2>&1; then
  exec translate --download-from-env --model-source "$SOURCE"
fi
echo "translate binary not found; build the project or run this script from a release package." >&2
exit 1
