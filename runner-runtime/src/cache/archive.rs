use anyhow::Context as _;
use scope_cache_domain::MAX_CACHE_OBJECT_BYTES;
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt as _,
    path::Path,
};

pub(super) fn reset_cache_directory(path: &Path) -> anyhow::Result<()> {
    fs::remove_dir_all(path)
        .with_context(|| format!("remove partial cache directory {}", path.display()))?;
    fs::create_dir_all(path).with_context(|| format!("recreate cache directory {}", path.display()))
}

pub(super) fn extract_archive(archive: &Path, destination: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(archive)?;
    let decoder = zstd::Decoder::new(file).context("open compressed cache")?;
    tar::Archive::new(decoder)
        .unpack(destination)
        .context("extract cache archive")
}

pub(super) fn create_archive(source: &Path, destination: &Path) -> anyhow::Result<(u64, String)> {
    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(destination)?;
    let bounded = BoundedWriter::new(file, MAX_CACHE_OBJECT_BYTES);
    let encoder = zstd::Encoder::new(bounded, 3).context("create compressed cache")?;
    let mut archive = tar::Builder::new(encoder);
    append_directory_contents(&mut archive, source, Path::new(""))?;
    let encoder = archive.into_inner()?;
    let writer = encoder.finish()?;
    Ok(writer.identity())
}

fn append_directory_contents<W: Write>(
    archive: &mut tar::Builder<W>,
    source: &Path,
    relative: &Path,
) -> anyhow::Result<()> {
    let directory = source.join(relative);
    let mut entries = fs::read_dir(&directory)
        .with_context(|| format!("read cache directory {}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let relative_path = relative.join(entry.file_name());
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read cache entry metadata {}", path.display()))?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            let mut header = normalized_header(tar::EntryType::Directory, 0, 0o755)?;
            append_entry(archive, &mut header, &relative_path, std::io::empty())?;
            append_directory_contents(archive, source, &relative_path)?;
        } else if file_type.is_file() {
            let mode = if metadata.permissions().mode() & 0o111 == 0 {
                0o644
            } else {
                0o755
            };
            let file = fs::File::open(&path)
                .with_context(|| format!("open cache entry {}", path.display()))?;
            let mut header = normalized_header(tar::EntryType::Regular, metadata.len(), mode)?;
            append_entry(archive, &mut header, &relative_path, file)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&path)
                .with_context(|| format!("read cache symlink {}", path.display()))?;
            let mut header = normalized_header(tar::EntryType::Symlink, 0, 0o777)?;
            archive
                .append_link(&mut header, &relative_path, target)
                .with_context(|| format!("archive cache symlink {}", relative_path.display()))?;
        } else {
            anyhow::bail!(
                "cache entry {} has an unsupported file type",
                path.display()
            );
        }
    }
    Ok(())
}

fn normalized_header(
    entry_type: tar::EntryType,
    size: u64,
    mode: u32,
) -> anyhow::Result<tar::Header> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(entry_type);
    header.set_size(size);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_username("")?;
    header.set_groupname("")?;
    header.set_cksum();
    Ok(header)
}

fn append_entry<W: Write, R: Read>(
    archive: &mut tar::Builder<W>,
    header: &mut tar::Header,
    path: &Path,
    content: R,
) -> anyhow::Result<()> {
    archive
        .append_data(header, path, content)
        .with_context(|| format!("archive cache entry {}", path.display()))
}

pub(super) struct BoundedWriter<W> {
    inner: W,
    pub(super) written: u64,
    max_bytes: u64,
    hasher: Sha256,
}

impl<W> BoundedWriter<W> {
    pub(super) fn new(inner: W, max_bytes: u64) -> Self {
        Self {
            inner,
            written: 0,
            max_bytes,
            hasher: Sha256::new(),
        }
    }

    pub(super) fn identity(self) -> (u64, String) {
        (self.written, hex::encode(self.hasher.finalize()))
    }
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.written.saturating_add(bytes.len() as u64) > self.max_bytes {
            return Err(std::io::Error::other(format!(
                "cache archive exceeds {} bytes",
                self.max_bytes
            )));
        }
        let written = self.inner.write(bytes)?;
        self.written += written as u64;
        self.hasher.update(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
