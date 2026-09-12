//! Reading an installer file: locate the footer, address the blocks, decode
//! the metadata, open the payload through a bounded window, inspect and
//! verify without executing anything.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::footer::{FOOTER_LEN, Footer, locate_footer};
use crate::payload::open_archive;
use crate::window::Window;
use crate::{FormatError, Metadata, hex, sha256, sha256_reader};

/// The payload opened as a ZIP archive over a bounded window of the file.
pub type PayloadArchive = zip::ZipArchive<Window<File>>;

/// Absolute positions of the blocks in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub file_length: u64,
    pub engine_length: u64,
    pub metadata_offset: u64,
    pub metadata_length: u64,
    pub payload_offset: u64,
    pub payload_length: u64,
    pub footer_offset: u64,
}

/// An opened installer file. The metadata is decoded and hash-checked at
/// open time; the payload is opened on demand through a window.
#[derive(Debug)]
pub struct Installer {
    path: PathBuf,
    file: File,
    footer: Footer,
    layout: Layout,
    metadata: Metadata,
    metadata_bytes: Vec<u8>,
}

/// One payload entry as `inspect` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub method: String,
    pub crc32: u32,
}

/// The result of a full verification: every check passed, or the first
/// failure per category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyOutcome {
    pub payload_sha256_ok: bool,
    pub entries_checked: usize,
    pub problems: Vec<FormatError>,
}

impl VerifyOutcome {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

impl Installer {
    /// Opens `path` with full sharing (a running executable can read itself).
    pub fn open(path: &Path) -> Result<Installer, FormatError> {
        let file = File::open(path).map_err(|err| {
            FormatError::new(
                "package_unreadable",
                format!("cannot open {}: {err}", path.display()),
            )
        })?;
        Self::from_file(path.to_path_buf(), file)
    }

    fn from_file(path: PathBuf, mut file: File) -> Result<Installer, FormatError> {
        let file_length = file.metadata()?.len();
        let mut head = vec![0u8; 4096];
        let head_len = read_up_to(&mut file, &mut head)?;
        head.truncate(head_len);
        let footer_offset = locate_footer(file_length, &head)?;

        file.seek(SeekFrom::Start(footer_offset))?;
        let mut footer_bytes = [0u8; FOOTER_LEN];
        file.read_exact(&mut footer_bytes)?;
        let footer = Footer::decode(&footer_bytes)?;
        footer.check_layout(footer_offset)?;

        let layout = Layout {
            file_length,
            engine_length: footer.metadata_offset,
            metadata_offset: footer.metadata_offset,
            metadata_length: footer.metadata_length,
            payload_offset: footer.payload_offset,
            payload_length: footer.payload_length,
            footer_offset,
        };

        let metadata_length = usize::try_from(footer.metadata_length)
            .map_err(|_| FormatError::new("footer_invalid", "metadata block too large"))?;
        let mut metadata_bytes = vec![0u8; metadata_length];
        file.seek(SeekFrom::Start(footer.metadata_offset))?;
        file.read_exact(&mut metadata_bytes)?;
        if sha256(&metadata_bytes) != footer.metadata_sha256 {
            return Err(FormatError::new(
                "metadata_hash_mismatch",
                "the metadata block does not match the hash in the footer",
            ));
        }
        let metadata = Metadata::decode_block(&metadata_bytes)?;

        Ok(Installer {
            path,
            file,
            footer,
            layout,
            metadata,
            metadata_bytes,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn footer(&self) -> &Footer {
        &self.footer
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn metadata_bytes(&self) -> &[u8] {
        &self.metadata_bytes
    }

    pub fn metadata_sha256_hex(&self) -> String {
        hex(&self.footer.metadata_sha256)
    }

    pub fn payload_sha256_hex(&self) -> String {
        hex(&self.footer.payload_sha256)
    }

    /// A fresh bounded reader over the engine block.
    pub fn engine_block(&self) -> Result<Window<File>, FormatError> {
        Ok(Window::new(
            self.file.try_clone()?,
            0,
            self.layout.engine_length,
        )?)
    }

    /// A fresh bounded reader over the payload block.
    pub fn payload_block(&self) -> Result<Window<File>, FormatError> {
        Ok(Window::new(
            self.file.try_clone()?,
            self.layout.payload_offset,
            self.layout.payload_length,
        )?)
    }

    /// The payload opened as a ZIP archive.
    pub fn payload_archive(&self) -> Result<PayloadArchive, FormatError> {
        open_archive(self.payload_block()?)
    }

    /// SHA-256 of the engine block, computed from the file.
    pub fn engine_sha256_hex(&self) -> Result<String, FormatError> {
        Ok(hex(&sha256_reader(&mut self.engine_block()?)?))
    }

    /// Lists the payload entries.
    pub fn entries(&self) -> Result<Vec<EntryInfo>, FormatError> {
        let mut archive = self.payload_archive()?;
        let mut entries = Vec::with_capacity(archive.len());
        for index in 0..archive.len() {
            let entry = archive.by_index_raw(index)?;
            entries.push(EntryInfo {
                name: entry.name().to_string(),
                size: entry.size(),
                compressed_size: entry.compressed_size(),
                method: entry.compression().to_string(),
                crc32: entry.crc32(),
            });
        }
        Ok(entries)
    }

    /// Verifies the payload hash, that every declared file has an entry of
    /// the declared size, and every entry's CRC-32 by reading it in full.
    /// (The footer CRC and the metadata hash were already checked at open.)
    pub fn verify(&self) -> Result<VerifyOutcome, FormatError> {
        let mut problems = Vec::new();
        let payload_sha256_ok =
            sha256_reader(&mut self.payload_block()?)? == self.footer.payload_sha256;
        if !payload_sha256_ok {
            problems.push(FormatError::new(
                "payload_hash_mismatch",
                "the payload block does not match the hash in the footer",
            ));
        }
        let mut archive = self.payload_archive()?;
        let mut entries_checked = 0;
        for file in &self.metadata.files {
            match archive.by_name(&file.entry) {
                Ok(mut entry) => {
                    if entry.size() != file.size {
                        problems.push(FormatError::new(
                            "payload_entry_size_mismatch",
                            format!(
                                "{}: entry is {} bytes, metadata declares {}",
                                file.path,
                                entry.size(),
                                file.size
                            ),
                        ));
                    }
                    if let Err(err) = std::io::copy(&mut entry, &mut std::io::sink()) {
                        problems.push(FormatError::new(
                            "payload_entry_crc_mismatch",
                            format!("{}: {err}", file.path),
                        ));
                    }
                    entries_checked += 1;
                }
                Err(_) => problems.push(FormatError::new(
                    "payload_entry_missing",
                    format!("{}: no payload entry {:?}", file.path, file.entry),
                )),
            }
        }
        Ok(VerifyOutcome {
            payload_sha256_ok,
            entries_checked,
            problems,
        })
    }
}

fn read_up_to<R: Read>(reader: &mut R, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let n = reader.read(&mut buffer[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{PayloadSource, compose};
    use crate::metadata::{Directory, Engine, File as MetaFile, Install, Package};
    use crate::payload::Compression;
    use std::io::Write;

    fn metadata_for(files: &[(&str, &[u8])]) -> Metadata {
        Metadata {
            schema: crate::metadata::SCHEMA,
            package: Some(Package {
                id: "IT-Tiger.Sample".into(),
                name: "Sample".into(),
                version: "1.0.0".into(),
                publisher: "IT Tiger".into(),
                ..Default::default()
            }),
            install: Some(Install {
                scopes: vec![crate::identity::Scope::User.tag()],
                user_root: "%LOCALAPPDATA%\\Programs\\Sample".into(),
                machine_root: String::new(),
                ..Default::default()
            }),
            files: files
                .iter()
                .map(|(path, bytes)| MetaFile {
                    path: path.to_string(),
                    size: bytes.len() as u64,
                    entry: path.to_string(),
                })
                .collect(),
            directories: vec![Directory { path: "bin".into() }],
            engine: Some(Engine {
                tigersetup_version: "0.1.0".into(),
                engine_sha256: hex(&sha256(b"stub engine")),
                engine_block_sha256: hex(&sha256(b"stub engine")),
            }),
            ..Default::default()
        }
    }

    fn build(dir: &Path, files: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("Sample-Setup.exe");
        let out = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let metadata = metadata_for(files);
        let sources = files.iter().map(|(path, bytes)| {
            Ok(PayloadSource {
                entry: path.to_string(),
                bytes: bytes.to_vec(),
            })
        });
        compose(
            out,
            &mut &b"stub engine"[..],
            &metadata,
            sources,
            Compression::Fast,
        )
        .unwrap();
        path
    }

    #[test]
    fn composed_installer_opens_inspects_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![7u8; 100_000];
        let path = build(
            dir.path(),
            &[("bin/app.exe", b"app bytes"), ("bin/data.bin", &big)],
        );
        let installer = Installer::open(&path).unwrap();
        assert_eq!(
            installer.layout().engine_length,
            b"stub engine".len() as u64
        );
        assert_eq!(installer.metadata().package().id, "IT-Tiger.Sample");
        assert_eq!(
            installer.engine_sha256_hex().unwrap(),
            hex(&sha256(b"stub engine"))
        );
        let entries = installer.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].name, "bin/data.bin");
        assert_eq!(entries[1].size, 100_000);
        let outcome = installer.verify().unwrap();
        assert!(outcome.is_ok(), "{:?}", outcome.problems);
        assert_eq!(outcome.entries_checked, 2);
        let mut archive = installer.payload_archive().unwrap();
        let mut out = Vec::new();
        crate::payload::copy_entry(&mut archive, "bin/app.exe", &mut out).unwrap();
        assert_eq!(out, b"app bytes");
    }

    #[test]
    fn payload_corruption_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let noise: Vec<u8> = (0..50_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 11) as u8)
            .collect();
        let path = build(dir.path(), &[("bin/noise.bin", &noise)]);
        let installer = Installer::open(&path).unwrap();
        let offset = installer.layout().payload_offset + 100;
        drop(installer);
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(&[0xAA; 16]).unwrap();
        drop(file);
        let installer = Installer::open(&path).unwrap();
        let outcome = installer.verify().unwrap();
        assert!(!outcome.payload_sha256_ok);
        assert!(
            outcome
                .problems
                .iter()
                .any(|p| p.code == "payload_entry_crc_mismatch"),
            "{:?}",
            outcome.problems
        );
    }

    #[test]
    fn metadata_corruption_is_detected_at_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = build(dir.path(), &[("bin/app.exe", b"app bytes")]);
        let offset = Installer::open(&path).unwrap().layout().metadata_offset;
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(offset + 2)).unwrap();
        file.write_all(&[0xFF]).unwrap();
        drop(file);
        assert_eq!(
            Installer::open(&path).unwrap_err().code,
            "metadata_hash_mismatch"
        );
    }

    /// The uninstaller copy the engine writes into the state directory is a
    /// complete installer file with an empty payload.
    #[test]
    fn a_package_with_no_payload_composes_opens_and_verifies() {
        use crate::identity::Scope;
        use crate::metadata::Role;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uninstall.exe");
        let out = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let mut metadata = metadata_for(&[]);
        metadata.directories.clear();
        metadata.role = Role::Uninstaller as i32;
        metadata.uninstaller_scope = Scope::User.tag();
        compose(
            out,
            &mut &b"stub engine"[..],
            &metadata,
            std::iter::empty(),
            Compression::Fast,
        )
        .unwrap();

        let installer = Installer::open(&path).unwrap();
        assert!(installer.metadata().is_uninstaller());
        assert_eq!(installer.metadata().served_scope(), Some(Scope::User));
        assert_eq!(installer.entries().unwrap(), Vec::new());
        assert_eq!(installer.payload_archive().unwrap().len(), 0);
        let outcome = installer.verify().unwrap();
        assert!(outcome.is_ok(), "{:?}", outcome.problems);
        assert_eq!(outcome.entries_checked, 0);
    }

    #[test]
    fn a_plain_file_is_not_an_installer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.bin");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        assert_eq!(Installer::open(&path).unwrap_err().code, "footer_missing");
    }
}
