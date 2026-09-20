//! Builds `tigersetup-loader.exe`, the C Win32 executable every generated
//! `Setup.exe` begins with, and places it in the profile directory beside
//! `tiger-setup.exe`, where the builder expects it.
//!
//! The loader is compiled from `src/loader.c` and libzstd's decoder, taken
//! from the sources the `zstd-sys` crate carries (`DEP_ZSTD_INCLUDE`), so
//! the loader decodes with the same libzstd the engine and the builder
//! link. Nothing of the compressor is compiled. The objects are linked with
//! `link.exe` directly: the loader is not a Rust program, and Cargo's own
//! linking would give it a Rust runtime it must not carry.
//!
//! Two products come out: the loader, written to the profile directory
//! atomically and announced to dependents as `DEP_TIGERSETUP_LOADER_LOADER`;
//! and the test fixture `fake-engine.exe`, compiled from
//! `tests/fake-engine.c` into `OUT_DIR` for this package's own tests.

#[path = "../build-support/version_resource.rs"]
mod version_resource;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The libzstd translation units the decoder needs, relative to `lib/`.
/// `xxhash.c` is among them because the decoder references the XXH64 state
/// functions for a frame that carries a checksum, even though TigerSetup
/// frames carry none; the linker keeps only the functions referenced.
const ZSTD_SOURCES: &[&str] = &[
    "common/debug.c",
    "common/entropy_common.c",
    "common/error_private.c",
    "common/fse_decompress.c",
    "common/xxhash.c",
    "common/zstd_common.c",
    "decompress/huf_decompress.c",
    "decompress/zstd_ddict.c",
    "decompress/zstd_decompress.c",
    "decompress/zstd_decompress_block.c",
];

/// libzstd configured as a decoder and nothing else: no legacy formats, no
/// deprecated interfaces, no error strings, no tracing, no assembly, no
/// runtime BMI2 dispatch. The allocator is the process heap through
/// `src/zstd_alloc.h`, forced into every unit, and xxhash's own allocator
/// (never reached by the decoder) is compiled out rather than linked.
///
/// The three size options are libzstd's documented ones, each measured on
/// the linked loader: one Huffman decoder instead of two (13.8 KB), one
/// sequence decoder instead of a short and a long one (7.2 KB), no forced
/// inlining (6.7 KB). Together they cost about 2.5 ms decoding a 2.6 MB
/// engine, once per launch.
const ZSTD_DEFINES: &[(&str, &str)] = &[
    ("ZSTD_LEGACY_SUPPORT", "0"),
    ("ZSTD_LIB_DEPRECATED", "0"),
    ("ZSTD_STRIP_ERROR_STRINGS", "1"),
    ("ZSTD_NO_UNUSED_FUNCTIONS", "1"),
    ("ZSTD_NO_TRACE", "1"),
    ("ZSTD_TRACE", "0"),
    ("ZSTD_DISABLE_ASM", "1"),
    ("DYNAMIC_BMI2", "0"),
    ("ZSTDLIB_VISIBLE", ""),
    ("ZSTDLIB_HIDDEN", ""),
    ("ZSTD_DEPS_MALLOC", "1"),
    ("XXH_NO_STDLIB", "1"),
    ("HUF_FORCE_DECOMPRESS_X1", "1"),
    ("ZSTD_FORCE_DECOMPRESS_SEQUENCES_SHORT", "1"),
    ("ZSTD_NO_INLINE", "1"),
];

/// The system libraries the loader imports from: kernel32 (files,
/// processes, the heap, the console), user32 (the failure message box),
/// advapi32 (the token query and the SDDL conversion) and bcrypt (SHA-256).
const LOADER_LIBRARIES: &[&str] = &["kernel32.lib", "user32.lib", "advapi32.lib", "bcrypt.lib"];

/// The static C runtime libraries, named explicitly: an object's
/// `/DEFAULTLIB:LIBCMT` directive names only the first, and the two the
/// first depends on are named by its members, which a link that starts at
/// the loader's own entry point never pulls. The linker keeps only what is
/// referenced: `memcpy`, `memset`, `memmove` and the `/GS` support.
const CRT_LIBRARIES: &[&str] = &["libcmt.lib", "libvcruntime.lib", "libucrt.lib"];

/// Whether the loader starts through the C runtime (`wWinMainCRTStartup`)
/// or at its own entry point, which seeds the `/GS` cookie itself and uses
/// no C runtime service. The entry point is the product: the C runtime's
/// start-up links 92 KB the loader never runs (173,056 against 81,408
/// bytes when measured). The other variant stays one constant away so the
/// comparison can be repeated.
const CRT_STARTUP: bool = false;

fn main() {
    let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR lies three levels below the profile directory")
        .to_path_buf();
    let zstd_include = PathBuf::from(
        env::var_os("DEP_ZSTD_INCLUDE").expect("zstd-sys exports the path of its sources"),
    );
    let repository = version_resource::repository_root();
    let manifest = repository.join("crates/build-support/app.manifest");
    let icon = repository.join(LOADER.icon);
    let loader_source = crate_dir.join("src/loader.c");
    let alloc_header = crate_dir.join("src/zstd_alloc.h");
    let fake_engine_source = crate_dir.join("tests/fake-engine.c");
    assert!(manifest.is_file(), "{} is missing", manifest.display());
    assert!(icon.is_file(), "{} is missing", icon.display());

    // Listing any file switches Cargo from "rerun on any change in the
    // package" to "rerun on these", so everything the script depends on is
    // named.
    version_resource::declare_inputs(&LOADER, Some(&manifest));
    for path in [&loader_source, &alloc_header, &fake_engine_source] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for source in ZSTD_SOURCES {
        println!(
            "cargo:rerun-if-changed={}",
            zstd_include.join(source).display()
        );
    }

    let mut objects = compile_zstd(&zstd_include, &alloc_header, &out_dir);
    objects.push(compile_loader(&loader_source, &zstd_include, &out_dir));
    let resources = compile_resources(&manifest, &out_dir);
    let loader = link_loader(&objects, &resources, &out_dir, &profile_dir);
    println!("cargo:loader={}", loader.display());
    println!("cargo:rustc-env=TIGERSETUP_LOADER_EXE={}", loader.display());

    let fake_engine = build_fake_engine(&fake_engine_source, &out_dir);
    println!(
        "cargo:rustc-env=TIGERSETUP_FAKE_ENGINE_EXE={}",
        fake_engine.display()
    );
}

/// What the loader says about itself: the end-user setup identity and
/// icon, like the engine's.
const LOADER: version_resource::Executable = version_resource::Executable {
    binary: "tigersetup-loader",
    file_description: "TigerSetup installer",
    original_filename: "tigersetup-loader.exe",
    internal_name: "tigersetup-loader",
    icon: version_resource::SETUP_ICON,
};

/// The compiler configuration every C object of the loader shares: C11,
/// the static C runtime, optimized for size, warnings as errors, control
/// flow guard, reproducible.
fn c_build(out_dir: &Path) -> cc::Build {
    let mut build = cc::Build::new();
    build
        .out_dir(out_dir)
        .std("c11")
        .static_crt(true)
        .opt_level_str("s")
        .debug(false)
        .warnings(true)
        .warnings_into_errors(true)
        .flag("/guard:cf")
        .flag("/GS")
        .flag("/Gw")
        .flag("/Oi")
        .cargo_metadata(false)
        .cargo_warnings(true);
    build
}

fn compile_zstd(zstd_include: &Path, alloc_header: &Path, out_dir: &Path) -> Vec<PathBuf> {
    let mut build = c_build(&out_dir.join("zstd"));
    build.include(zstd_include);
    for (name, value) in ZSTD_DEFINES {
        build.define(name, Some(*value));
    }
    build.flag(format!("/FI{}", alloc_header.display()));
    for source in ZSTD_SOURCES {
        build.file(zstd_include.join(source));
    }
    build.compile_intermediates()
}

fn compile_loader(source: &Path, zstd_include: &Path, out_dir: &Path) -> PathBuf {
    let mut build = c_build(&out_dir.join("loader"));
    build.include(zstd_include).file(source);
    if CRT_STARTUP {
        build.define("LOADER_CRT_STARTUP", None);
    }
    let objects = build.compile_intermediates();
    objects
        .into_iter()
        .next()
        .expect("the loader compiles to one object")
}

/// The Win32 resources of the loader: the side-by-side manifest as
/// `RT_MANIFEST 1`, TigerSetup's icon as `ICON 1`, and the version resource
/// naming the executable, from the one script every TigerSetup executable
/// is described by (`build-support/version_resource.rs`). The script is
/// compiled with the Windows SDK's `rc.exe` directly — `embed-resource`
/// only locates it — because its `compile` would also tell Cargo to link
/// the result into every dependent, and this package's resources belong
/// to the loader alone.
fn compile_resources(manifest: &Path, out_dir: &Path) -> PathBuf {
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let script = version_resource::script(&LOADER, Some(manifest), &version);
    let script_path = out_dir.join("tigersetup-loader.rc");
    fs::write(&script_path, script).expect("the resource script is written");
    let compiled = out_dir.join("tigersetup-loader.res");
    let rc = env::var_os("RC")
        .map(PathBuf::from)
        .or_else(|| embed_resource::find_windows_sdk_tool("rc.exe"))
        .expect("the Windows SDK's rc.exe is installed");
    let mut command = Command::new(rc);
    command
        .arg("/nologo")
        .arg(format!("/fo{}", compiled.display()))
        .arg(&script_path);
    run(&mut command, "compiling the loader's resources");
    compiled
}

fn linker() -> Command {
    cc::windows_registry::find_tool("x86_64-pc-windows-msvc", "link.exe")
        .expect("the MSVC linker is installed")
        .to_command()
}

/// Links the loader: a GUI-subsystem, 64-bit, reproducible executable with
/// the modern mitigations, that resolves its imports from System32 only,
/// written to the profile directory beside the other TigerSetup
/// executables. The map file beside the link output records what each
/// object contributed.
fn link_loader(
    objects: &[PathBuf],
    resources: &Path,
    out_dir: &Path,
    profile_dir: &Path,
) -> PathBuf {
    let linked = out_dir.join("tigersetup-loader.exe");
    let map = out_dir.join("tigersetup-loader.map");
    let mut link = linker();
    link.args([
        "/NOLOGO",
        "/SUBSYSTEM:WINDOWS",
        "/MACHINE:X64",
        "/Brepro",
        "/DYNAMICBASE",
        "/HIGHENTROPYVA",
        "/NXCOMPAT",
        "/guard:cf",
        "/OPT:REF",
        "/OPT:ICF",
        "/INCREMENTAL:NO",
        "/MANIFEST:NO",
        "/DEPENDENTLOADFLAG:0x800",
    ]);
    if !CRT_STARTUP {
        link.arg("/ENTRY:LoaderEntry");
    }
    link.arg(format!("/OUT:{}", linked.display()));
    link.arg(format!("/MAP:{}", map.display()));
    link.args(objects);
    link.arg(resources);
    link.args(LOADER_LIBRARIES);
    link.args(CRT_LIBRARIES);
    run(&mut link, "linking the loader");

    // The profile directory is what the builder reads; the exe appears
    // there complete or not at all.
    let target = profile_dir.join("tigersetup-loader.exe");
    let staged = profile_dir.join("tigersetup-loader.exe.linking");
    fs::copy(&linked, &staged).expect("the loader is staged beside its destination");
    fs::rename(&staged, &target).expect("the loader is placed in the profile directory");
    target
}

/// The test fixture: a console program with the ordinary C runtime,
/// compiled into `OUT_DIR` where nothing but this package's tests finds it.
fn build_fake_engine(source: &Path, out_dir: &Path) -> PathBuf {
    let mut build = c_build(&out_dir.join("fake-engine"));
    build.file(source);
    let objects = build.compile_intermediates();
    let exe = out_dir.join("fake-engine.exe");
    let mut link = linker();
    link.args([
        "/NOLOGO",
        "/SUBSYSTEM:CONSOLE",
        "/MACHINE:X64",
        "/Brepro",
        "/INCREMENTAL:NO",
        "/MANIFEST:NO",
    ]);
    link.arg(format!("/OUT:{}", exe.display()));
    link.args(&objects);
    link.args(["kernel32.lib", "advapi32.lib"]);
    run(&mut link, "linking the fake engine");
    exe
}

fn run(command: &mut Command, what: &str) {
    let output = command
        .output()
        .unwrap_or_else(|err| panic!("{what}: cannot run {:?}: {err}", command.get_program()));
    if !output.status.success() {
        panic!(
            "{what} failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
