# TigerSetupTestCompletion — the PowerShell completion package

The synthetic package the PowerShell completion lab rows install
(`lab/Invoke-CompletionRows.ps1`, `TigerSetup-Design.md` §5.15). It installs
one command, `bin\TigerSetupTestAction.exe`, in either scope, puts `bin` on
PATH behind the `path` option (on by default), and declares its completion as
a TigerCli application's:

```toml
[[completion]]
executable = "bin/TigerSetupTestAction.exe"
source = "tigercli"
```

The program is the workspace's own `TigerSetupTestAction.exe`
(`crates/tigersetup-test-action`), whose `--completion pwsh` prints a native
completer offering `alpha`, `beta` and `gamma` — what a TigerCli application
built with `UseShellCompletion()` prints. `tiger-setup build` runs it once and
packages the result as `completion\TigerSetupTestAction.ps1`.

Build it after the release binaries, because a lab row measures the engine
embedded in the installer:

```powershell
cargo build --release
pwsh -File packages\test-completion\Build-Package.ps1 -SkipBuild
# → artifacts\test-completion\TigerSetupTestCompletion-1.0.0-Setup.exe
```

Product `IT-Tiger.TigerSetupTestCompletion`, a test identity that cannot
collide with a real application. `stage\` is generated and not committed.
