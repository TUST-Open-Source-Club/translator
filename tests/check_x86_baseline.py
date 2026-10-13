"""Check the actual generated ggml CMake cache, after cargo build."""
from pathlib import Path
import re
import sys

target = Path(sys.argv[1] if len(sys.argv) > 1 else "target/release")
caches = [target] if target.is_file() else list(target.glob("build/llama-cpp-sys-2-*/out/build/CMakeCache.txt"))
if not caches:
    raise SystemExit(f"No llama.cpp CMake cache under {target}")
for cache in caches:
    text = cache.read_text(encoding="utf-8")
    enabled = re.findall(r"^GGML_(NATIVE|SSE42|AVX\w*|BMI2|FMA|F16C|AMX_\w+):BOOL=(?:ON|TRUE|1)$", text, re.M)
    if enabled:
        raise SystemExit(f"{cache}: non-baseline features enabled: {enabled}")
    for required in ("GGML_NATIVE", "GGML_AVX", "GGML_AVX2", "GGML_SSE42", "GGML_BMI2"):
        if not re.search(rf"^{required}:BOOL=OFF$", text, re.M):
            raise SystemExit(f"{cache}: missing explicit {required}=OFF")
    print(f"SSE2 release baseline verified: {cache}")
