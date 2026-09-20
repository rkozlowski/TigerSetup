//! Reading an installer file: locate the footer, address the blocks, decode
//! the metadata, open the payload through a bounded window, inspect and
//! verify without executing anything.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::footer::{FOOTER_LEN, Footer, locate_footer};
use crate::metadata::PayloadEntry;
use crate::payload::{self, PayloadReader};
use crate::window::Window;
use crate::{FormatError, Metadata, hex, sha256, sha256_reader};

/// The payload opened over a bounded window of the file.
pub type Payload = PayloadReader<Window<File>>;

/// Absolute positions of the blocks in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub file_length: u64,
    /// The loader is bytes `[0, engine_offset)`.
    pub loader_length: u64,
    pub engine_offset: u64,
    pub engine_length: u64,
    pub engine_uncompressed_length: u64,
    pub payload_offset: u64,
    pub payload_length: u64,
    pub payload_uncompressed_length: u64,
    pub metadata_offset: u64,
    pub metadata_length: u64,
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

/// One payload entry as `inspect` lists it: its index record.
pub type EntryInfo = PayloadEntry;

/// The result of a full verification: every check passed, or the first
/// failure per category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyOutcome {
    pub payload_sha256_ok: bool,
    pub engine_sha256_ok: bool,
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
        let footer = read_footer(&file)?;
        let footer_offset = locate_footer(file_length, &read_head(&mut &file)?)?;
        let layout = Layout {
            file_length,
            loader_length: footer.engine_offset,
            engine_offset: footer.engine_offset,
            engine_length: footer.engine_length,
            engine_uncompressed_length: footer.engine_uncompressed_length,
            payload_offset: footer.payload_offset,
            payload_length: footer.payload_length,
            payload_uncompressed_length: footer.payload_uncompressed_length,
            metadata_offset: footer.metadata_offset,
            metadata_length: footer.metadata_length,
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
        // An installer's index must cover what its metadata refers to, and
        // describe exactly the stream the footer measures.
        if !metadata.is_uninstaller() && !metadata.files.is_empty() && metadata.payload.is_empty() {
            return Err(FormatError::new(
                "metadata_invalid",
                "the installer declares files but carries no payload index",
            ));
        }
        let indexed_length = metadata
            .payload
            .last()
            .map(|region| region.offset + region.length)
            .unwrap_or(0);
        if indexed_length != footer.payload_uncompressed_length {
            return Err(FormatError::new(
                "metadata_invalid",
                format!(
                    "the payload index describes {indexed_length} bytes, the footer {}",
                    footer.payload_uncompressed_length
                ),
            ));
        }

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

    /// The hash the footer records for the compressed engine block.
    pub fn engine_block_sha256_hex(&self) -> String {
        hex(&self.footer.engine_sha256)
    }

    /// The hash the footer records for the engine executable the block
    /// decompresses to: what the loader runs.
    pub fn engine_executable_sha256_hex(&self) -> String {
        hex(&self.footer.engine_executable_sha256)
    }

    /// A fresh bounded reader over the loader block, bytes
    /// `[0, engine offset)`.
    pub fn loader_block(&self) -> Result<Window<File>, FormatError> {
        Ok(Window::new(
            self.file.try_clone()?,
            0,
            self.layout.loader_length,
        )?)
    }

    /// A fresh bounded reader over the compressed engine block.
    pub fn engine_block(&self) -> Result<Window<File>, FormatError> {
        Ok(Window::new(
            self.file.try_clone()?,
            self.layout.engine_offset,
            self.layout.engine_length,
        )?)
    }

    /// The compressed engine block as this file carries it, for composing
    /// another file around the same engine — the uninstaller copy — without
    /// decompressing and recompressing anything. The block is read whole;
    /// its hash is checked against the footer, so a damaged installer never
    /// propagates its damage.
    pub fn engine_block_for_composition(&self) -> Result<crate::compose::EngineBlock, FormatError> {
        let mut compressed = Vec::with_capacity(self.layout.engine_length as usize);
        self.engine_block()?.read_to_end(&mut compressed)?;
        if sha256(&compressed) != self.footer.engine_sha256 {
            return Err(FormatError::new(
                "engine_hash_mismatch",
                "the compressed engine block does not match the hash in the footer",
            ));
        }
        Ok(crate::compose::EngineBlock {
            compressed,
            uncompressed_length: self.footer.engine_uncompressed_length,
            executable_sha256: self.footer.engine_executable_sha256,
        })
    }

    /// A fresh bounded reader over the compressed payload block.
    pub fn payload_block(&self) -> Result<Window<File>, FormatError> {
        Ok(Window::new(
            self.file.try_clone()?,
            self.layout.payload_offset,
            self.layout.payload_length,
        )?)
    }

    /// The payload opened for reading entries.
    pub fn payload(&self) -> Result<Payload, FormatError> {
        PayloadReader::new(
            self.payload_block()?,
            self.layout.payload_length,
            &self.metadata.payload,
        )
    }

    /// SHA-256 of the loader block, computed from the file.
    pub fn loader_sha256_hex(&self) -> Result<String, FormatError> {
        Ok(hex(&sha256_reader(&mut self.loader_block()?)?))
    }

    /// SHA-256 of the compressed engine block, computed from the file.
    pub fn engine_sha256_hex(&self) -> Result<String, FormatError> {
        Ok(hex(&sha256_reader(&mut self.engine_block()?)?))
    }

    /// Decompresses the engine executable into `sink`, checking its length
    /// and its hash against the footer ([`extract_engine`]).
    pub fn extract_engine<W: Write>(&self, sink: &mut W) -> Result<u64, FormatError> {
        extract_engine(&self.file, &self.footer, sink)
    }

    /// Lists the payload entries in stream order.
    pub fn entries(&self) -> Result<Vec<EntryInfo>, FormatError> {
        Ok(self.metadata.payload.clone())
    }

    /// Verifies the payload and engine block hashes, then decodes the
    /// payload stream once, checking every entry's CRC-32 and SHA-256
    /// against the index and, for an entry the metadata pins by hash — an
    /// embedded dependency installer, a packaged action program — against
    /// that record too, which the engine refuses to run when it does not
    /// match. (The footer CRC, the metadata hash and the index's
    /// consistency were already checked at open.)
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
        let engine_sha256_ok =
            sha256_reader(&mut self.engine_block()?)? == self.footer.engine_sha256;
        if !engine_sha256_ok {
            problems.push(FormatError::new(
                "engine_hash_mismatch",
                "the compressed engine block does not match the hash in the footer",
            ));
        }

        // The hashed entries, by name.
        let mut pinned: Vec<(&str, &str, &str, &str)> = Vec::new();
        for dependency in &self.metadata.dependencies {
            if let Some(acquisition) = dependency
                .acquisition
                .as_ref()
                .filter(|a| a.source == crate::metadata::AcquisitionSource::Embedded as i32)
            {
                pinned.push((
                    &acquisition.entry,
                    &dependency.id,
                    &acquisition.sha256,
                    "dependency_entry",
                ));
            }
        }
        for action in &self.metadata.actions {
            if action.is_packaged() {
                pinned.push((&action.entry, &action.name, &action.sha256, "action_entry"));
            }
        }

        let mut payload = self.payload()?;
        let mut entries_checked = 0;
        for region in &self.metadata.payload {
            let pin = pinned.iter().find(|(entry, ..)| *entry == region.entry);
            match payload.sha256_of(&region.entry) {
                Ok(digest) => {
                    entries_checked += 1;
                    let digest = hex(&digest);
                    if digest != region.sha256 {
                        problems.push(FormatError::new(
                            "payload_entry_hash_mismatch",
                            format!(
                                "{}: entry has SHA-256 {digest}, the index records {}",
                                region.entry, region.sha256
                            ),
                        ));
                    }
                    if let Some((_, owner, expected, prefix)) = pin
                        && digest != *expected
                    {
                        problems.push(FormatError::new(
                            hash_mismatch_code(prefix),
                            format!(
                                "{owner}: entry {} has SHA-256 {digest}, metadata declares {expected}",
                                region.entry
                            ),
                        ));
                    }
                }
                Err(err) => {
                    problems.push(FormatError::new(
                        err.code,
                        format!("{}: {}", region.entry, err.message),
                    ));
                    // A stream that failed once is not read further: every
                    // entry after the failure would report the same thing.
                    break;
                }
            }
        }
        Ok(VerifyOutcome {
            payload_sha256_ok,
            engine_sha256_ok,
            entries_checked,
            problems,
        })
    }
}

fn hash_mismatch_code(prefix: &str) -> &'static str {
    match prefix {
        "dependency_entry" => "dependency_entry_hash_mismatch",
        _ => "action_entry_hash_mismatch",
    }
}

/// The footer of an installer file, located, decoded and checked against
/// the file's length. This and [`extract_engine`] are all a loader needs.
pub fn read_footer(file: &File) -> Result<Footer, FormatError> {
    let file_length = file.metadata()?.len();
    let mut file = file;
    let head = read_head(&mut file)?;
    let footer_offset = locate_footer(file_length, &head)?;
    file.seek(SeekFrom::Start(footer_offset))?;
    let mut footer_bytes = [0u8; FOOTER_LEN];
    file.read_exact(&mut footer_bytes)?;
    let footer = Footer::decode(&footer_bytes)?;
    footer.check_layout(footer_offset)?;
    Ok(footer)
}

/// Decompresses the engine executable of `file`, mapped by `footer`, into
/// `sink`, checking the compressed block's hash first and then the
/// decompressed length and hash against the footer. Nothing is reported as
/// written until every check passed: a caller that writes to a file it is
/// about to execute discards the file on an error.
pub fn extract_engine<W: Write>(
    file: &File,
    footer: &Footer,
    sink: &mut W,
) -> Result<u64, FormatError> {
    let mut block = Window::new(
        file.try_clone()?,
        footer.engine_offset,
        footer.engine_length,
    )?;
    if sha256_reader(&mut block)? != footer.engine_sha256 {
        return Err(FormatError::new(
            "engine_hash_mismatch",
            "the compressed engine block does not match the hash in the footer",
        ));
    }
    block.seek(SeekFrom::Start(0))?;
    let mut decoder = payload::decoder(block)?;
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    let mut written = 0u64;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let n = decoder.read(&mut buffer).map_err(|err| {
            FormatError::new(
                "engine_invalid",
                format!("the engine block cannot be decompressed: {err}"),
            )
        })?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > footer.engine_uncompressed_length {
            return Err(FormatError::new(
                "engine_invalid",
                "the engine block decompresses to more than the footer declares",
            ));
        }
        sha2::Digest::update(&mut hasher, &buffer[..n]);
        sink.write_all(&buffer[..n])?;
    }
    if written != footer.engine_uncompressed_length {
        return Err(FormatError::new(
            "engine_invalid",
            format!(
                "the engine block decompresses to {written} bytes, the footer declares {}",
                footer.engine_uncompressed_length
            ),
        ));
    }
    let digest: [u8; 32] = sha2::Digest::finalize(hasher).into();
    if digest != footer.engine_executable_sha256 {
        return Err(FormatError::new(
            "engine_hash_mismatch",
            "the engine executable does not match the hash in the footer",
        ));
    }
    Ok(written)
}

/// The first 4 KiB of the file: enough of the PE headers to find a
/// signature's certificate table.
fn read_head(file: &mut &File) -> Result<Vec<u8>, FormatError> {
    file.seek(SeekFrom::Start(0))?;
    let mut head = vec![0u8; 4096];
    let head_len = read_up_to(file, &mut head)?;
    head.truncate(head_len);
    Ok(head)
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
    use crate::compose::{EngineBlock, PayloadBytes, PayloadSource, compose};
    use crate::metadata::{Directory, Engine, File as MetaFile, Install, Package};
    use crate::payload::Compression;

    pub(crate) const STUB_LOADER: &[u8] = b"stub loader";
    pub(crate) const STUB_ENGINE: &[u8] = b"stub engine executable bytes";

    pub(crate) fn metadata_for(files: &[(&str, &[u8])]) -> Metadata {
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
                    when: None,
                })
                .collect(),
            directories: vec![Directory { path: "bin".into() }],
            engine: Some(Engine {
                tigersetup_version: "0.1.0".into(),
                engine_sha256: hex(&sha256(STUB_ENGINE)),
                engine_block_sha256: hex(&sha256(STUB_ENGINE)),
                loader_sha256: hex(&sha256(STUB_LOADER)),
                loader_block_sha256: hex(&sha256(STUB_LOADER)),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn build(dir: &Path, files: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("Sample-Setup.exe");
        let out = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let metadata = metadata_for(files);
        let sources = files
            .iter()
            .map(|(path, bytes)| PayloadSource {
                entry: path.to_string(),
                bytes: PayloadBytes::Memory(bytes.to_vec()),
            })
            .collect();
        compose(
            out,
            &mut &STUB_LOADER[..],
            &EngineBlock::compress(STUB_ENGINE, Compression::Fast).unwrap(),
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
        assert_eq!(installer.layout().loader_length, STUB_LOADER.len() as u64);
        assert_eq!(
            installer.layout().engine_uncompressed_length,
            STUB_ENGINE.len() as u64
        );
        assert_eq!(installer.metadata().package().id, "IT-Tiger.Sample");
        assert_eq!(
            installer.engine_executable_sha256_hex(),
            hex(&sha256(STUB_ENGINE))
        );
        assert_eq!(
            installer.loader_sha256_hex().unwrap(),
            hex(&sha256(STUB_LOADER))
        );
        let entries = installer.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry, "bin/app.exe");
        assert_eq!(entries[1].entry, "bin/data.bin");
        assert_eq!(entries[1].offset, 9);
        assert_eq!(entries[1].length, 100_000);
        assert_eq!(installer.layout().payload_uncompressed_length, 100_009);
        let outcome = installer.verify().unwrap();
        assert!(outcome.is_ok(), "{:?}", outcome.problems);
        assert_eq!(outcome.entries_checked, 2);
        let mut payload = installer.payload().unwrap();
        let mut out = Vec::new();
        payload.copy_entry("bin/app.exe", &mut out).unwrap();
        assert_eq!(out, b"app bytes");

        let mut engine = Vec::new();
        assert_eq!(
            installer.extract_engine(&mut engine).unwrap(),
            STUB_ENGINE.len() as u64
        );
        assert_eq!(engine, STUB_ENGINE);
    }

    fn corrupt(path: &Path, offset: u64, bytes: &[u8]) {
        let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn payload_corruption_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let noise: Vec<u8> = (0..50_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 11) as u8)
            .collect();
        let path = build(dir.path(), &[("bin/noise.bin", &noise)]);
        let offset = Installer::open(&path).unwrap().layout().payload_offset + 100;
        corrupt(&path, offset, &[0xAA; 16]);
        let installer = Installer::open(&path).unwrap();
        let outcome = installer.verify().unwrap();
        assert!(!outcome.payload_sha256_ok);
        assert!(
            outcome.problems.iter().any(|p| p.code == "payload_invalid"
                || p.code == "payload_entry_crc_mismatch"
                || p.code == "payload_truncated"),
            "{:?}",
            outcome.problems
        );
    }

    #[test]
    fn engine_corruption_is_detected_and_nothing_is_extracted() {
        let dir = tempfile::tempdir().unwrap();
        let path = build(dir.path(), &[("bin/app.exe", b"app bytes")]);
        let offset = Installer::open(&path).unwrap().layout().engine_offset + 4;
        corrupt(&path, offset, &[0xFF; 4]);
        let installer = Installer::open(&path).unwrap();
        let outcome = installer.verify().unwrap();
        assert!(!outcome.engine_sha256_ok);
        assert_eq!(
            installer.extract_engine(&mut Vec::new()).unwrap_err().code,
            "engine_hash_mismatch"
        );
    }

    #[test]
    fn metadata_corruption_is_detected_at_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = build(dir.path(), &[("bin/app.exe", b"app bytes")]);
        let offset = Installer::open(&path).unwrap().layout().metadata_offset;
        corrupt(&path, offset + 2, &[0xFF]);
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
        let mut metadata = metadata_for(&[("bin/app.exe", b"app bytes")]);
        metadata.role = Role::Uninstaller as i32;
        metadata.uninstaller_scope = Scope::User.tag();
        compose(
            out,
            &mut &STUB_LOADER[..],
            &EngineBlock::compress(STUB_ENGINE, Compression::Fast).unwrap(),
            &metadata,
            Vec::new(),
            Compression::Fast,
        )
        .unwrap();

        let installer = Installer::open(&path).unwrap();
        assert!(installer.metadata().is_uninstaller());
        assert_eq!(installer.metadata().served_scope(), Some(Scope::User));
        assert_eq!(installer.entries().unwrap(), Vec::new());
        assert_eq!(installer.layout().payload_length, 0);
        let outcome = installer.verify().unwrap();
        assert!(outcome.is_ok(), "{:?}", outcome.problems);
        assert_eq!(outcome.entries_checked, 0);
        assert_eq!(
            installer
                .payload()
                .unwrap()
                .copy_entry("bin/app.exe", &mut Vec::new())
                .unwrap_err()
                .code,
            "payload_entry_missing"
        );
    }

    #[test]
    fn a_plain_file_is_not_an_installer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.bin");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        assert_eq!(Installer::open(&path).unwrap_err().code, "footer_missing");
    }

    #[test]
    fn composition_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let files: &[(&str, &[u8])] = &[("bin/app.exe", b"app bytes"), ("bin/b.dll", b"b b b b")];
        let (dir_a, dir_b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        let a = std::fs::read(build(&dir_a, files)).unwrap();
        let b = std::fs::read(build(&dir_b, files)).unwrap();
        assert_eq!(a, b);
    }
}
