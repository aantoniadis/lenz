#!/usr/bin/env python3
"""Startup/RSS probe: private Xvfb on Linux, explicit --native desktop windows on macOS."""
import argparse
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import subprocess
import tempfile
import time
import platform
from perf_support import process_stats, prepare_gui_launch, isolate_user_state


def measure(binary, args, env, seconds, cwd=None):
    proc = subprocess.Popen([binary, *args], env=env, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    output = bytearray()
    start = time.monotonic()
    paint = None
    rss = peak = post_peak = 0
    idle_start = None
    try:
        with selectors.DefaultSelector() as events:
            events.register(proc.stdout, selectors.EVENT_READ)
            while time.monotonic() - start < 30:
                if events.select(0.1 if platform.system() == "Darwin" else 0.02):
                    chunk = os.read(proc.stdout.fileno(), 65536)
                    if not chunk:
                        break
                    output.extend(chunk)
                sample = process_stats(proc.pid)
                rss, cpu = sample['rss_mib'], sample['cpu_seconds']
                peak = max(peak, rss, sample['peak_rss_mib'] or 0)
                if paint is None:
                    match = re.search(rb"lenz: first paint (\d+) ms", output)
                    if match:
                        paint = (int(match[1]), time.monotonic())
                if paint:
                    post_peak = max(post_peak, rss)
                    if idle_start is None and time.monotonic() - paint[1] >= max(0, seconds - 1):
                        idle_start = (time.monotonic(), cpu)
                    if time.monotonic() - paint[1] >= seconds:
                        phases = re.search(rb"lenz: load (\d+) ms, diff (\d+) ms", output)
                        return {
                            "diagnostics": output.decode(errors="replace"),
                            "first_paint_ms": paint[0],
                            "load_ms": int(phases[1]) if phases else None,
                            "diff_ms": int(phases[2]) if phases else None,
                            "rss_mib": round(rss, 1),
                            "resident_mib": round(sample['resident_mib'], 1) if sample.get('resident_mib') else None,
                            "peak_rss_mib": round(peak, 1),
                            "post_paint_peak_mib": round(post_peak, 1),
                            "cpu_seconds": round(cpu, 3),
                            "last_second_cpu_percent": round(100 * (cpu - idle_start[1]) / max(.001, time.monotonic() - idle_start[0]), 2),
                        }
        raise RuntimeError(output.decode(errors="replace"))
    finally:
        if proc.poll() is None:
            proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        proc.stdout.close()


def main():
    isolate_user_state()  # never measure with the user's own settings or saved positions
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--binary", default="target/release/lenz")
    cli.add_argument("--baseline", help="Optional saved executable, alternated with --binary")
    cli.add_argument("--native", action="store_true", help="macOS only: open actual desktop windows (no synthetic input)")
    cli.add_argument("--warmups", type=int, default=0, help="Untimed launches per binary before warm measurements")
    cli.add_argument("--xvfb", default=shutil.which("Xvfb"))
    cli.add_argument("--runs", type=int, default=3)
    cli.add_argument("--seconds", type=float, default=3, help="RSS sampling duration after first paint")
    cli.add_argument("--fresh-cache", action="store_true", help="Use a new application cache for each run; OS page caches remain warm")
    cli.add_argument("args", nargs=argparse.REMAINDER, help="Arguments after --; four-file merges use disposable copies outside Git")
    opt = cli.parse_args()
    if (not opt.native and not opt.xvfb) or opt.runs < 1 or opt.warmups < 0 or not 0 < opt.seconds < 25:
        cli.error("requires Xvfb, positive --runs, and 0 < --seconds < 25")
    if opt.native and platform.system() != "Darwin":
        cli.error("--native is for macOS; Linux measurements always use a private display")
    if opt.native and opt.fresh_cache:
        cli.error("fresh application-cache isolation is only implemented for Linux")
    args = opt.args[1:] if opt.args[:1] == ["--"] else opt.args
    binaries = [("baseline", opt.baseline)] if opt.baseline else []
    binaries.append(("current", opt.binary))
    with tempfile.TemporaryDirectory(prefix="lenz-startup-") as scratch:
        args, launch_cwd = prepare_gui_launch(args, Path(scratch)/'merge-inputs')
        env = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1", LENZ_TIMINGS="1", XDG_RUNTIME_DIR=scratch)
        def launches():
            if not opt.native:
                env["XDG_CACHE_HOME"] = str(Path(scratch) / "cache-warm")
            for _, binary in binaries:
                for _ in range(opt.warmups):
                    measure(str(Path(binary).resolve()), args, env, opt.seconds, launch_cwd)
            for run in range(1, opt.runs + 1):
                # Alternate order as well as builds to reduce order/thermal bias.
                for label, binary in (binaries if run % 2 else list(reversed(binaries))):
                    if opt.fresh_cache:
                        env["XDG_CACHE_HOME"] = str(Path(scratch) / f"cache-{label}-{run}")
                    result = measure(str(Path(binary).resolve()), args, env, opt.seconds, launch_cwd)
                    print(json.dumps({"build": label, "run": run, "sample_seconds": opt.seconds,
                                      "fresh_cache": opt.fresh_cache, "display": "native-macos" if opt.native else "private-xvfb",
                                      "peak_method": "kernel-lifetime-max" if opt.native else "kernel-high-water",
                                      "merge_isolated": launch_cwd is not None,
                                      **result}), flush=True)
        if opt.native:
            launches()
            return
        env.pop("WAYLAND_DISPLAY", None)
        read_fd, write_fd = os.pipe()
        try:
            with open(Path(scratch) / "xvfb.log", "w+") as log, subprocess.Popen(
                [opt.xvfb, "-displayfd", str(write_fd), "-screen", "0", "1600x1000x24", "-nolisten", "tcp", "-noreset"],
                pass_fds=(write_fd,), stdout=log, stderr=log, env=env,
            ) as server:
                os.close(write_fd)
                write_fd = None
                try:
                    with selectors.DefaultSelector() as events:
                        events.register(read_fd, selectors.EVENT_READ)
                        if not events.select(10):
                            raise RuntimeError("Xvfb did not start within 10 seconds")
                    display = os.read(read_fd, 100).decode().strip()
                    if not display.isdigit():
                        raise RuntimeError("Xvfb failed to allocate a private display")
                    env["DISPLAY"] = ":" + display
                    launches()
                finally:
                    server.terminate()
                    try:
                        server.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait()
        finally:
            os.close(read_fd)
            if write_fd is not None:
                os.close(write_fd)


if __name__ == "__main__":
    main()
