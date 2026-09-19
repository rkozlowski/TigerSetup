; qBittorrent 5.2.3 benchmark package (Inno Setup 7). Functional contract:
; ../contract.md.

#define MyAppName "qBittorrent"
#define MyAppVersion "5.2.3"
#define MyAppPublisher "qBittorrent Team (repackaged for the TigerSetup benchmark)"
#define MyAppExeName "qbittorrent.exe"
#define PayloadDir "..\..\..\canonical\qBittorrent\payload"

[Setup]
AppId={{6C6A6E01-9E2E-4D64-9F02-1B2C3D4E5F01}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={commonpf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=true
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\..\artifacts\qbittorrent
OutputBaseFilename=qBittorrent-InnoSetup

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "startup"; Description: "Start qBittorrent on Windows startup"; Flags: unchecked
Name: "firewall"; Description: "Add a firewall exception for qBittorrent"; Flags: checkedonce
Name: "longpaths"; Description: "Disable the Windows path length limit"; Flags: checkedonce

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon
Name: "{commonstartup}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: startup

[Registry]
Root: HKLM; Subkey: "Software\Classes\.torrent"; ValueType: string; ValueName: ""; ValueData: "qBittorrent.File.Torrent"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Classes\qBittorrent.File.Torrent"; ValueType: string; ValueName: ""; ValueData: "BitTorrent file"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\qBittorrent.File.Torrent\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "Software\Classes\magnet"; ValueType: string; ValueName: ""; ValueData: "URL:Magnet URI"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\magnet"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKLM; Subkey: "Software\Classes\magnet\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\FileSystem"; ValueType: dword; ValueName: "LongPathsEnabled"; ValueData: "1"; Tasks: longpaths

[Run]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""qBittorrent"" dir=in action=allow program=""{app}\{#MyAppExeName}"" enable=yes"; Flags: runhidden; Tasks: firewall

[UninstallRun]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""qBittorrent"""; Flags: runhidden; RunOnceId: "RemoveFirewallRule"; Tasks: firewall
