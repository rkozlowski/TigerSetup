//! `TigerSetup.toml` → `Setup.exe`: validate, resolve, generate metadata,
//! give copies of the loader and the engine the product's Windows identity
//! and icon, compress the engine, compose loader, engine, payload and
//! metadata into one file, verify the result by reading it back.
//! Deterministic for a given input, resolved metadata, loader and engine.

use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use tigersetup_format::compose::{EngineBlock, PayloadBytes, PayloadSource, compose};
use tigersetup_format::identity::{self, Scope};
use tigersetup_format::metadata::{
    AppPath, ContextMenuTarget, ContextMenuVerb, Directory, Engine, EnvironmentVariable,
    File as MetaFile, FileAssociation, FirewallAction, FirewallDirection, FirewallProtocol,
    FirewallRule, Install, Legacy, Metadata, OptionChoice, OptionKind, Package, PathEntry,
    Registration, RegistryKind, RegistryRoot, RegistryValue, Role, SCHEMA, Shortcut,
    ShortcutLocation, UrlProtocol,
};
use tigersetup_format::payload::{Compression, PayloadStats};
use tigersetup_format::{Installer, hex, sha256};

use crate::fileset::{self, ResolvedFile};
use crate::manifest::{
    InstallerIcon, LoadedManifest, OptionDefault, install_relative, predicate_of,
};
use crate::metadata::{self, ResolvedPackage};
use crate::resource::{self, Identity};
use crate::{BuildError, Result};
use crate::{actions, dependencies};

/// The TigerSetup version of this builder, recorded in the metadata.
pub const TIGERSETUP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Name of the engine executable expected beside `tiger-setup.exe`.
pub const ENGINE_FILE_NAME: &str = "tigersetup-setup.exe";
/// Name of the loader executable expected beside `tiger-setup.exe`.
pub const LOADER_FILE_NAME: &str = "tigersetup-loader.exe";

pub struct BuildRequest<'a> {
    pub manifest_path: &'a Path,
    /// Where the installer goes: an existing directory (the file takes its
    /// default `<name>-<version>-Setup.exe` name), or a file path ending in
    /// `.exe`.
    pub output: &'a Path,
    /// The engine executable; defaults to `tigersetup-setup.exe` beside the
    /// running builder.
    pub engine_path: Option<&'a Path>,
    /// The loader executable; defaults to `tigersetup-loader.exe` beside the
    /// running builder.
    pub loader_path: Option<&'a Path>,
    /// Global MSBuild properties (`Name=Value`) added to the manifest's own.
    pub properties: &'a [(String, String)],
    /// Do not contact any catalog: dependency hints are left unresolved
    /// (the engine can still detect, and refresh when online) and the
    /// build reports which ones.
    pub offline: bool,
    /// How much effort goes into the payload's and the engine's encoding.
    /// The default is the release-quality build; `Compression::Fast` is the
    /// short iteration loop and produces a functionally identical, larger
    /// installer.
    pub compression: Compression,
}

#[derive(Debug, Clone)]
pub struct BuildResult {
    pub installer_path: PathBuf,
    pub engine_path: PathBuf,
    pub loader_path: PathBuf,
    /// SHA-256 of the engine executable the installer was built from.
    pub engine_sha256: String,
    /// SHA-256 of the engine executable as the loader runs it: that
    /// executable with the product's version resource and icons, before
    /// compression.
    pub engine_block_sha256: String,
    /// SHA-256 of the compressed engine block in the file.
    pub engine_compressed_sha256: String,
    pub engine_compressed_length: u64,
    /// SHA-256 of the loader executable the installer was built from.
    pub loader_sha256: String,
    /// SHA-256 of the loader block as composed: that executable with the
    /// product's version resource and icons.
    pub loader_block_sha256: String,
    pub loader_length: u64,
    pub metadata_sha256: String,
    /// The compressed metadata block and what it decompresses to.
    pub metadata_length: u64,
    pub metadata_uncompressed_length: u64,
    pub payload_sha256: String,
    pub file_count: usize,
    pub payload_length: u64,
    /// What the payload holds.
    pub payload_stats: PayloadStats,
    pub installer_length: u64,
    /// The resolved product metadata with its provenance.
    pub package: ResolvedPackage,
    /// Dependencies whose acquisition hint the catalog supplied, as
    /// `(id, version, url)`.
    pub resolved_dependencies: Vec<(String, String, String)>,
    /// Dependencies whose acquisition hint could not be resolved.
    pub unresolved_dependencies: Vec<(String, String)>,
    /// The custom actions the installer carries, as `(name, phase, program
    /// description)`, so a build says which programs its installer will run.
    pub actions: Vec<(String, String, String)>,
}

/// Where the engine bytes come from when the request names none.
pub fn default_engine_path() -> Result<PathBuf> {
    beside_builder(ENGINE_FILE_NAME)
}

/// Where the loader bytes come from when the request names none.
pub fn default_loader_path() -> Result<PathBuf> {
    beside_builder(LOADER_FILE_NAME)
}

fn beside_builder(file_name: &str) -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| {
        BuildError::new(
            "engine_missing",
            "cannot locate the builder's own directory",
        )
    })?;
    Ok(dir.join(file_name))
}

/// Every directory the files imply, parents first.
pub fn directories_of(files: &[ResolvedFile]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for file in files {
        let parts: Vec<&str> = file.relative.split('/').collect();
        for depth in 1..parts.len() {
            set.insert(parts[..depth].join("/"));
        }
    }
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort_by(|a, b| {
        a.matches('/')
            .count()
            .cmp(&b.matches('/').count())
            .then_with(|| a.cmp(b))
    });
    out
}

fn read_manifest_file(loaded: &LoadedManifest, what: &str, relative: &str) -> Result<Vec<u8>> {
    let path = loaded.resolve(relative);
    std::fs::read(&path).map_err(|err| {
        BuildError::new(
            "manifest_file_unreadable",
            format!("{what} {}: {err}", path.display()),
        )
    })
}

/// The provenance an installer records for the executables it was built
/// from: each release binary's hash and the hash of the block composed from
/// it.
#[derive(Debug, Clone, Default)]
pub struct EngineProvenance {
    pub engine_sha256: String,
    pub engine_block_sha256: String,
    pub loader_sha256: String,
    pub loader_block_sha256: String,
}

/// The runtime metadata for a manifest, its resolved product metadata, a
/// file set (in stream order), resolved dependencies and the provenance of
/// the engine and loader.
pub fn metadata_for(
    loaded: &LoadedManifest,
    package: &ResolvedPackage,
    files: &[ResolvedFile],
    dependencies: Vec<tigersetup_format::metadata::Dependency>,
    actions: Vec<tigersetup_format::metadata::Action>,
    quiescence: Vec<tigersetup_format::metadata::Quiescence>,
    provenance: &EngineProvenance,
) -> Result<Metadata> {
    let manifest = &loaded.manifest;
    let scopes = manifest.scopes();
    let license_text = match &manifest.package.license_file {
        Some(file) => {
            String::from_utf8_lossy(&read_manifest_file(loaded, "license file", file)?).into_owned()
        }
        None => String::new(),
    };
    let icon = match &manifest.package.icon {
        Some(file) => read_manifest_file(loaded, "icon", file)?,
        None => Vec::new(),
    };
    let description = package.description.clone();
    let relative = |value: &Option<String>| -> Result<String> {
        value
            .as_deref()
            .map(install_relative)
            .transpose()
            .map(Option::unwrap_or_default)
    };
    let shortcuts = manifest
        .shortcuts
        .iter()
        .map(|s| -> Result<Shortcut> {
            let is_url = s.url.is_some();
            Ok(Shortcut {
                location: match s.location.as_str() {
                    "desktop" => ShortcutLocation::Desktop as i32,
                    "startup" => ShortcutLocation::Startup as i32,
                    "send-to" => ShortcutLocation::SendTo as i32,
                    _ => ShortcutLocation::StartMenu as i32,
                },
                name: s
                    .name
                    .clone()
                    .unwrap_or_else(|| manifest.package.name.clone()),
                target: if is_url {
                    String::new()
                } else {
                    install_relative(&s.target)?
                },
                arguments: s.arguments.clone(),
                description: s.description.clone().unwrap_or_else(|| description.clone()),
                icon: relative(&s.icon)?,
                // The metadata carries the predicate in its own field; the
                // older `option` field stays empty so an engine reads one
                // gate, not two.
                option: String::new(),
                folder: relative(&s.folder)?,
                when: predicate_of(s.when.as_ref(), s.option.as_ref()),
                working_directory: relative(&s.working_directory)?,
                app_user_model_id: s.app_user_model_id.clone().unwrap_or_default(),
                url: s.url.clone().unwrap_or_default(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let path_entries = manifest
        .path
        .iter()
        .map(|p| -> Result<PathEntry> {
            Ok(PathEntry {
                path: install_relative(&p.entry)?,
                option: String::new(),
                when: predicate_of(p.when.as_ref(), p.option.as_ref()),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let registry_values = manifest
        .registry
        .iter()
        .map(|r| RegistryValue {
            key: r.key.clone(),
            name: r.name.clone(),
            kind: match r.kind.as_str() {
                "dword" => RegistryKind::Dword as i32,
                "expand-string" => RegistryKind::ExpandString as i32,
                _ => RegistryKind::String as i32,
            },
            data: r.data.clone(),
            when: predicate_of(r.when.as_ref(), None),
            // Validated by the manifest; an unknown spelling never gets here.
            root: r
                .root
                .as_deref()
                .and_then(RegistryValue::root_of)
                .unwrap_or(RegistryRoot::Software) as i32,
        })
        .collect();
    let environment_variables = manifest
        .environment
        .iter()
        .map(|v| EnvironmentVariable {
            name: v.name.clone(),
            value: v.value.clone(),
            expandable: v.expandable,
            when: predicate_of(v.when.as_ref(), None),
        })
        .collect();
    let file_associations = manifest
        .file_associations
        .iter()
        .map(|a| -> Result<FileAssociation> {
            Ok(FileAssociation {
                prog_id: a.prog_id.clone(),
                extensions: a
                    .extensions
                    .iter()
                    .map(|e| e.to_ascii_lowercase())
                    .collect(),
                description: a.description.clone(),
                icon: relative(&a.icon)?,
                executable: install_relative(&a.executable)?,
                arguments: a.arguments.clone().unwrap_or_default(),
                when: predicate_of(a.when.as_ref(), None),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let url_protocols = manifest
        .url_protocols
        .iter()
        .map(|u| -> Result<UrlProtocol> {
            Ok(UrlProtocol {
                scheme: u.scheme.to_ascii_lowercase(),
                prog_id: u.prog_id.clone().unwrap_or_default(),
                description: u.description.clone().unwrap_or_default(),
                icon: relative(&u.icon)?,
                executable: install_relative(&u.executable)?,
                arguments: u.arguments.clone().unwrap_or_default(),
                when: predicate_of(u.when.as_ref(), None),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let app_paths = manifest
        .app_paths
        .iter()
        .map(|a| -> Result<AppPath> {
            Ok(AppPath {
                executable: install_relative(&a.executable)?,
                add_directory: a.add_directory,
                when: predicate_of(a.when.as_ref(), None),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let context_menu_verbs = manifest
        .context_menu
        .iter()
        .map(|v| -> Result<ContextMenuVerb> {
            Ok(ContextMenuVerb {
                target: match v.target.as_str() {
                    "directories" => ContextMenuTarget::Directories as i32,
                    "directory-background" => ContextMenuTarget::DirectoryBackground as i32,
                    _ => ContextMenuTarget::Files as i32,
                },
                verb: v.verb.clone(),
                label: v.label.clone(),
                executable: install_relative(&v.executable)?,
                arguments: v.arguments.clone().unwrap_or_default(),
                icon: relative(&v.icon)?,
                extensions: v
                    .extensions
                    .iter()
                    .map(|e| e.to_ascii_lowercase())
                    .collect(),
                when: predicate_of(v.when.as_ref(), None),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let firewall_rules = manifest
        .firewall
        .iter()
        .map(|f| -> Result<FirewallRule> {
            Ok(FirewallRule {
                name: f.name.clone(),
                description: f.description.clone(),
                program: install_relative(&f.program)?,
                direction: match f.direction.as_str() {
                    "out" => FirewallDirection::Out as i32,
                    _ => FirewallDirection::In as i32,
                },
                action: match f.action.as_str() {
                    "block" => FirewallAction::Block as i32,
                    _ => FirewallAction::Allow as i32,
                },
                protocol: match f.protocol.as_deref().unwrap_or("any") {
                    "tcp" => FirewallProtocol::Tcp as i32,
                    "udp" => FirewallProtocol::Udp as i32,
                    _ => FirewallProtocol::Unspecified as i32,
                },
                local_ports: f.local_ports.clone().unwrap_or_default(),
                when: predicate_of(f.when.as_ref(), None),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let registration = Registration {
        key_name: manifest.registration.key_name.clone().unwrap_or_default(),
        display_name: manifest
            .registration
            .display_name
            .clone()
            .unwrap_or_default(),
        display_version: manifest
            .registration
            .display_version
            .clone()
            .unwrap_or_default(),
        display_icon: manifest
            .registration
            .display_icon
            .as_deref()
            .map(install_relative)
            .transpose()?
            .unwrap_or_default(),
    };
    let meta_files: Vec<MetaFile> = files
        .iter()
        .map(|f| MetaFile {
            path: f.relative.clone(),
            size: f.size,
            entry: f.relative.clone(),
            when: f.when.clone(),
        })
        .collect();
    // The batches the engine journals and recovers the files by, decided
    // here from the stream order and the sizes, and carried as data.
    let file_batches = tigersetup_format::metadata::file_batches(&meta_files);
    Ok(Metadata {
        schema: SCHEMA,
        package: Some(Package {
            id: manifest.package.id.clone(),
            name: manifest.package.name.clone(),
            version: package.version.clone(),
            publisher: manifest.package.publisher.clone(),
            description,
            copyright: package.copyright.clone(),
            license: manifest.package.license.clone().unwrap_or_default(),
            license_text,
            website_url: manifest.package.website.clone().unwrap_or_default(),
            support_url: manifest.package.support.clone().unwrap_or_default(),
            help_url: manifest.package.help.clone().unwrap_or_default(),
            icon,
            file_version: package.file_version.clone(),
        }),
        install: Some(Install {
            scopes: scopes.iter().map(|s| s.tag()).collect(),
            user_root: manifest.install_root_template(Scope::User),
            machine_root: manifest.install_root_template(Scope::Machine),
            minimum_build: manifest.install.minimum_build.unwrap_or(0),
            architecture: manifest
                .install
                .architecture
                .clone()
                .unwrap_or_else(|| "x64".into()),
            estimated_size: files.iter().map(|f| f.size).sum(),
            existing_scope: manifest.existing_scope_policy() as i32,
        }),
        files: meta_files,
        file_batches,
        directories: directories_of(files)
            .into_iter()
            .map(|path| Directory { path })
            .collect(),
        engine: Some(Engine {
            tigersetup_version: TIGERSETUP_VERSION.into(),
            engine_sha256: provenance.engine_sha256.clone(),
            engine_block_sha256: provenance.engine_block_sha256.clone(),
            loader_sha256: provenance.loader_sha256.clone(),
            loader_block_sha256: provenance.loader_block_sha256.clone(),
        }),
        role: Role::Installer as i32,
        uninstaller_scope: 0,
        options: manifest
            .options
            .iter()
            .map(|o| tigersetup_format::metadata::InstallOption {
                name: o.name.to_ascii_lowercase(),
                default: matches!(o.default, OptionDefault::Bool(true)),
                kind: match o.kind_name() {
                    "path" => OptionKind::Path as i32,
                    "desktop-shortcut" => OptionKind::DesktopShortcut as i32,
                    "choice" => OptionKind::Choice as i32,
                    _ => OptionKind::Custom as i32,
                },
                labels: o
                    .label
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                choices: o
                    .choices
                    .iter()
                    .map(|c| OptionChoice {
                        value: c.value.to_ascii_lowercase(),
                        labels: c
                            .label
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    })
                    .collect(),
                default_choice: match &o.default {
                    OptionDefault::Choice(value) => value.to_ascii_lowercase(),
                    OptionDefault::Bool(_) => String::new(),
                },
            })
            .collect(),
        shortcuts,
        path_entries,
        registry_values,
        registration: Some(registration),
        legacy: manifest.legacy.as_ref().map(|l| Legacy {
            installer_type: l.installer_type.clone(),
            registration_key: l.registration_key.clone(),
        }),
        dependencies,
        environment_variables,
        file_associations,
        url_protocols,
        app_paths,
        context_menu_verbs,
        firewall_rules,
        actions,
        // The index is written by composition, once the payload exists.
        payload: Vec::new(),
        quiescence,
    })
}

/// Reads a release binary the installer is built from.
fn read_binary(what: &str, path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|err| {
        BuildError::new(
            "engine_missing",
            format!("cannot open {what} {}: {err}", path.display()),
        )
    })
}

/// The installer file the request names.
pub fn installer_path_for(output: &Path, name: &str, version: &str) -> PathBuf {
    let is_file = output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        && !output.is_dir();
    if is_file {
        output.to_path_buf()
    } else {
        output.join(identity::installer_file_name(name, version))
    }
}

/// The Windows identity a generated installer carries: the product being
/// installed, from the manifest and the resolved metadata, under the file
/// name the installer actually gets. Nothing of TigerSetup is in it.
pub fn installer_identity(
    loaded: &LoadedManifest,
    package: &ResolvedPackage,
    installer_path: &Path,
) -> Identity {
    let manifest = &loaded.manifest;
    let file_name = installer_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let internal_name = file_name
        .strip_suffix(".exe")
        .or_else(|| file_name.strip_suffix(".EXE"))
        .unwrap_or(&file_name)
        .to_string();
    Identity {
        company_name: manifest.package.publisher.clone(),
        product_name: manifest.package.name.clone(),
        product_version: package.version.clone(),
        file_version: resource::padded_version(&package.version),
        file_description: format!("{} Setup", manifest.package.name),
        legal_copyright: package.copyright.clone(),
        original_filename: file_name,
        internal_name,
    }
}

/// The bytes of the `.ico` that becomes the installer's executable icon,
/// by the manifest's `installer.icon` policy.
fn executable_icon(loaded: &LoadedManifest) -> Result<Vec<u8>> {
    match loaded.manifest.installer_icon() {
        InstallerIcon::TigerSetup => Ok(resource::TIGERSETUP_ICON.to_vec()),
        InstallerIcon::File(relative) => read_manifest_file(loaded, "installer icon", &relative),
    }
}

pub fn build(request: &BuildRequest<'_>) -> Result<BuildResult> {
    let loaded = LoadedManifest::load(request.manifest_path)?;
    let package = metadata::resolve_package(&loaded, request.properties)?;
    let mut files = fileset::resolve(&loaded.directory, &loaded.manifest.files)?;
    // The file list is the stream order: what the metadata lists is what
    // the engine installs, in that order, reading the payload once.
    tigersetup_format::payload::sort_product_entries(&mut files, |file| file.relative.as_str());
    let resolved = dependencies::resolve(&loaded, request.offline)?;
    let actions = actions::resolve(&loaded)?;

    let action_summary: Vec<(String, String, String)> = actions
        .actions
        .iter()
        .map(|a| {
            (
                a.name.clone(),
                a.phase().as_str().to_string(),
                if a.is_packaged() {
                    format!("packaged {} sha256 {}", a.file_name, a.sha256)
                } else {
                    a.command.clone()
                },
            )
        })
        .collect();

    let engine_path = match request.engine_path {
        Some(path) => path.to_path_buf(),
        None => default_engine_path()?,
    };
    let loader_path = match request.loader_path {
        Some(path) => path.to_path_buf(),
        None => default_loader_path()?,
    };
    let engine = read_binary("engine", &engine_path)?;
    let loader = read_binary("loader", &loader_path)?;
    let engine_sha256 = hex(&sha256(&engine));
    let loader_sha256 = hex(&sha256(&loader));

    let installer_path = installer_path_for(
        request.output,
        &loaded.manifest.package.name,
        &package.version,
    );
    let identity = installer_identity(&loaded, &package, &installer_path);
    let exe_icon = executable_icon(&loaded)?;
    // Both copies get the product's identity: the loader is what Explorer
    // shows for the file, the engine is the process the taskbar, Task
    // Manager and the Restart Manager name while the installer runs.
    let loader_block = resource::apply(&loader, &identity, &exe_icon, resource::TIGERSETUP_ICON)?;
    drop(loader);
    let engine_block = resource::apply(&engine, &identity, &exe_icon, resource::TIGERSETUP_ICON)?;
    drop(engine);
    let loader_block_sha256 = hex(&sha256(&loader_block));
    let engine_block_sha256 = hex(&sha256(&engine_block));
    let compressed_engine = EngineBlock::compress(&engine_block, request.compression)?;
    drop(engine_block);

    let provenance = EngineProvenance {
        engine_sha256,
        engine_block_sha256,
        loader_sha256,
        loader_block_sha256,
    };
    let metadata = metadata_for(
        &loaded,
        &package,
        &files,
        resolved.dependencies,
        actions.actions,
        actions.quiescence,
        &provenance,
    )?;
    metadata.validate()?;

    if let Some(parent) = installer_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial_path = installer_path.with_extension("exe.partial");
    let _ = std::fs::remove_file(&partial_path);
    let out = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&partial_path)?;

    // The stream: every embedded dependency installer and every packaged
    // action file under its reserved entry name first — the engine needs
    // those before and at the start of its transaction — then the product
    // files in their order.
    let mut reserved: Vec<(String, PathBuf)> = resolved
        .embedded
        .iter()
        .chain(actions.packaged.iter())
        .map(|(entry, path)| (entry.clone(), path.clone()))
        .collect();
    reserved.sort_by(|a, b| a.0.cmp(&b.0));
    let sources: Vec<PayloadSource> = reserved
        .into_iter()
        .chain(
            files
                .iter()
                .map(|file| (file.relative.clone(), file.source.clone())),
        )
        .map(|(entry, source)| PayloadSource {
            entry,
            bytes: PayloadBytes::File(source),
        })
        .collect();
    let composed = match compose(
        out,
        &mut &loader_block[..],
        &compressed_engine,
        &metadata,
        sources,
        request.compression,
    ) {
        Ok(composed) => composed,
        Err(err) => {
            let _ = std::fs::remove_file(&partial_path);
            return Err(err.into());
        }
    };
    let _ = std::fs::remove_file(&installer_path);
    std::fs::rename(&partial_path, &installer_path)?;

    // Read the result back through the same code the engine uses.
    let installer = Installer::open(&installer_path)?;
    let outcome = installer.verify()?;
    if !outcome.is_ok() {
        let problems: Vec<String> = outcome.problems.iter().map(|p| p.to_string()).collect();
        return Err(BuildError::new(
            "installer_invalid",
            format!(
                "the composed installer failed verification: {}",
                problems.join("; ")
            ),
        ));
    }

    Ok(BuildResult {
        installer_length: installer.layout().file_length,
        payload_stats: composed.payload,
        installer_path,
        engine_path,
        loader_path,
        engine_sha256: provenance.engine_sha256,
        engine_block_sha256: provenance.engine_block_sha256,
        engine_compressed_sha256: hex(&composed.footer.engine_sha256),
        engine_compressed_length: composed.footer.engine_length,
        loader_sha256: provenance.loader_sha256,
        loader_block_sha256: provenance.loader_block_sha256,
        loader_length: composed.footer.engine_offset,
        metadata_sha256: hex(&composed.footer.metadata_sha256),
        metadata_length: composed.footer.metadata_length,
        metadata_uncompressed_length: composed.footer.metadata_uncompressed_length,
        payload_sha256: hex(&composed.footer.payload_sha256),
        file_count: files.len(),
        payload_length: composed.footer.payload_length,
        package,
        resolved_dependencies: resolved.resolved,
        unresolved_dependencies: resolved.unresolved,
        actions: action_summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect;

    /// A real PE to stand in for the engine and the loader alike: a builder
    /// test must not depend on the product binaries, and the rewrite needs a
    /// file the resource API accepts.
    fn engine_fixture(root: &Path) -> PathBuf {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let inbox = PathBuf::from(system_root)
            .join("System32")
            .join("notepad.exe");
        let engine = root.join("engine.exe");
        std::fs::copy(inbox, &engine).unwrap();
        engine
    }

    fn write_sample(root: &Path, manifest: &str) {
        std::fs::create_dir_all(root.join("payload/bin")).unwrap();
        std::fs::write(
            root.join("payload/bin/app.exe"),
            b"application bytes application bytes",
        )
        .unwrap();
        std::fs::write(root.join("payload/readme.txt"), b"hello").unwrap();
        std::fs::write(root.join("LICENSE.txt"), b"MIT License").unwrap();
        std::fs::write(root.join("TigerSetup.toml"), manifest).unwrap();
    }

    /// A two-image icon with recognisable bytes, unlike TigerSetup's.
    fn sample_icon() -> (Vec<u8>, [&'static [u8]; 2]) {
        let images: [&[u8]; 2] = [b"sample icon image one", b"sample icon image two, longer"];
        let mut ico = vec![0u8, 0, 1, 0, 2, 0];
        let mut offset = 6 + 2 * 16;
        for (width, image) in [16u8, 32].iter().zip(images) {
            ico.extend_from_slice(&[*width, *width, 0, 0, 1, 0, 32, 0]);
            ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
            ico.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += image.len();
        }
        for image in images {
            ico.extend_from_slice(image);
        }
        (ico, images)
    }

    const SAMPLE: &str = "[package]\nid = \"IT-Tiger.Sample\"\nname = \"Sample\"\nversion = \"1.2.3\"\npublisher = \"IT Tiger\"\nlicense_file = \"LICENSE.txt\"\n\n[[files]]\nsource = \"payload/**\"\n\n[[options]]\nname = \"path\"\nkind = \"path\"\ndefault = true\n\n[[path]]\nentry = \".\"\noption = \"path\"\n\n[[shortcuts]]\nlocation = \"start-menu\"\ntarget = \"bin/app.exe\"\n";

    #[test]
    fn builds_a_verifiable_installer_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_sample(root, SAMPLE);
        let engine = engine_fixture(root);

        let request = BuildRequest {
            manifest_path: &root.join("TigerSetup.toml"),
            output: &root.join("out"),
            engine_path: Some(&engine),
            loader_path: Some(&engine),
            properties: &[],
            offline: true,
            // Tests build many installers; the payload's size is not what
            // they measure.
            compression: Compression::Fast,
        };
        let first = build(&request).unwrap();
        assert_eq!(
            first.installer_path.file_name().unwrap(),
            "Sample-1.2.3-Setup.exe"
        );
        assert_eq!(first.file_count, 2);
        let bytes_first = std::fs::read(&first.installer_path).unwrap();
        let second = build(&request).unwrap();
        assert_eq!(bytes_first, std::fs::read(&second.installer_path).unwrap());

        let installer = Installer::open(&first.installer_path).unwrap();
        let metadata = installer.metadata();
        assert_eq!(metadata.directories, vec![Directory { path: "bin".into() }]);
        assert_eq!(metadata.engine().engine_sha256, first.engine_sha256);
        assert_eq!(
            metadata.engine().engine_block_sha256,
            first.engine_block_sha256
        );
        assert_eq!(
            installer.engine_executable_sha256_hex(),
            first.engine_block_sha256,
            "the footer records the engine the loader runs"
        );
        assert_eq!(
            installer.engine_sha256_hex().unwrap(),
            first.engine_compressed_sha256
        );
        let mut extracted = Vec::new();
        installer.extract_engine(&mut extracted).unwrap();
        assert_eq!(hex(&sha256(&extracted)), first.engine_block_sha256);
        assert_ne!(first.engine_sha256, first.engine_block_sha256);
        assert_eq!(metadata.engine().loader_sha256, first.loader_sha256);
        assert_eq!(
            installer.loader_sha256_hex().unwrap(),
            first.loader_block_sha256
        );
        assert_eq!(
            first.engine_sha256,
            hex(&sha256(&std::fs::read(&engine).unwrap())),
            "the recorded engine hash is the pristine executable's"
        );
        assert_eq!(metadata.package().license_text, "MIT License");
        assert_eq!(metadata.install().estimated_size, 40);
        assert_eq!(
            metadata.option_default("path"),
            Some(tigersetup_format::metadata::OptionValue::Bool(true))
        );
        assert_eq!(metadata.path_entries[0].path, "");
        assert_eq!(metadata.shortcuts[0].name, "Sample");
        assert_eq!(metadata.shortcuts[0].target, "bin/app.exe");
        assert_eq!(metadata.registration_key_name(), "IT-Tiger.Sample");
        assert!(installer.verify().unwrap().is_ok());

        // A file path as the output names the installer directly, and the
        // identity follows the actual file name.
        let named = BuildRequest {
            output: &root.join("named").join("Custom-Setup.exe"),
            ..request
        };
        let result = build(&named).unwrap();
        assert_eq!(
            result.installer_path,
            root.join("named").join("Custom-Setup.exe")
        );
        let inspection = inspect::inspect(&result.installer_path).unwrap();
        let windows = &inspection.to_json()["windows"];
        assert_eq!(windows["original_filename"], "Custom-Setup.exe");
        assert_eq!(windows["internal_name"], "Custom-Setup");
    }

    #[test]
    fn the_installer_carries_the_products_identity_and_icon() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_sample(
            root,
            &SAMPLE.replace(
                "publisher = \"IT Tiger\"",
                "publisher = \"Sample Publisher\"\ncopyright = \"(c) 2026 Sample Publisher\"\nicon = \"assets/sample.ico\"",
            ),
        );
        let (ico, images) = sample_icon();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/sample.ico"), &ico).unwrap();
        let engine = engine_fixture(root);

        let request = BuildRequest {
            manifest_path: &root.join("TigerSetup.toml"),
            output: &root.join("out"),
            engine_path: Some(&engine),
            loader_path: Some(&engine),
            properties: &[],
            offline: true,
            compression: Compression::Fast,
        };
        let result = build(&request).unwrap();
        let inspection = inspect::inspect(&result.installer_path).unwrap();
        assert!(inspection.is_ok());
        let report = inspection.to_json();

        let windows = &report["windows"];
        assert_eq!(windows["company_name"], "Sample Publisher");
        assert_eq!(windows["product_name"], "Sample");
        assert_eq!(windows["product_version"], "1.2.3");
        assert_eq!(windows["file_version"], "1.2.3.0");
        assert_eq!(windows["file_description"], "Sample Setup");
        assert_eq!(windows["copyright"], "(c) 2026 Sample Publisher");
        assert_eq!(windows["original_filename"], "Sample-1.2.3-Setup.exe");
        assert_eq!(windows["internal_name"], "Sample-1.2.3-Setup");
        let text = serde_json::to_string(windows).unwrap();
        assert!(!text.contains("TigerSetup"), "{text}");

        // The executable's icon is the branding icon by default; the brand
        // mark stays TigerSetup's.
        let icon = report["icon"].as_array().unwrap();
        assert_eq!(icon.len(), 2);
        assert_eq!(icon[0]["width"], 16);
        assert_eq!(icon[1]["width"], 32);
        assert_eq!(icon[1]["height"], 32);
        assert_eq!(icon[1]["bits"], 32);
        assert_eq!(icon[1]["bytes"], images[1].len());
        assert_eq!(icon[1]["sha256"], hex(&sha256(images[1])));
        let brand = resource::read_group_icon(&result.installer_path, resource::BRAND_ICON_ID)
            .unwrap()
            .unwrap();
        assert_eq!(
            brand,
            resource::icon::parse("t", resource::TIGERSETUP_ICON).unwrap()
        );

        let engine_info = &report["package"]["engine"];
        assert_eq!(engine_info["engine_sha256"], result.engine_sha256);
        assert_eq!(
            engine_info["engine_sha256"],
            hex(&sha256(&std::fs::read(&engine).unwrap()))
        );
        assert_eq!(
            engine_info["engine_block_sha256"],
            result.engine_block_sha256
        );
        assert_eq!(
            engine_info["engine_block_sha256_actual"],
            result.engine_block_sha256
        );
        let block = Installer::open(&result.installer_path)
            .unwrap()
            .engine_executable_sha256_hex();
        assert_eq!(engine_info["engine_block_sha256"], block);
        assert_eq!(report["verification"]["status"], "ok");

        // The wizard's branding icon in the metadata stays what it was.
        let installer = Installer::open(&result.installer_path).unwrap();
        assert_eq!(installer.metadata().package().icon, ico);

        // "tigersetup" keeps TigerSetup's icon on the executable even beside
        // a branding icon.
        std::fs::write(
            root.join("TigerSetup.toml"),
            format!(
                "{}\n[installer]\nicon = \"tigersetup\"\n",
                std::fs::read_to_string(root.join("TigerSetup.toml")).unwrap()
            ),
        )
        .unwrap();
        let result = build(&request).unwrap();
        let report = inspect::inspect(&result.installer_path).unwrap().to_json();
        // TigerSetup's own icon: the six sizes of the small 8-bit artwork,
        // and no 256-pixel image.
        assert_eq!(report["icon"].as_array().unwrap().len(), 6);
        assert_eq!(report["windows"]["product_name"], "Sample");
    }

    /// A packaged action program travels under the reserved payload
    /// directory with its exact bytes and hash, once however many actions
    /// share it; `inspect` shows every action and `verify` checks the bytes.
    #[test]
    fn packaged_actions_are_embedded_hashed_and_inspected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_sample(
            root,
            &format!(
                "{SAMPLE}\n[[actions]]\nname = \"configure\"\nphase = \"post-install\"\nkind = \"powershell\"\nsource = \"actions/configure.ps1\"\narguments = [\"-Root\", \"%INSTALLROOT%\"]\n\n                 [[actions]]\nname = \"unconfigure\"\nphase = \"pre-uninstall\"\nkind = \"powershell\"\nsource = \"actions/configure.ps1\"\narguments = [\"-Remove\"]\non_failure = \"continue\"\n\n                 [[actions]]\nname = \"notify\"\nphase = \"post-install\"\nkind = \"exe\"\ncommand = \"%INSTALLROOT%\\\\bin\\\\app.exe\"\nreboot_codes = [3010]\n"
            ),
        );
        std::fs::create_dir_all(root.join("actions")).unwrap();
        let script = b"param($Root) 'configured'";
        std::fs::write(root.join("actions/configure.ps1"), script).unwrap();
        let engine = engine_fixture(root);
        let request = BuildRequest {
            manifest_path: &root.join("TigerSetup.toml"),
            output: &root.join("out"),
            engine_path: Some(&engine),
            loader_path: Some(&engine),
            properties: &[],
            offline: true,
            compression: Compression::Fast,
        };
        let result = build(&request).unwrap();
        assert_eq!(
            result.file_count, 2,
            "action programs are not product files"
        );

        let installer = Installer::open(&result.installer_path).unwrap();
        let metadata = installer.metadata();
        assert_eq!(metadata.actions.len(), 3);
        let configure = &metadata.actions[0];
        assert_eq!(configure.entry, ".tigersetup/actions/configure.ps1");
        assert_eq!(configure.file_name, "configure.ps1");
        assert_eq!(configure.size, script.len() as u64);
        assert_eq!(configure.sha256, hex(&sha256(script)));
        assert_eq!(metadata.actions[1].entry, configure.entry, "shared");
        assert!(!metadata.actions[2].is_packaged());
        let entries: Vec<String> = installer
            .entries()
            .unwrap()
            .into_iter()
            .map(|e| e.entry)
            .collect();
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.as_str() == ".tigersetup/actions/configure.ps1")
                .count(),
            1,
            "one entry for one file: {entries:?}"
        );
        assert!(installer.verify().unwrap().is_ok());

        let inspection = inspect::inspect(&result.installer_path).unwrap();
        assert!(inspection.is_ok());
        let report = inspection.to_json();
        let actions = report["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0]["name"], "configure");
        assert_eq!(actions[0]["phase"], "post-install");
        assert_eq!(actions[0]["kind"], "powershell");
        assert_eq!(
            actions[0]["run_on"],
            serde_json::json!(["install", "upgrade", "reinstall"])
        );
        assert_eq!(actions[0]["packaged"]["sha256"], hex(&sha256(script)));
        assert_eq!(
            actions[0]["packaged"]["entry"],
            ".tigersetup/actions/configure.ps1"
        );
        assert_eq!(actions[0]["timeout_seconds"], 300);
        assert_eq!(actions[0]["on_failure"], "fail");
        assert_eq!(actions[1]["on_failure"], "continue");
        assert_eq!(actions[1]["run_on"], serde_json::json!(["uninstall"]));
        assert!(actions[2]["packaged"].is_null());
        assert_eq!(actions[2]["command"], "%INSTALLROOT%\\bin\\app.exe");
        assert_eq!(actions[2]["reboot_codes"], serde_json::json!([3010]));
        let decoded = inspect::metadata_json(metadata);
        assert_eq!(decoded["actions"].as_array().unwrap().len(), 3);
        let text = inspection.to_text();
        assert!(
            text.contains(
                "Action:    configure · post-install · powershell · packaged configure.ps1 sha256"
            ),
            "{text}"
        );
        assert!(text.contains("Action:    notify · post-install · exe · %INSTALLROOT%"));
    }

    #[test]
    fn a_missing_or_unusable_installer_icon_stops_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_sample(
            root,
            &format!("{SAMPLE}\n[installer]\nicon = \"assets/setup.ico\"\n"),
        );
        let engine = engine_fixture(root);
        let request = BuildRequest {
            manifest_path: &root.join("TigerSetup.toml"),
            output: &root.join("out"),
            engine_path: Some(&engine),
            loader_path: Some(&engine),
            properties: &[],
            offline: true,
            compression: Compression::Fast,
        };
        assert_eq!(
            build(&request).unwrap_err().code,
            "manifest_file_unreadable"
        );
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/setup.ico"), b"not an icon").unwrap();
        assert_eq!(build(&request).unwrap_err().code, "icon_invalid");
        assert!(
            !root.join("out").join("Sample-1.2.3-Setup.exe").exists(),
            "nothing is written when the icon is refused"
        );
    }
}
