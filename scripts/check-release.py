#!/usr/bin/env python3
"""Validate manifest agreement and the exact version/tag before an optional release."""
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

def validate(root, environment):
    manifest = json.loads((root / "plugin.json").read_text())
    version = manifest["version"]
    package_lock = json.loads((root / "package-lock.json").read_text())
    cargo_lock = tomllib.loads((root / "Cargo.lock").read_text())
    versions = [
        json.loads((root / ".claude-plugin/plugin.json").read_text())["version"],
        json.loads((root / "package.json").read_text())["version"],
        package_lock["version"],
        package_lock["packages"][""]["version"],
        tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"],
        next(package["version"] for package in cargo_lock["package"] if package["name"] == manifest["name"]),
    ]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version) or any(value != version for value in versions):
        raise SystemExit("Package versions must agree and use a numeric major.minor.patch version")
    target = environment.get("REVIT_RELEASE_TARGET")
    if target:
        rust_version = subprocess.check_output(["rustc", "-vV"], text=True)
        host = next(line.removeprefix("host: ") for line in rust_version.splitlines() if line.startswith("host: "))
        if host != target:
            raise SystemExit(f"Expected native Rust host {target}; runner supplied {host}")
    tag = f"v{version}"
    requested = environment.get("REVIT_RELEASE_TAG", "")
    publish = environment.get("REVIT_RELEASE_PUBLISH", "false").lower() == "true"
    tag_build = environment.get("REVIT_RELEASE_REF_TYPE") == "tag"
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    if requested or publish or tag_build:
        if requested != tag:
            raise SystemExit(f"Expected exact release tag {tag}")
        try:
            tagged = subprocess.check_output(["git", "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}"], cwd=root, text=True, stderr=subprocess.PIPE).strip()
        except subprocess.CalledProcessError:
            raise SystemExit(f"Release tag {tag} must already exist") from None
        if tagged != head:
            raise SystemExit("Build checkout must be the exact release tag commit")
    return {"version": version, "tag": tag, "commit": head}


if __name__ == "__main__":
    values = validate(Path(__file__).resolve().parent.parent, os.environ)
    output = "".join(f"{key}={value}\n" for key, value in values.items())
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as destination:
            destination.write(output)
    print(output, end="")
