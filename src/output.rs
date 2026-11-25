//! Path collision checks and atomic publication shared by every output format.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

fn normalized(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => {
                resolved.push(other.as_os_str());
                if resolved.exists() {
                    resolved = resolved.canonicalize().context("Resolve output path")?;
                }
            }
        }
    }
    Ok(resolved)
}

fn collision_key(path: &Path) -> String {
    let key = path.to_string_lossy().into_owned();
    // Conservative on systems commonly configured with case-insensitive volumes.
    // Atomic replacement still protects distinct hard-link aliases on Windows.
    if cfg!(any(windows, target_os = "macos")) {
        key.to_lowercase()
    } else {
        key
    }
}

/// Inputs are protected before creating any output, including non-existing aliases.
pub fn validate(inputs: &[&Path], outputs: &[&Path]) -> Result<()> {
    let inputs = inputs
        .iter()
        .map(|p| normalized(p))
        .collect::<Result<Vec<_>>>()?;
    let input_keys: BTreeSet<_> = inputs.iter().map(|p| collision_key(p)).collect();
    let mut seen = BTreeSet::new();
    for output in outputs {
        let path = normalized(output)?;
        ensure!(
            !path.is_dir(),
            "An output path names a directory: {}",
            output.display()
        );
        ensure!(
            !input_keys.contains(&collision_key(&path)),
            "An output path aliases an input file: {}",
            output.display()
        );
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::metadata(output) {
            use std::os::unix::fs::MetadataExt;
            for source in &inputs {
                let source = std::fs::metadata(source)?;
                ensure!(
                    metadata.dev() != source.dev() || metadata.ino() != source.ino(),
                    "An output path is a hard link to an input file"
                );
            }
        }
        ensure!(
            seen.insert(collision_key(&path)),
            "Output paths must be distinct: {}",
            output.display()
        );
    }
    Ok(())
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn temporary_file(parent: &Path) -> Result<(Temporary, File)> {
    for _ in 0..1024 {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".revit-view-extract.{}.{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((Temporary(path), file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("Create temporary output"),
        }
    }
    anyhow::bail!("Cannot allocate a unique temporary output")
}

/// Publish only after serialization, flush and sync succeed. The temporary file
/// shares the destination directory, so rename never falls back to a copy.
/// std::fs::rename replaces existing files on Unix and Windows; a sharing or ACL
/// failure leaves the old destination intact. All error paths clean up our temp.
pub fn write_atomic(path: &Path, write: impl FnOnce(&mut dyn Write) -> Result<()>) -> Result<()> {
    path.file_name().context("Output has no filename")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let (temporary, file) = temporary_file(parent)?;
    {
        let mut writer = BufWriter::new(file);
        write(&mut writer)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
    }
    std::fs::rename(&temporary.0, path).context("Publish completed output")?;
    Ok(())
}

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    write_atomic(path, |writer| {
        serde_json::to_writer(writer, value)?;
        Ok(())
    })
}
