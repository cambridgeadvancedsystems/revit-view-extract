#!/usr/bin/env python3
"""Build an installable platform ZIP; no source models or working outputs enter it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
}


def linked_path(path):
    return any(parent.is_symlink() or parent.is_junction() for parent in [path, *path.parents])


def regular_file(path):
    path = path.absolute()
    if linked_path(path):
        raise ValueError("Package inputs must not contain symlinks")
    if not path.is_file():
        raise ValueError(f"Missing package input: {path.name}")
    return path


def replace_outputs(staged, output):
    """Publish complete files, restoring earlier outputs if promotion fails."""
    replaced = []
    try:
        for source in staged:
            destination = output / source.name
            backup = source.with_name(source.name + ".previous")
            if destination.exists() or destination.is_symlink():
                if not destination.is_file() or destination.is_symlink():
                    raise ValueError("Release outputs must be regular files")
                shutil.copyfile(destination, backup)
            os.replace(source, destination)
            replaced.append((destination, backup))
    except BaseException:
        for destination, backup in reversed(replaced):
            if backup.exists():
                os.replace(backup, destination)
            else:
                destination.unlink()
        raise


def package(binary, target, output):
    if target not in TARGETS:
        raise ValueError("Unsupported target triple")
    binary = regular_file(binary)
    manifest = json.loads(regular_file(ROOT / "plugin.json").read_text())
    version = manifest["version"]
    archive_root = manifest["name"]
    if not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", archive_root) or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Unsafe package name or version")
    output = output.absolute()
    if linked_path(output):
        raise ValueError("Release output directories must not contain symlinks")
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{archive_root}-{version}-{target}.zip"
    sources = [ROOT / "plugin.json", ROOT / ".claude-plugin/plugin.json", ROOT / "README.md", ROOT / "LICENSE", ROOT / "NOTICE"]
    for directory in ["assets", "skills", "docs", "third_party"]:
        directory_root = ROOT / directory
        if linked_path(directory_root) or not directory_root.is_dir():
            raise ValueError(f"Missing or symlinked package directory: {directory}")
        for path in directory_root.rglob("*"):
            if path.is_symlink() or path.is_junction():
                raise ValueError("Package inputs must not contain symlinks")
            if path.is_file():
                sources.append(path)
    sources.extend([ROOT / "views/viewer.html", ROOT / "views/inspector.html"])
    executable = "revit-view-extract.exe" if "windows" in target else "revit-view-extract"
    entries = [(regular_file(path), f"{archive_root}/{path.relative_to(ROOT).as_posix()}", 0o644) for path in sources]
    entries.append((binary, f"{archive_root}/bin/{executable}", 0o755))
    with tempfile.TemporaryDirectory(prefix=".package-", dir=output) as staging:
        staged_archive = Path(staging) / archive.name
        with zipfile.ZipFile(staged_archive, "w", compression=zipfile.ZIP_DEFLATED) as destination:
            for path, name, mode in sorted(entries, key=lambda entry: entry[1]):
                info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = (stat.S_IFREG | mode) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                with regular_file(path).open("rb") as source, destination.open(info, "w") as entry:
                    shutil.copyfileobj(source, entry)
        with staged_archive.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        checksum = staged_archive.with_suffix(staged_archive.suffix + ".sha256")
        checksum.write_text(f"{digest}  {archive.name}\n")
        replace_outputs([staged_archive, checksum], output)
    return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--target", required=True, choices=sorted(TARGETS))
    parser.add_argument("--out", type=Path, default=ROOT / "runs/releases")
    arguments = parser.parse_args()
    print(package(arguments.binary.absolute(), arguments.target, arguments.out.absolute()))
