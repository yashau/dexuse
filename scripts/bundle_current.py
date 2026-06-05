import platform
import shutil
import sys
from pathlib import Path


def npm_platform() -> str:
    if sys.platform.startswith("win"):
        return "win32"
    if sys.platform == "darwin":
        return "darwin"
    if sys.platform.startswith("linux"):
        return "linux"
    return sys.platform


def npm_arch() -> str:
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return "x64"
    if machine in {"aarch64", "arm64"}:
        return "arm64"
    if machine in {"i386", "i686", "x86"}:
        return "ia32"
    return machine


platform_name = npm_platform()
arch = npm_arch()
exe = ".exe" if platform_name == "win32" else ""
source = Path("target") / "release" / f"dexuse{exe}"
dest = Path("bin") / f"dexuse-{platform_name}-{arch}{exe}"

if not source.exists():
    raise SystemExit(f"missing release binary: {source}; run `cargo build --release` first")

Path("bin").mkdir(exist_ok=True)
shutil.copy2(source, dest)
print(f"copied {source} -> {dest}")
