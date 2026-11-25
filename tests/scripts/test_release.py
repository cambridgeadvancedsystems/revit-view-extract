"""Exercise release archives and the tag/host policy at their real boundaries."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zipfile

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[2]
RUNS = ROOT / "runs" / "script-tests"
RUNS.mkdir(parents=True, exist_ok=True)


def load_script(name):
    specification = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


packager = load_script("package")
release = load_script("check-release")
builder = load_script("build-release")


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=RUNS)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for directory in ["assets", "skills", "docs", "third_party", ".claude-plugin", "views", "bin"]:
            (self.root / directory).mkdir()
        (self.root / "plugin.json").write_text(json.dumps({"name": "revit-view-extract", "version": "0.1.1"}))
        for name in [".claude-plugin/plugin.json", "README.md", "LICENSE", "NOTICE", "views/viewer.html", "views/inspector.html", "third_party/rust-NOTICES.txt", "third_party/rust-dependencies.json"]:
            (self.root / name).write_text(name)
        (self.root / "assets/licenses").mkdir()
        (self.root / "assets/licenses/three-MIT.txt").write_text("MIT notice")
        self.binary = self.root / "bin/revit-view-extract"
        self.binary.write_bytes(b"native executable fixture")
        self.output = self.root / "output"
        self.patch = mock.patch.object(packager, "ROOT", self.root)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def build(self, target="aarch64-apple-darwin"):
        return packager.package(self.binary, target, self.output)

    def test_archive_preserves_recursive_notices_modes_and_checksum(self):
        archive = self.build()
        first = archive.read_bytes()
        with zipfile.ZipFile(archive) as zipped:
            self.assertIsNone(zipped.testzip())
            names = zipped.namelist()
            self.assertIn("revit-view-extract/third_party/rust-NOTICES.txt", names)
            self.assertIn("revit-view-extract/third_party/rust-dependencies.json", names)
            self.assertIn("revit-view-extract/assets/licenses/three-MIT.txt", names)
            executable = zipped.getinfo("revit-view-extract/bin/revit-view-extract")
            self.assertEqual(executable.external_attr >> 16 & 0o777, 0o755)
            self.assertEqual(zipped.read(executable), self.binary.read_bytes())
        digest = hashlib.sha256(first).hexdigest()
        self.assertEqual(archive.with_suffix(".zip.sha256").read_text(), f"{digest}  {archive.name}\n")
        self.assertEqual(self.build().read_bytes(), first)
        windows = self.build("x86_64-pc-windows-msvc")
        with zipfile.ZipFile(windows) as zipped:
            self.assertIn("revit-view-extract/bin/revit-view-extract.exe", zipped.namelist())

    def test_failed_compression_never_publishes_partial_archive(self):
        with mock.patch.object(packager.shutil, "copyfileobj", side_effect=OSError("Disk full")):
            with self.assertRaisesRegex(OSError, "Disk full"):
                self.build()
        self.assertEqual(list(self.output.iterdir()), [])

    def test_failed_rebuild_keeps_existing_archive_and_checksum(self):
        archive = self.build()
        checksum = archive.with_suffix(".zip.sha256")
        previous = (archive.read_bytes(), checksum.read_bytes())
        with mock.patch.object(packager.shutil, "copyfileobj", side_effect=OSError("Disk full")):
            with self.assertRaises(OSError):
                self.build()
        self.assertEqual((archive.read_bytes(), checksum.read_bytes()), previous)

    def test_failed_second_promotion_restores_both_existing_outputs(self):
        archive = self.build()
        checksum = archive.with_suffix(".zip.sha256")
        previous = (archive.read_bytes(), checksum.read_bytes())
        self.binary.write_bytes(b"revised executable")
        original = packager.os.replace
        failed = False

        def replace(source, destination):
            nonlocal failed
            if Path(destination) == checksum and not failed:
                failed = True
                raise OSError("Promotion failed")
            return original(source, destination)

        with mock.patch.object(packager.os, "replace", side_effect=replace):
            with self.assertRaisesRegex(OSError, "Promotion failed"):
                self.build()
        self.assertEqual((archive.read_bytes(), checksum.read_bytes()), previous)
        self.assertEqual(sorted(path.name for path in self.output.iterdir()), sorted([archive.name, checksum.name]))

    @unittest.skipIf(__import__("os").name == "nt", "Windows symlinks require elevated runner privileges")
    def test_symlinked_file_directory_and_binary_are_rejected(self):
        source = self.root / "private.txt"
        source.write_text("Must not be packaged")
        for name, destination in [("docs/file", source), ("docs/folder", self.root / "bin")]:
            link = self.root / name
            link.symlink_to(destination, target_is_directory=destination.is_dir())
            with self.assertRaisesRegex(ValueError, "symlink"):
                self.build()
            link.unlink()
        linked_binary = self.root / "linked-bin"
        linked_binary.symlink_to(self.root / "bin", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            packager.package(linked_binary / self.binary.name, "aarch64-apple-darwin", self.output)
        linked_output = self.root / "linked-output"
        linked_output.symlink_to(self.root / "bin", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            packager.package(self.binary, "aarch64-apple-darwin", linked_output / "releases")
        self.assertFalse((self.root / "bin/releases").exists())
        self.assertFalse(list(self.output.glob("*.zip")))

    def test_missing_required_input_and_unsafe_archive_root_fail_closed(self):
        (self.root / "LICENSE").unlink()
        with self.assertRaisesRegex(ValueError, "Missing package input"):
            self.build()
        (self.root / "LICENSE").write_text("license")
        (self.root / "plugin.json").write_text(json.dumps({"name": "../escape", "version": "0.1.1"}))
        with self.assertRaisesRegex(ValueError, "Unsafe package"):
            self.build()
        self.assertFalse(list(self.output.glob("*.zip")))


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=RUNS)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        subprocess.run(["git", "clone", "--shared", "--no-checkout", "--quiet", str(ROOT), str(self.root)], check=True)
        for name in ["plugin.json", ".claude-plugin/plugin.json", "package.json", "package-lock.json", "Cargo.toml", "Cargo.lock"]:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        self.tag = "v" + json.loads((self.root / "plugin.json").read_text())["version"]

    def git(self, *arguments):
        return subprocess.check_output(["git", *arguments], cwd=self.root, text=True).strip()

    def test_nonpublishing_build_has_immutable_commit_output(self):
        values = release.validate(self.root, {})
        self.assertEqual(values["commit"], self.git("rev-parse", "HEAD"))
        self.assertEqual(values["tag"], self.tag)

    def test_publication_needs_the_exact_existing_tag_at_head(self):
        with self.assertRaisesRegex(SystemExit, "exact release tag"):
            release.validate(self.root, {"REVIT_RELEASE_PUBLISH": "true"})
        environment = {"REVIT_RELEASE_PUBLISH": "true", "REVIT_RELEASE_TAG": self.tag}
        self.git("update-ref", "-d", f"refs/tags/{self.tag}")
        with self.assertRaisesRegex(SystemExit, "must already exist"):
            release.validate(self.root, environment)
        self.git("update-ref", f"refs/tags/{self.tag}", self.git("rev-parse", "HEAD"))
        original = subprocess.check_output

        def different_checkout(arguments, **keywords):
            if arguments == ["git", "rev-parse", "HEAD"]:
                return "0" * 40
            return original(arguments, **keywords)

        with mock.patch.object(release.subprocess, "check_output", side_effect=different_checkout):
            with self.assertRaisesRegex(SystemExit, "exact release tag commit"):
                release.validate(self.root, environment)
        self.assertEqual(release.validate(self.root, environment)["commit"], self.git("rev-parse", "HEAD"))
        with self.assertRaisesRegex(SystemExit, "exact release tag"):
            release.validate(self.root, {"REVIT_RELEASE_TAG": "main"})

    def test_native_host_and_every_lockfile_version_are_checked(self):
        host = next(line.removeprefix("host: ") for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
        release.validate(self.root, {"REVIT_RELEASE_TARGET": host})
        with self.assertRaisesRegex(SystemExit, "Expected native Rust host"):
            release.validate(self.root, {"REVIT_RELEASE_TARGET": "incompatible-host"})
        filename = self.root / "package-lock.json"
        value = json.loads(filename.read_text())
        value["packages"][""]["version"] = "999.0.0"
        filename.write_text(json.dumps(value))
        with self.assertRaisesRegex(SystemExit, "versions must agree"):
            release.validate(self.root, {})


class ReleaseBuildTests(unittest.TestCase):
    def test_real_rustc_remaps_workspace_paths_with_spaces_and_preserves_encoded_flags(self):
        with tempfile.TemporaryDirectory(prefix="private workspace ", dir=RUNS) as folder:
            workspace = Path(folder)
            source = workspace / "main.rs"
            source.write_text('#[cfg(custom_release)] fn main() { println!("{}", file!()); }\n')
            original = {**os.environ, "CARGO_ENCODED_RUSTFLAGS": "--cfg=custom_release", "RUSTFLAGS": "--invalid-unused-fallback"}
            with mock.patch.object(builder, "ROOT", workspace):
                environment = builder.release_environment(original)
            self.assertEqual(original["CARGO_ENCODED_RUSTFLAGS"], "--cfg=custom_release")
            executable = workspace / ("probe.exe" if os.name == "nt" else "probe")
            flags = environment["CARGO_ENCODED_RUSTFLAGS"].split("\x1f")
            subprocess.run(["rustc", str(source), "-o", str(executable), *flags], check=True)
            output = subprocess.check_output([str(executable)], text=True).strip().replace("\\", "/")
            self.assertEqual(output, "src/main.rs")


if __name__ == "__main__":
    unittest.main()
