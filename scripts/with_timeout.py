#!/usr/bin/env python3
"""Run a command with a cross-platform wall-clock timeout.

Usage:
    python scripts/with_timeout.py 120 -- cargo test

The wrapper avoids relying on POSIX `timeout`, which is not consistently available
from mise on Windows. It terminates the whole process tree where possible.
"""

from __future__ import annotations

import os
import shutil
import signal
import subprocess
import sys
from collections.abc import Sequence


def usage() -> int:
    print("usage: with_timeout.py SECONDS -- COMMAND [ARGS...]", file=sys.stderr)
    return 2


def parse_argv(argv: list[str]) -> tuple[float, list[str]]:
    if len(argv) < 4:
        raise ValueError("missing arguments")
    try:
        seconds = float(argv[1])
    except ValueError as exc:
        raise ValueError("SECONDS must be a number") from exc
    if seconds <= 0:
        raise ValueError("SECONDS must be positive")
    try:
        separator = argv.index("--", 2)
    except ValueError as exc:
        raise ValueError("missing -- before command") from exc
    command = argv[separator + 1 :]
    if not command:
        raise ValueError("missing command")
    return seconds, command


def resolve_command(command: Sequence[str]) -> list[str]:
    resolved = shutil.which(command[0])
    if resolved is None:
        return list(command)
    return [resolved, *command[1:]]


def start_process(command: Sequence[str]) -> subprocess.Popen[bytes]:
    resolved_command = resolve_command(command)
    if os.name == "nt":
        # CREATE_NEW_PROCESS_GROUP lets CTRL_BREAK reach console children.
        return subprocess.Popen(resolved_command, creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
    return subprocess.Popen(resolved_command, start_new_session=True)


def terminate_process_tree(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    if os.name == "nt":
        try:
            process.send_signal(signal.CTRL_BREAK_EVENT)
            process.wait(timeout=5)
            return
        except Exception:
            pass
        try:
            subprocess.run(
                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            return
        except Exception:
            pass
    else:
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
            return
        except Exception:
            pass
        try:
            os.killpg(process.pid, signal.SIGKILL)
            return
        except Exception:
            pass
    try:
        process.kill()
    except Exception:
        pass


def main() -> int:
    try:
        seconds, command = parse_argv(sys.argv)
    except ValueError as error:
        print(f"with_timeout.py: {error}", file=sys.stderr)
        return usage()

    process = start_process(command)
    try:
        return process.wait(timeout=seconds)
    except subprocess.TimeoutExpired:
        print(f"timeout after {seconds:g}s: {' '.join(command)}", file=sys.stderr)
        terminate_process_tree(process)
        return 124


if __name__ == "__main__":
    raise SystemExit(main())
