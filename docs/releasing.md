# Building and publishing releases

The release workflow builds native plugin ZIPs and SHA-256 files on five GitHub-hosted runners. Tag pushes and ordinary manual runs produce downloadable workflow artifacts. A manual run with `publish: true` and an exact existing version tag also publishes the verified assets as a GitHub release.

| Backend | Runner | Target |
| --- | --- | --- |
| macOS Apple Silicon | `macos-15` | `aarch64-apple-darwin` |
| macOS Intel | `macos-15-intel` | `x86_64-apple-darwin` |
| Linux x86-64 / glibc | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` |
| Linux ARM64 / glibc | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` |
| Windows x86-64 | `windows-2022` | `x86_64-pc-windows-msvc` |

These architectures follow the [GitHub runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). Each job verifies its Rust host triple and installs the declared target. Linux binaries are built on Ubuntu 24.04; older glibc distributions may require a source build. macOS binaries are not notarised.

## Build locally

```sh
npm ci
npm run check
python3 -m unittest discover -s tests/scripts
cargo test --locked --all-targets
python3 scripts/build-release.py
python3 scripts/package.py --binary target/release/revit-view-extract --target aarch64-apple-darwin
```

Use the target matching the compiled binary. Windows uses `python` and `revit-view-extract.exe`. The release build remaps home, dependency and workspace paths to portable prefixes. Packaging preserves Unix executable permissions and emits archives under gitignored `runs/releases/`. It includes the plugin, bundled views, skill, user guides and required third-party notices; source models, extracted data, caches and credentials are excluded.

## Publish

- [ ] Set matching versions in Cargo.toml, plugin.json, the compatibility manifest and package metadata.
- [ ] Run checks and commit regenerated views with the release source.
- [ ] Create and push the exact version tag, such as `v0.1.0`.
- [ ] Run **Build platform plugin archives** for that tag with `publish: true`.

Publication requires all five native jobs to pass, the checkout to match the tag, and all expected archives and checksums to verify. Existing releases are not overwritten. The workflow does not create repositories, push branches/tags or update the Marketplace.

The official Marketplace uses a local source snapshot under `plugins/revit-view-extract`, preserving the released source, manifests, icon, views, skill and licenses. This makes the icon visible before installation and leaves Cargo source available for the first build. Its catalog policy is `AVAILABLE` with authentication `NONE` and execution `local`.
