; WinMerge 2.16.58(.2) benchmark package (Inno Setup 7). Functional contract:
; ../contract.md. Version uses the three-part "2.16.58" for identity parity
; with the TigerSetup and NSIS benchmark packages.

#define MyAppName "WinMerge"
#define MyAppVersion "2.16.58"
#define MyAppPublisher "WinMerge Team (repackaged for the TigerSetup benchmark)"
#define MyAppExeName "WinMergeU.exe"
#define PayloadDir "..\..\..\canonical\WinMerge\payload"

[Setup]
AppId={{9E2E6F2C-6C6A-4D64-9F02-9C9C6F6B7E02}
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
ChangesAssociations=true
ChangesEnvironment=true
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\..\artifacts\winmerge
OutputBaseFilename=WinMerge-InnoSetup

[Tasks]
Name: "desktopicon"; Description: "Create a desktop icon"; Flags: unchecked
Name: "contextmenu"; Description: "Add WinMerge to the Explorer context menu"
Name: "modifypath"; Description: "Add WinMerge to PATH"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#MyAppExeName}"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExeName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.WinMerge"; ValueType: string; ValueName: ""; ValueData: "WinMerge.Project.File"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\WinMerge.Project.File"; ValueType: string; ValueName: ""; ValueData: "WinMerge Project File"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\WinMerge.Project.File\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""

[Run]
Filename: "{sys}\regsvr32.exe"; Parameters: "/s ""{app}\ShellExtensionX64.dll"""; Flags: runhidden; Tasks: contextmenu

[UninstallRun]
Filename: "{sys}\regsvr32.exe"; Parameters: "/u /s ""{app}\ShellExtensionX64.dll"""; Flags: runhidden; RunOnceId: "UnregisterShellExtension"; Tasks: contextmenu

[Code]
// Inno Setup has no built-in "add to PATH" primitive (unlike TigerSetup's
// typed [[path]] entry); every Inno installer that needs this writes its own
// registry read-modify-write, which is what this does.
const
  EnvKeyUser = 'Environment';
  EnvKeyMachine = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';

procedure ModifyPath(Add: Boolean);
var
  RootKey: Integer;
  SubKey, Path, Dir: string;
begin
  if IsAdminInstallMode then begin
    RootKey := HKLM;
    SubKey := EnvKeyMachine;
  end else begin
    RootKey := HKCU;
    SubKey := EnvKeyUser;
  end;
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(RootKey, SubKey, 'Path', Path) then
    Path := '';
  if Add then begin
    if Pos(';' + Dir + ';', ';' + Path + ';') = 0 then begin
      if (Path <> '') and (Path[Length(Path)] <> ';') then
        Path := Path + ';';
      Path := Path + Dir;
      RegWriteExpandStringValue(RootKey, SubKey, 'Path', Path);
    end;
  end else begin
    StringChangeEx(Path, Dir + ';', '', True);
    StringChangeEx(Path, ';' + Dir, '', True);
    StringChangeEx(Path, Dir, '', True);
    RegWriteExpandStringValue(RootKey, SubKey, 'Path', Path);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and IsTaskSelected('modifypath') then
    ModifyPath(True);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    ModifyPath(False);
end;
