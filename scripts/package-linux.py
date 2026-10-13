#!/usr/bin/env python3
"""Build deb/rpm/pacman packages from the same staged release files."""
import argparse
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

# Repair upgrades from the old 0.1.0.alpha / 0.1.0-alpha packages too. Merely
# switching new prereleases to '~' would leave the existing alpha above final.
EPOCH = "1"
DEPENDENCIES = {
    "deb": ("libvulkan1", "libgomp1", "libstdc++6"),
    "rpm": ("vulkan-loader", "libgomp", "libstdc++"),
    "pacman": ("vulkan-icd-loader", "libgomp", "libstdc++"),
}
MODELS = ("Qwen3.5-2B-Q4_K_M.gguf", "Qwen3.5-2B-mmproj-F16.gguf")


def encode_prerelease(prerelease):
    """Preserve SemVer ordering in one lowercase alphabetic package segment.

    Native package comparators split digits, letters and punctuation differently.
    Encoding avoids those rules: all three compare one lowercase segment
    lexicographically. 'a' terminates an identifier (before any encoded data);
    'b' and 'c' distinguish numeric from nonnumeric identifiers. Numeric lengths
    are unary so arbitrary-size integers keep their numeric order without int().
    Non-numeric identifiers use fixed-width base-25 ASCII codes in 'b'..'z'.
    A shorter identifier or tuple therefore sorts before an extension of itself.
    """
    parts = []
    for identifier in prerelease.split("."):
        if identifier.isascii() and identifier.isdigit():
            parts.append("b" + "c" * len(identifier) + "b"
                         + "".join(chr(ord("d") + ord(c) - ord("0")) for c in identifier))
        else:
            parts.append("c" + "".join(chr(ord("b") + ord(c) // 25)
                                        + chr(ord("b") + ord(c) % 25) for c in identifier))
        parts.append("a")
    return "".join(parts)


def package_version(tag, kind):
    number = r"(?:0|[1-9][0-9]*)"
    identifier = r"[0-9A-Za-z-]+"
    match = re.fullmatch(rf"v?({number}\.{number}\.{number})"
                         rf"(?:-({identifier}(?:\.{identifier})*))?"
                         rf"(?:\+{identifier}(?:\.{identifier})*)?", tag)
    if not match:
        raise ValueError(f"expected a semantic version tag, got {tag!r}")
    base, prerelease = match.groups()
    if not prerelease:
        return base
    if any(part.isdigit() and len(part) > 1 and part.startswith("0")
           for part in prerelease.split(".")):
        raise ValueError(f"numeric prerelease identifiers must not have leading zeroes: {tag!r}")
    encoded = encode_prerelease(prerelease)
    # Pacman alphabetic suffixes sort before an otherwise identical final;
    # a dot between the base and suffix would reverse that order.
    if kind == "pacman":
        return base + "pre" + encoded
    return base + "~" + encoded


def build_packages(repo, binary, models, output, tag):
    # Reject invalid/untrusted tags before they reach output paths or fpm.
    versions = {kind: package_version(tag, kind) for kind in DEPENDENCIES}
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="qwen-package-") as tmp:
        stage = Path(tmp)
        bin_dir = stage / "usr/bin"
        share = stage / "usr/share/qwen-translate"
        bin_dir.mkdir(parents=True)
        share.mkdir(parents=True)
        shutil.copy2(binary, bin_dir / "translate")
        for directory in ("firefox", "chrome", "scripts"):
            shutil.copytree(repo / directory, share / directory)
        for filename in ("README.md", "README.zh-CN.md", "LICENSE"):
            shutil.copy2(repo / filename, share / filename)
        for full in (False, True):
            name = "qwen-translate-full" if full else "qwen-translate"
            if full:
                for model in MODELS:
                    shutil.copy2(models / model, share / model)
            for kind, dependencies in DEPENDENCIES.items():
                version = versions[kind]
                filename = {
                    "deb": f"{name}_{tag}_amd64.deb",
                    "rpm": f"{name}-{tag}-1.x86_64.rpm",
                    "pacman": f"{name}-{tag}-1-x86_64.pkg.tar.zst",
                }[kind]
                command = ["fpm", "-s", "dir", "-C", str(stage), "-t", kind,
                           "-n", name, "-v", version, "--epoch", EPOCH,
                           "--iteration", "1", "--license", "GPL-3.0-only",
                           "--maintainer", "TUST-Open-Source-Club",
                           "--url", "https://github.com/TUST-Open-Source-Club/translator",
                           "--description", "Local offline translation CLI (Qwen3.5-2B + llama.cpp); "
                           + ("model bundled" if full else "model auto-downloads on first run")
                           + f"; release {tag}",
                           "--conflicts", "qwen-translate" if full else "qwen-translate-full",
                           "-p", str(output / filename)]
                for dependency in dependencies:
                    command += ["--depends", dependency]
                subprocess.run(command, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag")
    parser.add_argument("--binary", type=Path, default=Path("target/release/translate"))
    parser.add_argument("--models", type=Path, default=Path.home() / ".local/share/qwen-translate")
    parser.add_argument("--output", type=Path, default=Path("dist"))
    args = parser.parse_args()
    # Validate before creating any packages.
    package_version(args.tag, "deb")
    build_packages(Path(__file__).resolve().parent.parent, args.binary.resolve(),
                   args.models.resolve(), args.output.resolve(), args.tag)
