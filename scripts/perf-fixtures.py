#!/usr/bin/env python3
"""Create disposable large-file benchmarks in a new directory (prints its absolute path)."""
import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import zlib

cli = argparse.ArgumentParser(description=__doc__)
cli.add_argument("--engine", help="Prebuilt engine benchmark executable; avoids Cargo during suite collection")
cli.add_argument("directory", nargs="?", help="New or empty directory; defaults to a temporary directory")
opt = cli.parse_args()
root = Path(opt.directory).resolve() if opt.directory else Path(tempfile.mkdtemp(prefix="lenz-perf-"))
root.mkdir(parents=True, exist_ok=True)
if any(root.iterdir()):
    cli.error("refusing to overwrite a nonempty directory")
repo = Path(__file__).resolve().parent.parent
subprocess.run(
    ([str(Path(opt.engine).resolve()), "--write-fixtures", str(root)] if opt.engine else
     ["cargo", "bench", "--locked", "-p", "lenz-engine", "--bench", "engine", "--", "--write-fixtures", str(root)]),
    cwd=repo, check=True, env=dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1"),
)
for side in ("left", "right"):
    directory = root / "files" / side
    directory.mkdir(parents=True)
    for i in range(12):
        shutil.copyfile(root / "generated-100k" / f"{side}.rs", directory / f"file-{i:02}.rs")
    directory = root / "long"
    directory.mkdir(exist_ok=True)
    (directory / f"{side}.txt").write_text(("let " + side + " = value; needle " + "a" * 20 + "\t") * 40_000 + "\n")

merge = root / "merge"
merge.mkdir()
base = [f"let v{i} = compute({i}, {i % 13});\n" for i in range(100_000)]
(merge / "base.rs").write_text("".join(base))
(merge / "merged.rs").write_text("".join(base))
for side, every in (("ours", 89), ("theirs", 97)):
    (merge / f"{side}.rs").write_text("".join(line.rstrip("\n") + f" // {side}\n" if i % every == 0 else line for i, line in enumerate(base)))


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


images = root / "images-4k"
images.mkdir()
for side in ("left", "right"):
    encoder = zlib.compressobj(3)
    payload = bytearray()
    for y in range(4096):
        row = bytearray(bytes([24, 32, 48, 255]) * 4096)
        if side == "right" and 1900 <= y < 2028:
            row[1900 * 4:2028 * 4] = bytes([200, 80, 60, 255]) * 128
        payload.extend(encoder.compress(b"\0" + row))
    payload.extend(encoder.flush())
    path = images / f"{side}.png"
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 4096, 4096, 8, 6, 0, 0, 0)) + chunk(b"IDAT", payload) + chunk(b"IEND", b""))
    directory = root / "image-files" / side
    directory.mkdir(parents=True)
    for i in range(6):
        shutil.copyfile(path, directory / f"file-{i}.png")
for label, left, right in [
    ("small", b"one\ntwo\nthree\n", b"one\nTWO\nthree\n"),
    ("unicode", ("café λ K İ 漢字\n" * 20000).encode(), ("CAFÉ λ K i 漢字\n" * 20000).encode()),
    ("crlf", b"old\r\nsame\r\n" * 20000, b"new\r\nsame\r\n" * 20000),
    ("windows1252", b"caf\xe9 ancien\n" * 20000, b"caf\xe9 nouveau\n" * 20000),
    ("binary", b"old\x00binary", b"new\x00binary"),
    ("oversized", b"left\n" * 5000000, b"right\n" * 5000000),
]:
    directory = root / label
    directory.mkdir()
    (directory / "left.txt").write_bytes(left)
    (directory / "right.txt").write_bytes(right)
for side in ("left", "right"):
    directory = root / "many-small" / side
    directory.mkdir(parents=True)
    for i in range(1000):
        (directory / f"file-{i:04}.txt").write_text(f"same\n{side} {i}\nsame\n")

# Deterministic local Git history, without network, user identity/config or hooks.
history = root / "history"
history.mkdir()
env = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1")
subprocess.run(["git", "-c", "init.defaultBranch=main", "init", "-q", str(history)], env=env, check=True)
stream = bytearray()
for i in range(500):
    data = f"first\nline {i}\nlast\n".encode()
    msg = f"fixture commit {i}\n".encode()
    stream.extend(f"commit refs/heads/main\ncommitter Benchmark <benchmark@example.invalid> {1700000000+i} +0000\ndata {len(msg)}\n".encode())
    stream.extend(msg)
    stream.extend(f"M 100644 inline history.txt\ndata {len(data)}\n".encode())
    stream.extend(data + b"\n")
subprocess.run(["git", "-C", str(history), "fast-import", "--quiet"], input=stream, env=env, check=True)
subprocess.run(["git", "-C", str(history), "reset", "--hard", "-q", "HEAD"], env=env, check=True)
print(root)
