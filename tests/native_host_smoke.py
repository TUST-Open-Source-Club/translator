#!/usr/bin/env python3
"""Smoke test for the native messaging host.

Usage: tests/native_host_smoke.py <path to translate binary>

Starts the binary the way browsers do (Chrome: the extension origin, plus
--parent-window on Windows; Firefox: manifest path + add-on ID), sends requests
with the native messaging framing and checks the replies.
"""

import json
import os
import struct
import subprocess
import sys

CHROME_ORIGIN = "chrome-extension://bfkmmaobjaldoelkdphajjagomjehbpc/"
FIREFOX_ARGS = ["/tmp/native-host-firefox.json", "qwen-translate@local"]


def exchange(binary, args, requests):
    proc = subprocess.Popen([binary, *args], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    replies = []
    for req in requests:
        body = req if isinstance(req, bytes) else json.dumps(req).encode()
        proc.stdin.write(struct.pack("=I", len(body)) + body)
        proc.stdin.flush()
        header = proc.stdout.read(4)
        assert len(header) == 4, f"host closed the pipe (args {args})"
        (length,) = struct.unpack("=I", header)
        replies.append(json.loads(proc.stdout.read(length)))
    proc.stdin.close()
    assert proc.wait(timeout=60) == 0, "host did not exit cleanly on EOF"
    return replies


def main():
    # Absolute, native path: Windows' CreateProcess is picky about "./a/b.exe"
    binary = os.path.abspath(sys.argv[1])
    replies = exchange(
        binary,
        [CHROME_ORIGIN, "--parent-window=0"],
        [
            {"id": 1, "text": "I love programming"},
            {"id": "two", "text": "今天天气很好"},
            {"id": 3, "text": "hello", "to": "xx"},
            b"{not json",
        ],
    )
    print(json.dumps(replies, ensure_ascii=False))
    assert replies[0]["id"] == 1 and "编程" in replies[0]["translation"]
    assert replies[1]["id"] == "two" and "weather" in replies[1]["translation"].lower()
    assert replies[2]["id"] == 3 and "unsupported language" in replies[2]["error"]
    assert "invalid request" in replies[3]["error"]

    replies = exchange(binary, FIREFOX_ARGS, [{"text": "I love programming", "to": "zh"}])
    print(json.dumps(replies, ensure_ascii=False))
    assert "编程" in replies[0]["translation"]
    print("native host OK")


if __name__ == "__main__":
    main()
