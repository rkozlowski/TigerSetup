//! `tiger-setup inspect --output-payload / --output-zip / --output-meta /
//! --output-meta-json / --output-engine`: the embedded blocks written out
//! exactly as the file carries them, the payload reconstructed as an
//! ordinary archive, the engine decompressed, the metadata decoded, and
//! nothing written where it would mislead — a destination that exists, or
//! an installer that does not verify.

mod common;

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use common::built_installer;
use serde_json::Value;
use tigersetup_build::inspect::{self, ExportRequest, metadata_json};
use tigersetup_format::{Installer, hex, sha256};

const TIGER_SETUP: &str = env!("CARGO_BIN_EXE_tiger-setup");

/// A package that declares one of everything the metadata can carry, so the
/// decoded document has every section to show.
const MANIFEST: &str = r#"
[package]
id = "ItTiger.SampleViewer"
name = "SampleViewer"
version = "0.8.1"
publisher = "IT Tiger"
description = "A local Markdown viewer."
copyright = "Copyright (c) 2026 Ryszard Kozlowski"
license = "MIT"
website = "https://github.com/example/SampleViewer"

[install]
scopes = ["user", "machine"]
existing_scope = "allow-parallel"

[[files]]
source = "payload/**"

[[options]]
name = "path"
kind = "path"
default = true

[[options]]
name = "samples"
label = { "en-US" = "Install the samples", "pl-PL" = "Zainstaluj przykłady" }

[[shortcuts]]
location = "start-menu"
target = "bin/app.exe"

[[path]]
entry = "bin"
option = "path"

[[registry]]
key = "IT Tiger\\SampleViewer"
name = "InstallRoot"
kind = "expand-string"
data = "%INSTALLROOT%"

[registration]
key_name = "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"
display_icon = "bin/app.exe"

[legacy]
installer_type = "inno"
registration_key = "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"

[launch]
executable = "bin/app.exe"
arguments = ["--welcome", "%INSTALLROOT%\\data"]
checked = false

[[dependencies]]
id = "Vendor.Custom"
minimum = "2.0"
detect = { kind = "file-version", path = "%PROGRAMFILES%\\Vendor\\vendor.dll" }
acquire = { url = "https://example.invalid/vendor.exe", sha256 = "0000000000000000000000000000000000000000000000000000000000000000" }
install = { arguments = ["/S"], success_codes = [3010] }
"#;

/// The bytes of `path` between `offset` and `offset + length`.
fn range_of(path: &Path, offset: u64, length: u64) -> Vec<u8> {
    let mut file = std::fs::File::open(path).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut bytes = vec![0u8; usize::try_from(length).unwrap()];
    file.read_exact(&mut bytes).unwrap();
    bytes
}

struct Ran {
    status: i32,
    stdout: String,
    stderr: String,
}

fn tiger_setup(args: &[&str]) -> Ran {
    let output = Command::new(TIGER_SETUP).args(args).output().unwrap();
    Ran {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn inspect_args<'a>(installer: &'a str, more: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["inspect", installer];
    args.extend_from_slice(more);
    args
}

#[test]
fn the_exported_payload_and_metadata_are_the_bytes_the_footer_addresses() {
    let dir = tempfile::tempdir().unwrap();
    let installer = built_installer(dir.path(), MANIFEST);
    let opened = Installer::open(&installer).unwrap();
    let layout = opened.layout().clone();
    let footer = opened.footer().clone();
    drop(opened);

    let payload = dir.path().join("payload.zst");
    let zip = dir.path().join("payload.zip");
    let meta = dir.path().join("metadata.pb");
    let engine = dir.path().join("engine-extracted.exe");
    let ran = tiger_setup(&inspect_args(
        installer.to_str().unwrap(),
        &[
            "--output-payload",
            payload.to_str().unwrap(),
            "--output-zip",
            zip.to_str().unwrap(),
            "--output-meta",
            meta.to_str().unwrap(),
            "--output-engine",
            engine.to_str().unwrap(),
        ],
    ));
    assert_eq!(ran.status, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout.contains("Verify:    ok"),
        "the report is still printed: {}",
        ran.stdout
    );
    for exported in [&payload, &zip, &meta, &engine] {
        assert!(
            ran.stdout
                .contains(&format!("Exported  {}", exported.display())),
            "{}",
            ran.stdout
        );
    }

    // The very byte ranges the footer points at, hashed to what it records.
    let payload_bytes = std::fs::read(&payload).unwrap();
    assert_eq!(
        payload_bytes,
        range_of(&installer, layout.payload_offset, layout.payload_length)
    );
    assert_eq!(sha256(&payload_bytes), footer.payload_sha256);
    // The metadata comes out decompressed: the block in the file is one
    // zstd frame hashed as the block, what it decodes to is hashed as the
    // metadata, and the export is the latter.
    let meta_bytes = std::fs::read(&meta).unwrap();
    let meta_block = range_of(&installer, layout.metadata_offset, layout.metadata_length);
    assert_eq!(sha256(&meta_block), footer.metadata_block_sha256);
    assert_eq!(meta_bytes.len() as u64, layout.metadata_uncompressed_length);
    assert_eq!(sha256(&meta_bytes), footer.metadata_sha256);
    assert_eq!(
        tigersetup_format::payload::decompress_exact(&meta_block, meta_bytes.len() as u64).unwrap(),
        meta_bytes
    );

    // The engine is what the loader runs: the block decompressed, hashing to
    // what the footer and the metadata record.
    let engine_bytes = std::fs::read(&engine).unwrap();
    assert_eq!(engine_bytes.len() as u64, layout.engine_uncompressed_length);
    assert_eq!(sha256(&engine_bytes), footer.engine_executable_sha256);
    let decoded = tigersetup_format::Metadata::decode_block(&meta_bytes).unwrap();
    assert_eq!(
        decoded.engine().engine_block_sha256,
        hex(&sha256(&engine_bytes))
    );
    assert_eq!(decoded, *Installer::open(&installer).unwrap().metadata());

    // The exported ZIP is a reconstruction: an ordinary archive holding the
    // declared files as stored entries, in stream order.
    let zip_bytes = std::fs::read(&zip).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();
    assert_eq!(names, ["bin/app.exe", "readme.txt"]);
    let mut app = Vec::new();
    let mut entry = archive.by_name("bin/app.exe").unwrap();
    assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
    entry.read_to_end(&mut app).unwrap();
    assert_eq!(app, b"application bytes");
}

#[test]
fn the_decoded_metadata_is_the_message_tree_and_not_the_report() {
    let dir = tempfile::tempdir().unwrap();
    let installer = built_installer(dir.path(), MANIFEST);
    let json = dir.path().join("metadata.json");
    let ran = tiger_setup(&inspect_args(
        installer.to_str().unwrap(),
        &["--output-meta-json", json.to_str().unwrap()],
    ));
    assert_eq!(ran.status, 0, "{}{}", ran.stdout, ran.stderr);
    let document: Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();

    // The proto's top-level fields and nothing that belongs to the report:
    // no layout, no footer hashes, no entry listing, no verification, no
    // Windows resource.
    let mut keys: Vec<&str> = document
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = [
        "schema",
        "role",
        "uninstaller_scope",
        "package",
        "install",
        "files",
        "directories",
        "engine",
        "options",
        "shortcuts",
        "path_entries",
        "registry_values",
        "registration",
        "legacy",
        "dependencies",
        "environment_variables",
        "file_associations",
        "url_protocols",
        "app_paths",
        "context_menu_verbs",
        "firewall_rules",
        "actions",
        "quiescence",
        "file_batches",
        "launch",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(document["schema"], 2);
    assert_eq!(document["role"], "installer");
    assert!(document["uninstaller_scope"].is_null());
    assert_eq!(document["package"]["id"], "ItTiger.SampleViewer");
    assert_eq!(document["package"]["version"], "0.8.1");
    assert_eq!(document["package"]["license"], "MIT");
    assert_eq!(
        document["package"]["website_url"],
        "https://github.com/example/SampleViewer"
    );
    assert!(document["package"]["icon"].is_null());
    assert_eq!(
        document["install"]["scopes"],
        serde_json::json!(["user", "machine"])
    );
    assert_eq!(document["install"]["existing_scope"], "allow-parallel");
    assert_eq!(document["install"]["architecture"], "x64");
    assert_eq!(document["files"].as_array().unwrap().len(), 2);
    assert_eq!(document["files"][0]["path"], "bin/app.exe");
    assert_eq!(document["directories"], serde_json::json!(["bin"]));
    assert_eq!(
        document["launch"],
        serde_json::json!({
            "executable": "bin/app.exe",
            "arguments": ["--welcome", "%INSTALLROOT%\\data"],
            "working_directory": null,
            "checked": false,
        })
    );
    assert!(
        document["engine"]["engine_block_sha256"]
            .as_str()
            .unwrap()
            .len()
            == 64
    );
    assert_eq!(document["options"][0]["kind"], "path");
    assert_eq!(document["options"][1]["kind"], "custom");
    assert_eq!(
        document["options"][1]["labels"]["pl-PL"],
        "Zainstaluj przykłady"
    );
    assert_eq!(document["shortcuts"][0]["location"], "start-menu");
    assert_eq!(document["path_entries"][0]["path"], "bin");
    assert_eq!(document["registry_values"][0]["kind"], "expand-string");
    assert_eq!(
        document["registration"]["key_name"],
        "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"
    );
    assert_eq!(document["registration"]["display_icon"], "bin/app.exe");
    assert_eq!(document["legacy"]["installer_type"], "inno");
    assert_eq!(document["dependencies"][0]["id"], "Vendor.Custom");
    assert_eq!(document["dependencies"][0]["acquisition"]["source"], "url");
    assert_eq!(document["dependencies"][0]["install"]["declared"], true);
    assert_eq!(
        document["dependencies"][0]["install"]["success_codes"],
        serde_json::json!([3010])
    );

    // The file is exactly what the library renders for the same metadata.
    let expected = metadata_json(Installer::open(&installer).unwrap().metadata());
    assert_eq!(document, expected);
}

#[test]
fn every_export_at_once_beside_the_json_report() {
    let dir = tempfile::tempdir().unwrap();
    let installer = built_installer(dir.path(), MANIFEST);
    let payload = dir.path().join("payload.zst");
    let meta = dir.path().join("metadata.pb");
    let json = dir.path().join("metadata.json");
    let ran = tiger_setup(&inspect_args(
        installer.to_str().unwrap(),
        &[
            "--json",
            "--output-payload",
            payload.to_str().unwrap(),
            "--output-meta",
            meta.to_str().unwrap(),
            "--output-meta-json",
            json.to_str().unwrap(),
        ],
    ));
    assert_eq!(ran.status, 0, "{}{}", ran.stdout, ran.stderr);

    // stdout is the report and only the report: one JSON document, the same
    // one `inspect --json` prints without exports.
    let report: Value = serde_json::from_str(&ran.stdout).expect("stdout is one JSON document");
    assert_eq!(report["schema"], 1);
    assert_eq!(report["verification"]["status"], "ok");
    assert_eq!(report["package"]["id"], "ItTiger.SampleViewer");
    let plain = tiger_setup(&inspect_args(installer.to_str().unwrap(), &["--json"]));
    assert_eq!(plain.stdout, ran.stdout, "exports do not change the report");
    assert_eq!(
        serde_json::from_str::<Value>(&plain.stdout).unwrap(),
        inspect::inspect(&installer).unwrap().to_json()
    );

    let payload_bytes = std::fs::read(&payload).unwrap();
    let meta_bytes = std::fs::read(&meta).unwrap();
    assert_eq!(
        report["package"]["payload_sha256"],
        hex(&sha256(&payload_bytes)),
        "the report's payload hash is the exported payload block's"
    );
    assert_eq!(
        report["package"]["metadata_sha256"],
        hex(&sha256(&meta_bytes)),
        "the report's metadata hash is the exported metadata's"
    );
    let document: Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_ne!(document, report, "the metadata document is not the report");
    assert!(document.get("layout").is_none() && document.get("verification").is_none());
    assert_eq!(document["package"]["id"], report["package"]["id"]);
    assert_eq!(document["files"], report["files"]);
}

#[test]
fn an_existing_destination_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let installer = built_installer(dir.path(), MANIFEST);
    let zip = dir.path().join("payload.zip");
    let meta = dir.path().join("metadata.pb");
    let json = dir.path().join("metadata.json");
    std::fs::write(&json, b"precious").unwrap();

    // The conflict is on the last destination, requested last; the earlier
    // ones must not have been written on the way to finding it.
    let ran = tiger_setup(&inspect_args(
        installer.to_str().unwrap(),
        &[
            "--output-zip",
            zip.to_str().unwrap(),
            "--output-meta",
            meta.to_str().unwrap(),
            "--output-meta-json",
            json.to_str().unwrap(),
        ],
    ));
    assert_eq!(ran.status, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr.contains("output_exists") && ran.stderr.contains("metadata.json"),
        "{}",
        ran.stderr
    );
    assert_eq!(
        std::fs::read(&json).unwrap(),
        b"precious",
        "not overwritten"
    );
    assert!(!zip.exists(), "no earlier export is left behind");
    assert!(!meta.exists());
    assert!(
        ran.stdout.contains("Verify:    ok"),
        "the report itself was still given: {}",
        ran.stdout
    );

    // The library form says the same.
    let inspection = inspect::inspect(&installer).unwrap();
    let err = inspection
        .export(&ExportRequest {
            zip: Some(zip.clone()),
            meta_json: Some(json.clone()),
            ..ExportRequest::default()
        })
        .unwrap_err();
    assert_eq!(err.code, "output_exists");
    assert!(!zip.exists());
}

#[test]
fn an_installer_that_does_not_verify_exports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let installer = built_installer(dir.path(), MANIFEST);
    // Inside the payload block, which is short: a few files compress to a
    // few dozen bytes.
    let offset = Installer::open(&installer).unwrap().layout().payload_offset + 2;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&installer)
        .unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&[0xAA; 8]).unwrap();
    drop(file);

    let zip = dir.path().join("payload.zip");
    let meta = dir.path().join("metadata.pb");
    let json = dir.path().join("metadata.json");
    let ran = tiger_setup(&inspect_args(
        installer.to_str().unwrap(),
        &[
            "--output-zip",
            zip.to_str().unwrap(),
            "--output-meta",
            meta.to_str().unwrap(),
            "--output-meta-json",
            json.to_str().unwrap(),
        ],
    ));
    assert_eq!(
        ran.status, 1,
        "a failed verification keeps its exit code: {}{}",
        ran.stdout, ran.stderr
    );
    assert!(ran.stdout.contains("Verify:    FAILED"), "{}", ran.stdout);
    assert!(ran.stderr.contains("export_refused"), "{}", ran.stderr);
    assert!(!zip.exists() && !meta.exists() && !json.exists());

    // A file that is not an installer at all is refused the way it always
    // was, and writes nothing either.
    let plain = dir.path().join("plain.bin");
    std::fs::write(&plain, vec![0u8; 8192]).unwrap();
    let ran = tiger_setup(&inspect_args(
        plain.to_str().unwrap(),
        &["--output-zip", zip.to_str().unwrap()],
    ));
    assert_eq!(ran.status, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stderr.contains("footer_missing"), "{}", ran.stderr);
    assert!(!zip.exists());
}

#[test]
fn each_export_stands_alone_and_the_text_report_is_unchanged_by_them() {
    let dir = tempfile::tempdir().unwrap();
    let installer = built_installer(dir.path(), MANIFEST);
    let path = installer.to_str().unwrap();
    let before = tiger_setup(&inspect_args(path, &[]));
    assert_eq!(before.status, 0);

    let outputs: [(&str, PathBuf); 5] = [
        ("--output-payload", dir.path().join("only.zst")),
        ("--output-zip", dir.path().join("only.zip")),
        ("--output-meta", dir.path().join("only.pb")),
        ("--output-meta-json", dir.path().join("only.json")),
        ("--output-engine", dir.path().join("only.exe")),
    ];
    for (option, destination) in &outputs {
        let ran = tiger_setup(&inspect_args(
            path,
            &[option, destination.to_str().unwrap()],
        ));
        assert_eq!(ran.status, 0, "{option}: {}{}", ran.stdout, ran.stderr);
        assert!(destination.exists(), "{option} wrote its file");
        let (report, exported) = ran
            .stdout
            .split_once("Exported  ")
            .expect("the export line follows the report");
        assert_eq!(report, before.stdout, "{option} left the report as it was");
        assert_eq!(exported.trim_end(), destination.to_str().unwrap());
    }
}
