# TigerSetup Help

TigerSetup is a command-line tool for Windows that builds installers. You
describe what your application consists of in a small text file,
`TigerSetup.toml`, and `tiger-setup build` turns it into one self-contained
`Setup.exe`. That `Setup.exe` installs, upgrades, repairs and uninstalls your
application, with a wizard for people and a quiet mode for scripts, and it
needs nothing else on the target machine.

TigerSetup has no window of its own: you use it from a command prompt.

## Start TigerSetup

Open **Start → TigerSetup → TigerSetup Shell**.

A command prompt opens in your user folder, shows a short getting-started
summary (`tiger-setup --help-brief`), and waits for your commands. In that
window `tiger-setup` runs the TigerSetup you opened it from — it comes first
on that window's `PATH` — whether or not the installer added TigerSetup to
your `PATH`. The window changes nothing on your system; close it with `exit`
when you are done.

If you kept the installer's **Add TigerSetup to PATH** option (it is on by
default), `tiger-setup` also works in every new command prompt, PowerShell
window and terminal.

TigerSetup is installed in:

| Installed for | Folder |
|---|---|
| you only (the default) | `%LOCALAPPDATA%\Programs\TigerSetup` |
| everyone on the computer | `%ProgramFiles%\TigerSetup` |

The folder holds `tiger-setup.exe`, the tool you run, and the two files it
builds installers from — `tigersetup-setup.exe`, the installer engine, and
`tigersetup-loader.exe`, the loader every `Setup.exe` starts with. Keep the
three together. This help is in its `help` folder: **Start → TigerSetup →
TigerSetup Help** opens it as PDF, and **TigerSetup Help (Markdown)** opens
the Markdown it is made from.

## Commands

| Command | What it does |
|---|---|
| `tiger-setup build TigerSetup.toml` | build `<name>-<version>-Setup.exe` from a manifest |
| `tiger-setup build TigerSetup.toml --fast` | the same, quicker and larger — for trying things out |
| `tiger-setup inspect MyApp-1.0.0-Setup.exe` | show what an installer contains and claims, without running it |
| `tiger-setup verify MyApp-1.0.0-Setup.exe` | check an installer's hashes; exit code `0` means intact |
| `tiger-setup metadata TigerSetup.toml` | show the product name, version and publisher a build would use, and where each came from |
| `tiger-setup winget prepare` / `finalize` | write the WinGet manifests for a published installer |
| `tiger-setup --help` | every command, and the options common to them |
| `tiger-setup build --help` | every option of one command — `build` here |
| `tiger-setup --version` | the TigerSetup version |

## Build your first installer

In the TigerSetup Shell, make a folder with something to install:

```bat
mkdir %USERPROFILE%\Hello
cd /d %USERPROFILE%\Hello
mkdir publish
echo Hello from my first installer> publish\readme.txt
notepad TigerSetup.toml
```

Put this in `TigerSetup.toml` and save it:

```toml
[package]
id = "Contoso.Hello"        # stable identity: the Add/Remove Programs key and the WinGet id
name = "Hello"
version = "1.0.0"
publisher = "Contoso"

[[files]]
source = "publish/**"       # every file under publish\, installed with the same layout
```

Build it, look inside, and run it:

```bat
tiger-setup build TigerSetup.toml
tiger-setup inspect Hello-1.0.0-Setup.exe
Hello-1.0.0-Setup.exe
```

The last line opens the installer's wizard. It installs `readme.txt` into
`%LOCALAPPDATA%\Programs\Hello` and registers **Hello** in **Settings → Apps →
Installed apps**, where you can uninstall it again.

## The manifest

`TigerSetup.toml` describes one product. Only `[package]` and `[[files]]` are
required; everything else is added when you need it:

```toml
[package]
id = "Contoso.MyApp"
name = "MyApp"
version = "1.0.0"
publisher = "Contoso"
description = "Does the thing."
license_file = "publish/LICENSE.txt"   # shown on the wizard's licence page
icon = "assets/MyApp.ico"

[install]
scopes = ["user", "machine"]           # who it is installed for; the first is the default

[[files]]
source = "publish/**"
exclude = ["*.pdb"]

[[shortcuts]]
location = "start-menu"                # or "desktop", "startup", "send-to"
target = "MyApp.exe"                   # a file the package installs

[[options]]
name = "path"                          # a check box in the wizard,
kind = "path"                          # --option path on|off on the command line
default = true

[[path]]
entry = "."                            # add the install folder to PATH
option = "path"                        # ... when the option is on
```

Paths in the manifest are relative to the manifest's own folder; `target`,
`entry` and other installed paths are relative to the installation folder.
TigerSetup can also set registry values and environment variables, register
file types and URL protocols, add firewall rules, install prerequisites such
as the .NET Desktop Runtime or WebView2, run your own program at install
time, offer to start the application when setup finishes, and replace an
older Inno Setup installation — each one more section in `TigerSetup.toml`.

## Inspect and verify

`tiger-setup inspect` reads an installer without running it: the product and
version, the files and their hashes, the engine that will run, the shortcuts
and registration it will create. `--json` gives the same as one
machine-readable document.

`tiger-setup verify` is the yes/no form for a build pipeline: exit code `0`
when every hash matches, `1` when the file is damaged, `2` when it is not a
TigerSetup installer.

## Installing for one user or for everyone

A generated `Setup.exe` installs either for the current user or for
everyone on the computer — whichever the manifest allows, and the first one
it lists by default.

| | for the current user | for everyone |
|---|---|---|
| Folder | `%LOCALAPPDATA%\Programs\<name>` | `%ProgramFiles%\<name>` |
| Needs an administrator | no | yes — Windows shows its UAC prompt |
| Start Menu, `PATH`, registration | the user's own | shared by all users |

The wizard asks which one and puts the UAC shield on **Next** when the choice
needs an administrator. On the command line, `--scope user` or `--scope
machine` chooses. Running `Setup.exe` again later upgrades or repairs what is
already installed.

## Quiet and unattended use

Every generated installer takes the same commands:

```bat
MyApp-1.0.0-Setup.exe install --quiet                  :: install, or upgrade what is there
MyApp-1.0.0-Setup.exe install --quiet --scope machine  :: for everyone (needs an administrator)
MyApp-1.0.0-Setup.exe install --quiet --option path off
MyApp-1.0.0-Setup.exe uninstall --quiet
MyApp-1.0.0-Setup.exe verify --json                    :: is the installation intact?
```

`--quiet` shows no window and waits for nobody. `--json` prints one
machine-readable result, `--log <file>` chooses where the log goes. The exit
code says what happened: `0` success, `1` failed and rolled back, `2` invalid
arguments, `3` a prerequisite is missing, `4` administrator rights required or
refused, `5` cancelled, `6` an application would not close, `7` an earlier
interrupted run could not be recovered (the log says why), `8` unsupported
Windows, `3010` success but Windows must restart.

An install or upgrade either finishes completely or is rolled back, even if
the computer loses power in the middle: the next run completes it or rolls it
back, and never leaves a mixture of two versions.

## Publishing

Build once and publish exactly the file you tested. For WinGet, generate the
manifests from that file:

```bat
tiger-setup winget prepare TigerSetup.toml --installer MyApp-1.0.0-Setup.exe --output winget
tiger-setup winget finalize winget --url https://example.com/MyApp-1.0.0-Setup.exe --installer MyApp-1.0.0-Setup.exe
```

A generated installer runs on Windows 10 version 1809 or later, Windows 11
and Windows Server 2019 or later, 64-bit. It does not need .NET, PowerShell 7,
WinGet or an Internet connection unless the manifest declares a prerequisite
that has to be downloaded.

## Uninstalling TigerSetup

Open **Settings → Apps → Installed apps**, find **TigerSetup**, and choose
**Uninstall**. If you installed it with WinGet, `winget uninstall
ItTiger.TigerSetup` does the same. Installers you built with TigerSetup keep
working without it.

## More help

- `tiger-setup <command> --help` lists every option of a command.
- IT Tiger, the publisher: <https://www.ittiger.net/>
- TigerSetup is released under the MIT License.
