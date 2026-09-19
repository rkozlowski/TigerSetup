; ShareX 21.0.0 benchmark package (Inno Setup 7). Functional contract:
; ../contract.md. Payload is the canonical ShareX payload, referenced
; relative to this script.
;
; Compression: LZMA2/max, solid -- the benchmark's production-quality
; setting for all three technologies (see benchmark/README.md).

#define MyAppName "ShareX"
#define MyAppVersion "21.0.0"
#define MyAppPublisher "ShareX Team (repackaged for the TigerSetup benchmark)"
#define MyAppExeName "ShareX.exe"
#define PayloadDir "..\..\..\canonical\ShareX\payload"

[Setup]
AppId={{2B7B6B10-6E29-4B7C-9B36-7C0B1E9A2A01}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\..\artifacts\sharex
OutputBaseFilename=ShareX-InnoSetup

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "sendto"; Description: "Add a Send To entry"; Flags: unchecked
Name: "contextmenu"; Description: "Add ""Upload with ShareX"" to the Explorer context menu"; Flags: unchecked
Name: "startup"; Description: "Start ShareX at sign-in"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon
Name: "{userappdata}\Microsoft\Windows\SendTo\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: sendto
Name: "{userstartup}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Parameters: "-silent"; Tasks: startup

[Registry]
; "Upload with ShareX" on any file, matching the classic Explorer verb the
; real installer offers as an optional task.
Root: HKA; Subkey: "Software\Classes\*\shell\ShareX"; ValueType: string; ValueName: ""; ValueData: "Upload with ShareX"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\*\shell\ShareX"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\ShareX\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" -workflow ""%1"""; Tasks: contextmenu
