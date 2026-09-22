# TigerSetupTestLaunch — the launch-after-install package

The synthetic package the launch-after-install lab rows install
(`lab/Invoke-LaunchRows.ps1`, `TigerSetup-Design.md` §11.7). It installs one
program, `bin\TigerSetupTestLaunch.exe`, and a `data\` directory, in either
scope, and declares it as the completion page's launch offer:

```toml
[launch]
executable = "bin/TigerSetupTestLaunch.exe"
arguments = ["--report", "%PROGRAMDATA%\\TigerSetupTestLaunch\\report.json", ..., "--", <nine hostile arguments>]
working_directory = "data"
checked = true
```

The program is the workspace's own `TigerSetupTestLaunch.exe`
(`crates/tigersetup-test-launch`): an ordinary window that never asks for the
foreground itself, and writes down how it was started — its process and
parent, the account and token it runs with (elevated or not, integrity,
whether the Administrators group is enabled), every argument as the C runtime
split them, its working directory, and whether its window reached the
foreground — to `C:\ProgramData\TigerSetupTestLaunch\report.json`. The
arguments after `--` are chosen to break a command line that is joined or
split carelessly; `v%VERSION%` must arrive as `v1.0.0`.

Build it after the release binaries, because a lab row measures the engine
embedded in the installer:

```powershell
cargo build --release
pwsh -File packages\test-launch\Build-Package.ps1 -SkipBuild
# → artifacts\test-launch\TigerSetupTestLaunch-1.0.0-Setup.exe
```

Product `IT-Tiger.TigerSetupTestLaunch`, a test identity that cannot collide
with a real application. `stage\` is generated and not committed.
