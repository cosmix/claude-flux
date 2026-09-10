//! Bounded, dirfd-relative reads that reject symlinks at every path component.

use anyhow::{bail, Context, Result};
use std::fs::{File, Metadata};
use std::io;
use std::io::{Read, Take};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::safe_fs::{open_safely, safe_open_dirfd};

/// Whether any cause in `error`'s chain is `ENOENT`: an absent file, an
/// absent parent directory, or an absent dirfd root. Shared by every no-follow
/// open that treats a missing path as `Ok(None)` rather than an error.
pub(crate) fn is_not_found(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io_error| io_error.kind() == io::ErrorKind::NotFound)
    })
}

/// fstat an already-opened file and refuse anything that is not a regular
/// file. Shared by `read_bounded` and `open_regular_no_follow`, which both
/// resolve through a no-follow dirfd walk and need the same check on the fd
/// they get back.
fn regular_file_metadata(file: &File) -> Result<Metadata> {
    let metadata = file.metadata().context("Failed to inspect opened file")?;
    if !metadata.is_file() {
        bail!("opened path is not a regular file");
    }
    Ok(metadata)
}

/// Read at most `max_bytes` from a regular file beneath `root`.
///
/// `relative` must not be absolute or contain `..`. The kernel opens every
/// component with no-follow semantics, so an attacker cannot substitute a
/// symlink between validation and use.
pub fn read_bounded(root: &Path, relative: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    let root_fd = safe_open_dirfd(root)?;
    let file_fd = open_safely(root_fd.as_raw_fd(), relative, libc::O_RDONLY, 0)
        .with_context(|| format!("Refusing unsafe read of {}", relative.display()))?;
    let file = File::from(file_fd);
    let metadata = regular_file_metadata(&file)
        .with_context(|| format!("Refusing unsafe read of {}", relative.display()))?;
    if metadata.len() > max_bytes as u64 {
        bail!(
            "{} exceeds the {} byte verification limit",
            relative.display(),
            max_bytes
        );
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut reader: Take<File> = file.take(limit);
    reader
        .read_to_end(&mut bytes)
        .context("Failed to read opened file")?;
    if bytes.len() > max_bytes {
        bail!(
            "{} grew beyond the {} byte verification limit while reading",
            relative.display(),
            max_bytes
        );
    }
    Ok(bytes)
}

/// Read a bounded UTF-8 file beneath `root` without following symlinks.
pub fn read_to_string_bounded(root: &Path, relative: &Path, max_bytes: usize) -> Result<String> {
    let bytes = read_bounded(root, relative, max_bytes)?;
    String::from_utf8(bytes).with_context(|| format!("{} is not valid UTF-8", relative.display()))
}

/// Open `relpath` beneath `root` without following a symlink at any path
/// component, refusing anything that is not a regular file with a single
/// link. Shared by the memory, telemetry, and stage-request spools: each
/// sits inside a sandboxed session's write boundary, so the session can swap
/// the spool or its parent directory for a symlink, a hard link, or a FIFO -
/// following any of those would let it redirect the trusted daemon's read or
/// truncate outside the worktree. `O_NONBLOCK` keeps a FIFO from blocking the
/// open before the regular-file check runs, and has no effect on a regular
/// file.
///
/// `Ok(None)` when `root`, an intermediate directory, or `relpath` itself is
/// absent - the common case on every daemon poll tick - so callers need no
/// separate existence check. Never creates anything.
pub(crate) fn open_regular_no_follow(
    root: &Path,
    relpath: &str,
    flags: i32,
) -> Result<Option<File>> {
    let opened = safe_open_dirfd(root).and_then(|root_fd| {
        open_safely(
            root_fd.as_raw_fd(),
            Path::new(relpath),
            flags | libc::O_NONBLOCK,
            0,
        )
    });
    let file = match opened {
        Ok(descriptor) => File::from(descriptor),
        Err(error) if is_not_found(&error) => return Ok(None),
        Err(error) => return Err(error.context("no-follow open failed")),
    };
    let metadata = regular_file_metadata(&file)?;
    if metadata.nlink() != 1 {
        bail!("opened path is a hard link with more than one name");
    }
    Ok(Some(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_leaf_symlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();

        assert!(read_bounded(root.path(), Path::new("link"), 1024).is_err());
    }

    #[test]
    fn rejects_intermediate_symlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("dir")).unwrap();

        assert!(read_bounded(root.path(), Path::new("dir/secret"), 1024).is_err());
    }
}
