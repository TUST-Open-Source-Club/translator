#!/usr/bin/env python3
"""Pack an extension directory into a CRX3 package (Chrome / Edge / Brave …).

Usage:
  scripts/pack-crx.py <extension dir> <private key .pem> <output .crx>
                      [--zip <output .zip>] [--version <x.y.z>]

Needs only the Python standard library and the openssl CLI. The extension ID is
derived from the signing key; the key must match the "key" in the extension's
manifest.json, so the packed extension keeps the ID the native host allows.
"""

import argparse
import base64
import hashlib
import io
import json
import os
import re
import struct
import subprocess
import sys
import zipfile


def openssl(*args, data=None):
    return subprocess.run(
        ["openssl", *args], input=data, capture_output=True, check=True
    ).stdout


def extension_id(public_key_der):
    digest = hashlib.sha256(public_key_der).hexdigest()[:32]
    return "".join(chr(ord("a") + int(c, 16)) for c in digest)


def zip_extension(src, version):
    """Deterministic zip of `src` (sorted entries, fixed timestamps)."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        for root, dirs, files in os.walk(src):
            dirs[:] = sorted(d for d in dirs if not d.startswith("."))
            for name in sorted(files):
                if name.startswith("."):
                    continue
                path = os.path.join(root, name)
                arcname = os.path.relpath(path, src).replace(os.sep, "/")
                with open(path, "rb") as f:
                    data = f.read()
                if arcname == "manifest.json" and version:
                    manifest = json.loads(data)
                    manifest["version"] = version
                    data = (json.dumps(manifest, ensure_ascii=False, indent=2) + "\n").encode()
                info = zipfile.ZipInfo(arcname, date_time=(1980, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = 0o644 << 16
                zf.writestr(info, data)
    return buf.getvalue()


def varint(n):
    out = bytearray()
    while True:
        byte = n & 0x7F
        n >>= 7
        if n:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def field(number, payload):
    """Protobuf length-delimited field"""
    return varint(number << 3 | 2) + varint(len(payload)) + payload


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("src", help="extension directory (contains manifest.json)")
    parser.add_argument("key", help="RSA private key (PEM)")
    parser.add_argument("out", help="output .crx")
    parser.add_argument("--zip", help="also write the plain zip (Web Store / Load unpacked)")
    parser.add_argument("--version", help="override the manifest version (1-4 dot-separated integers)")
    args = parser.parse_args()

    if args.version and not re.fullmatch(r"\d+(\.\d+){0,3}", args.version):
        sys.exit(f"invalid extension version: {args.version}")

    public_key = openssl("pkey", "-in", args.key, "-pubout", "-outform", "DER")
    with open(os.path.join(args.src, "manifest.json"), encoding="utf-8") as f:
        manifest_key = json.load(f).get("key")
    if manifest_key and base64.b64decode(manifest_key) != public_key:
        sys.exit(
            "the signing key does not match the manifest \"key\"; the packed extension "
            "would get a different ID than the one the native host allows"
        )

    archive = zip_extension(args.src, args.version)

    # CRX3: "Cr24", version 3, header length, CrxFileHeader protobuf, zip archive.
    # The signature covers a fixed prefix, the signed header data and the archive.
    crx_id = hashlib.sha256(public_key).digest()[:16]
    signed_data = field(1, crx_id)  # SignedData.crx_id
    signature = openssl(
        "dgst",
        "-sha256",
        "-sign",
        args.key,
        data=b"CRX3 SignedData\x00" + struct.pack("<I", len(signed_data)) + signed_data + archive,
    )
    proof = field(1, public_key) + field(2, signature)  # AsymmetricKeyProof
    header = field(2, proof) + field(10000, signed_data)  # sha256_with_rsa, signed_header_data
    with open(args.out, "wb") as f:
        f.write(b"Cr24" + struct.pack("<II", 3, len(header)) + header + archive)
    if args.zip:
        with open(args.zip, "wb") as f:
            f.write(archive)
    print(f"{args.out}: extension ID {extension_id(public_key)}")


if __name__ == "__main__":
    main()
