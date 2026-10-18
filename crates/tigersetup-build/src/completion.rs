//! PowerShell tab completion, made at build time (`TigerSetup-Design.md`
//! §5.15): every `[[completion]]` becomes one ordinary packaged file,
//! `completion/<command>.ps1`, and the declarations the engine activates it
//! by.
//!
//! The script comes from one of three sources, and nothing of it is made on
//! the target machine:
//!
//! - `tigercli` — the packaged executable asked for its own script with
//!   `--completion pwsh`, which a TigerCli application built with
//!   `UseShellCompletion()` prints and exits with success; one built without
//!   it rejects the option as unknown, and the build says what is missing;
//! - `command` — `program` (by default the packaged executable) run with
//!   `arguments`, whose output is the script;
//! - `script` — a `.ps1` the developer supplies.
//!
//! A generated script is saved as UTF-8 with a byte-order mark, which both
//! PowerShell editions read as UTF-8 (Windows PowerShell 5.1 reads a file
//! without one as ANSI); its line endings stay as generated. A supplied
//! script is packaged byte for byte — it may be signed — and is refused when
//! 5.1 would misread it: non-ASCII UTF-8 without the mark, or not text.
//!
//! The command must be on PATH for completion to work, so its executable's
//! directory must be a `[[path]]` entry (the manifest checks it), and each
//! such entry gives the completion its predicate: the completion is
//! activated exactly while the command is on PATH. The script file itself
//! follows the executable's own predicate, as every installed file does.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tigersetup_format::metadata::ShellCompletion;

use crate::fileset::ResolvedFile;
use crate::manifest::{CompletionEntry, LoadedManifest, install_relative, predicate_of};
use crate::{BuildError, Result};

/// How long a program may take to print its completion script.
const GENERATION_TIMEOUT: Duration = Duration::from_secs(60);

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// The packaged scripts and the declarations that activate them. The
/// generated files live in a scratch directory that goes away with this.
pub struct Completions {
    pub files: Vec<ResolvedFile>,
    pub declarations: Vec<ShellCompletion>,
    _scratch: Option<tempfile::TempDir>,
}

fn failed(code: &'static str, message: String) -> BuildError {
    BuildError::new(code, message)
}

/// Makes every declared completion's script and declarations, for the files
/// the package installs.
pub fn resolve(loaded: &LoadedManifest, files: &[ResolvedFile]) -> Result<Completions> {
    let manifest = &loaded.manifest;
    let mut out = Completions {
        files: Vec::new(),
        declarations: Vec::new(),
        _scratch: None,
    };
    if manifest.completion.is_empty() {
        return Ok(out);
    }
    let scratch = tempfile::Builder::new()
        .prefix("tigersetup-completion-")
        .tempdir()?;
    for entry in &manifest.completion {
        let command = entry.command_name();
        let executable = install_relative(&entry.executable)?;
        let packaged = files
            .iter()
            .find(|f| f.relative.eq_ignore_ascii_case(&executable))
            .ok_or_else(|| {
                failed(
                    "completion_executable_missing",
                    format!(
                        "completion {command}: {} is not a file the package installs",
                        entry.executable
                    ),
                )
            })?;
        let installed = entry.installed_script();
        if files
            .iter()
            .any(|f| f.relative.eq_ignore_ascii_case(&installed))
        {
            return Err(failed(
                "completion_script_conflict",
                format!(
                    "completion {command} is installed as {installed}, which the package's [[files]] already install"
                ),
            ));
        }
        let bytes = script_bytes(loaded, entry, &command, &packaged.source)?;
        let source = scratch.path().join(format!("{command}.ps1"));
        std::fs::write(&source, &bytes)?;
        out.files.push(ResolvedFile {
            relative: installed.clone(),
            source,
            size: bytes.len() as u64,
            when: packaged.when.clone(),
        });
        // One declaration per PATH entry that puts the command's directory on
        // PATH, each under that entry's predicate.
        let directory = executable.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for path in &manifest.path {
            if install_relative(&path.entry)?.eq_ignore_ascii_case(directory) {
                out.declarations.push(ShellCompletion {
                    command: command.clone(),
                    script: installed.clone(),
                    when: predicate_of(path.when.as_ref(), path.option.as_ref()),
                });
            }
        }
    }
    out._scratch = Some(scratch);
    Ok(out)
}

/// The script's bytes as the package installs them.
fn script_bytes(
    loaded: &LoadedManifest,
    entry: &CompletionEntry,
    command: &str,
    executable: &Path,
) -> Result<Vec<u8>> {
    match entry.source.as_str() {
        "script" => {
            let relative = entry.script.as_deref().unwrap_or_default();
            let path = loaded.resolve(relative);
            let bytes = std::fs::read(&path).map_err(|err| {
                failed(
                    "completion_script_unreadable",
                    format!(
                        "completion {command}: cannot read {}: {err}",
                        path.display()
                    ),
                )
            })?;
            check_supplied(command, &path, &bytes)?;
            Ok(bytes)
        }
        "tigercli" => {
            let output = run(command, executable, &["--completion".into(), "pwsh".into()])?;
            if output.code != Some(0) {
                return Err(failed(
                    "completion_unavailable",
                    format!(
                        "completion {command}: {} --completion pwsh exited {}: a TigerCli application prints its script only when it calls UseShellCompletion(). {}",
                        executable.display(),
                        output
                            .code
                            .map_or("without a code".to_string(), |c| c.to_string()),
                        output.stderr.trim()
                    ),
                ));
            }
            let script = generated_text(command, executable, &output.stdout)?;
            if !script.contains("Register-ArgumentCompleter") {
                return Err(failed(
                    "completion_unavailable",
                    format!(
                        "completion {command}: {} --completion pwsh printed no PowerShell argument completer",
                        executable.display()
                    ),
                ));
            }
            Ok(with_bom(&script))
        }
        _ => {
            let program = match &entry.program {
                Some(program) => loaded.resolve(program),
                None => executable.to_path_buf(),
            };
            let output = run(command, &program, &entry.arguments)?;
            if output.code != Some(0) {
                return Err(failed(
                    "completion_generation_failed",
                    format!(
                        "completion {command}: {} exited {}: {}",
                        program.display(),
                        output
                            .code
                            .map_or("without a code".to_string(), |c| c.to_string()),
                        output.stderr.trim()
                    ),
                ));
            }
            Ok(with_bom(&generated_text(
                command,
                &program,
                &output.stdout,
            )?))
        }
    }
}

/// A generated script as text: UTF-8, its own byte-order mark dropped, and
/// not empty.
fn generated_text(command: &str, program: &Path, stdout: &[u8]) -> Result<String> {
    let bytes = stdout.strip_prefix(&UTF8_BOM).unwrap_or(stdout);
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
        failed(
            "completion_script_invalid",
            format!(
                "completion {command}: {} printed a script that is not UTF-8",
                program.display()
            ),
        )
    })?;
    if text.trim().is_empty() {
        return Err(failed(
            "completion_script_invalid",
            format!(
                "completion {command}: {} printed no script",
                program.display()
            ),
        ));
    }
    Ok(text)
}

fn with_bom(text: &str) -> Vec<u8> {
    [&UTF8_BOM[..], text.as_bytes()].concat()
}

/// A supplied script must be text both editions read the same: UTF-16 or
/// UTF-8 with a byte-order mark, or plain ASCII.
fn check_supplied(command: &str, path: &Path, bytes: &[u8]) -> Result<()> {
    let refuse = |why: &str| {
        Err(failed(
            "completion_script_invalid",
            format!("completion {command}: {} {why}", path.display()),
        ))
    };
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Ok(());
    }
    let body = bytes.strip_prefix(&UTF8_BOM).unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(body) else {
        return refuse("is not UTF-8 or UTF-16 text");
    };
    if text.trim().is_empty() {
        return refuse("is empty");
    }
    if !bytes.starts_with(&UTF8_BOM) && !text.is_ascii() {
        return refuse(
            "is UTF-8 without a byte-order mark and not ASCII; Windows PowerShell 5.1 would read it as ANSI. Save it as UTF-8 with a byte-order mark",
        );
    }
    Ok(())
}

struct Output {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

/// Runs a generator with no input, in the directory of the program, and
/// waits for it at most [`GENERATION_TIMEOUT`].
fn run(command: &str, program: &Path, arguments: &[String]) -> Result<Output> {
    let mut child = Command::new(program)
        .args(arguments)
        .current_dir(program.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            failed(
                "completion_generation_failed",
                format!(
                    "completion {command}: cannot run {}: {err}",
                    program.display()
                ),
            )
        })?;
    let mut stdout_pipe = child.stdout.take().expect("piped");
    let mut stderr_pipe = child.stderr.take().expect("piped");
    let stdout = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut bytes);
        bytes
    });
    let stderr = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + GENERATION_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failed(
                "completion_generation_failed",
                format!(
                    "completion {command}: {} did not finish within {} seconds",
                    program.display(),
                    GENERATION_TIMEOUT.as_secs()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Output {
        code: status.code(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A manifest with one command on PATH under two values of a choice
    /// option, and `completion` as its completion section.
    fn loaded(dir: &Path, completion: &str) -> LoadedManifest {
        let text = format!(
            r#"[package]
id = "Vendor.Tool"
name = "Tool"
version = "1.0.0"
publisher = "Vendor"

[[files]]
source = "payload/**"

[[options]]
name = "path-mode"
kind = "choice"
default = "command"
label = {{ "en-US" = "PATH" }}
choices = [
  {{ value = "none", label = {{ "en-US" = "None" }} }},
  {{ value = "command", label = {{ "en-US" = "Command" }} }},
  {{ value = "tools", label = {{ "en-US" = "Tools" }} }},
]

[[path]]
entry = "bin"
when = {{ option = "path-mode", equals = "command" }}

[[path]]
entry = "bin"
when = {{ option = "path-mode", equals = "tools" }}

[[completion]]
executable = "bin/tool.exe"
{completion}
"#
        );
        std::fs::write(dir.join("TigerSetup.toml"), text).unwrap();
        LoadedManifest::load(&dir.join("TigerSetup.toml")).unwrap()
    }

    fn packaged(dir: &Path, source: &Path) -> Vec<ResolvedFile> {
        let _ = dir;
        vec![ResolvedFile {
            relative: "bin/tool.exe".into(),
            source: source.to_path_buf(),
            size: 1,
            when: None,
        }]
    }

    #[test]
    fn a_supplied_script_is_packaged_as_it_is_under_each_path_predicate() {
        let dir = tempfile::tempdir().unwrap();
        let script =
            "Register-ArgumentCompleter -Native -CommandName tool -ScriptBlock { 'a' }\r\n";
        std::fs::write(dir.path().join("tool.ps1"), script).unwrap();
        let loaded = loaded(dir.path(), "source = \"script\"\nscript = \"tool.ps1\"");
        let made = resolve(&loaded, &packaged(dir.path(), &dir.path().join("x"))).unwrap();
        assert_eq!(made.files.len(), 1);
        assert_eq!(made.files[0].relative, "completion/tool.ps1");
        assert_eq!(
            std::fs::read(&made.files[0].source).unwrap(),
            script.as_bytes()
        );
        let predicates: Vec<String> = made
            .declarations
            .iter()
            .map(|d| d.when.as_ref().unwrap().equals.clone())
            .collect();
        assert_eq!(predicates, vec!["command", "tools"]);
        assert!(made.declarations.iter().all(|d| d.command == "tool"));
    }

    #[test]
    fn a_supplied_script_windows_powershell_would_misread_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool.ps1"), "# zażółć\r\n").unwrap();
        let loaded = loaded(dir.path(), "source = \"script\"\nscript = \"tool.ps1\"");
        let err = resolve(&loaded, &packaged(dir.path(), &dir.path().join("x")))
            .err()
            .unwrap();
        assert_eq!(err.code, "completion_script_invalid");
        // With the mark it is fine, and kept as it is.
        std::fs::write(
            dir.path().join("tool.ps1"),
            [&UTF8_BOM[..], "# zażółć\r\n".as_bytes()].concat(),
        )
        .unwrap();
        assert!(resolve(&loaded, &packaged(dir.path(), &dir.path().join("x"))).is_ok());
    }

    #[test]
    fn a_generated_script_gets_a_byte_order_mark() {
        let dir = tempfile::tempdir().unwrap();
        // A program that prints its script: cmd.exe stands in for one.
        let cmd = PathBuf::from(std::env::var("ComSpec").unwrap());
        std::fs::write(dir.path().join("gen.cmd"), "@echo # generated for %1\r\n").unwrap();
        let loaded = loaded(
            dir.path(),
            "source = \"command\"\nprogram = \"gen.cmd\"\narguments = [\"tool\"]",
        );
        let made = resolve(&loaded, &packaged(dir.path(), &cmd)).unwrap();
        let bytes = std::fs::read(&made.files[0].source).unwrap();
        assert!(bytes.starts_with(&UTF8_BOM));
        assert_eq!(&bytes[3..], b"# generated for tool\r\n");
        // A generator that fails fails the build, with its error.
        std::fs::write(
            dir.path().join("gen.cmd"),
            "@echo broken 1>&2\r\n@exit /b 3\r\n",
        )
        .unwrap();
        let err = resolve(&loaded, &packaged(dir.path(), &cmd)).err().unwrap();
        assert_eq!(err.code, "completion_generation_failed");
        assert!(err.message.contains("broken"), "{}", err.message);
    }

    #[test]
    fn an_application_without_tigercli_completion_is_named() {
        let dir = tempfile::tempdir().unwrap();
        // cmd.exe given `--completion pwsh` does not print a completer.
        let cmd = PathBuf::from(std::env::var("ComSpec").unwrap());
        let loaded = loaded(dir.path(), "source = \"tigercli\"");
        let err = resolve(&loaded, &packaged(dir.path(), &cmd)).err().unwrap();
        assert_eq!(err.code, "completion_unavailable");
    }

    #[test]
    fn the_command_and_its_script_come_from_the_executable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool.ps1"), "# x\r\n").unwrap();
        let loaded = loaded(dir.path(), "source = \"script\"\nscript = \"tool.ps1\"");
        let entry = &loaded.manifest.completion[0];
        assert_eq!(entry.command_name(), "tool");
        assert_eq!(entry.installed_script(), "completion/tool.ps1");
        // An executable the package does not install has nothing to complete.
        let err = resolve(&loaded, &[]).err().unwrap();
        assert_eq!(err.code, "completion_executable_missing");
    }
}
