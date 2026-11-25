#!/usr/bin/env python3
"""Build the native release without embedding private machine paths."""
import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def release_environment(environment):
    result = dict(environment)
    encoded = result.get("CARGO_ENCODED_RUSTFLAGS")
    flags = encoded.split("\x1f") if encoded else result.get("RUSTFLAGS", "").split()
    home = Path.home()
    # rustc uses the last matching prefix, so the workspace must come last.
    prefixes = [
        (home, "local"),
        (Path(result.get("CARGO_HOME", home / ".cargo")), "cargo"),
        (Path(result.get("RUSTUP_HOME", home / ".rustup")), "rustup"),
        (ROOT, "src"),
    ]
    flags.extend(f"--remap-path-prefix={source}={destination}" for source, destination in prefixes)
    result["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", help="Optional Rust target triple; use the native host for release archives")
    arguments = parser.parse_args()
    command = ["cargo", "build", "--locked", "--release"]
    if arguments.target:
        command.extend(["--target", arguments.target])
    subprocess.run(command, cwd=ROOT, env=release_environment(os.environ), check=True)
