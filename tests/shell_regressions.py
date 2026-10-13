#!/usr/bin/env python3
"""Run installation scripts in temporary homes; never touches browser profiles.

Usage: python tests/shell_regressions.py [path/to/bash]
This exercises shell and JSON generation, not a browser session.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BASH = sys.argv[1] if len(sys.argv) > 1 else shutil.which("bash")
if not BASH:
    raise SystemExit("bash is required")


def run(args, env, cwd):
    argv = [BASH, *map(str, args)]
    if os.name == "nt":
        # MSYS also interprets bare single quotes in Windows command lines.
        # Quote every argv element (Python normally only quotes whitespace).
        def quote(value):
            value = re.sub(r'(\\*)"', r'\1\1\\"', value)
            value = re.sub(r'(\\+)$', r'\1\1', value)
            return '"' + value + '"'
        argv = " ".join(quote(value) for value in argv)
    result = subprocess.run(argv, env=env, cwd=cwd,
                            capture_output=True, text=True, encoding="utf-8", timeout=30)
    assert result.returncode == 0, (args, result.stdout, result.stderr)
    return result.stdout


with tempfile.TemporaryDirectory(prefix="translate-scripts-") as tmp:
    base = Path(tmp)
    names = ["spaces and 中文", "literal-$UNSET", "literal-$(touch unexpected)",
             "literal-`touch unexpected`", "single'quote"]
    if os.name != "nt":
        names += ['double"quote', "line\nbreak", "back\\slash", "tab\tname", "trailing\n"]
    for index, name in enumerate(names):
        case = base / name
        case.mkdir()
        binary = case / ("translate\n" if os.name != "nt" and name == "trailing\n" else "translate")
        binary.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n', encoding="utf-8")
        binary.chmod(0o755)
        home = case / "home"
        home.mkdir()
        env = {**os.environ, "HOME": str(home), "XDG_DATA_HOME": str(home / "data"),
               "XDG_CONFIG_HOME": str(home / "config"), "CDPATH": "."}
        env.pop("UNSET", None)
        wrapper = home / "data/qwen-translate/native-host.sh"
        for binary_arg in (binary, binary.relative_to(base)):
            run([ROOT / "scripts/install-native-host.sh", binary_arg], env, base)
            assert run([wrapper, "space arg", "literal$arg"], env, base).splitlines() == [
                "--native-host", "space arg", "literal$arg"]
        manifests = list(home.rglob("qwen_translate.json"))
        assert len(manifests) == 2
        for manifest in manifests:
            data = json.loads(manifest.read_text(encoding="utf-8"))
            assert data["path"].endswith("/qwen-translate/native-host.sh")
            assert name in data["path"]
        assert not (base / "unexpected").exists()

    # The legacy script must delegate to the lock-owning downloader and preserve
    # literal arguments. No model or network is needed for this dispatch check.
    binary = base / "fake downloader"
    binary.write_text('#!/bin/sh\nprintf "%s\\n" "$@" "$VARIANT" "$MODEL_REPO" "$MODEL_NAME"\n')
    binary.chmod(0o755)
    env = {**os.environ, "TRANSLATE_BIN": str(binary), "MODEL_SOURCE": "hf",
           "VARIANT": "fast", "MODEL_REPO": "fixture/repo", "MODEL_NAME": "custom.gguf"}
    assert run([ROOT / "scripts/download-model.sh"], env, base).splitlines() == [
        "--download-from-env", "--model-source", "hf", "fast", "fixture/repo", "custom.gguf"]
print(f"shell regressions OK ({len(names)} absolute/relative path cases with CDPATH plus downloader dispatch)")
