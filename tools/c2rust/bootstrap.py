#!/usr/bin/env python3
"""Build the project's C2Rust with its recursive-type traversal fix under target/."""
import fcntl
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
BUILD = HERE.parents[1] / "target/c2rust"
UPSTREAM = "https://github.com/immunant/c2rust.git"
TAG = "v0.22.1"
REVISION = "e9ccd715597f7d63e8635629f4450e040a230118"


def compiler_environment():
    env = os.environ.copy()
    candidates = [env.get("LLVM_CONFIG_PATH"), shutil.which("llvm-config-21")]
    brew = shutil.which("brew")
    if brew:
        prefix = subprocess.check_output([brew, "--prefix", "llvm@21"], text=True).strip()
        candidates.append(str(Path(prefix) / "bin/llvm-config"))
    candidates.append(shutil.which("llvm-config"))
    config = next((shutil.which(item) for item in candidates if item and shutil.which(item)), None)
    if not config:
        raise SystemExit("Install LLVM and Clang development files, or set LLVM_CONFIG_PATH to llvm-config.")
    env["LLVM_CONFIG_PATH"] = config
    env.setdefault("LIBCLANG_PATH", subprocess.check_output([config, "--libdir"], text=True).strip())
    # The old toolchain file is for C2Rust's refactoring tools. The transpiler
    # supports stable Rust, which also builds the desktop runtime.
    env.setdefault("RUSTUP_TOOLCHAIN", "stable")
    return env


def main():
    patch = HERE / "recursive-types.patch"
    patch_id = hashlib.sha256(patch.read_bytes()).hexdigest()[:12]
    source = BUILD / f"source-{REVISION[:12]}-{patch_id}"
    binary = BUILD / "bin/c2rust-transpile"
    binary.parent.mkdir(parents=True, exist_ok=True)
    with (BUILD / "bootstrap.lock").open("a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if not source.is_dir():
            with tempfile.TemporaryDirectory(dir=BUILD) as stage:
                checkout = Path(stage) / "source"
                subprocess.run(["git", "clone", "--depth=1", "--branch", TAG, UPSTREAM, checkout], check=True)
                commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
                if commit != REVISION:
                    raise SystemExit(f"Expected C2Rust {REVISION}, got {commit}")
                subprocess.run(["git", "apply", patch], cwd=checkout, check=True)
                checkout.rename(source)
        subprocess.run(["cargo", "build", "--locked", "--release", "--package", "c2rust",
                        "--bin", "c2rust-transpile", "--target-dir", BUILD / "cargo"],
                       cwd=source, env=compiler_environment(), check=True)
        with tempfile.TemporaryDirectory(dir=binary.parent) as stage:
            executable = Path(stage) / binary.name
            shutil.copy2(BUILD / "cargo/release/c2rust-transpile", executable)
            os.replace(executable, binary)
    print(binary)


if __name__ == "__main__":
    main()
