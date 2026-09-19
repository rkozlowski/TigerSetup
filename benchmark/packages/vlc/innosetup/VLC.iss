; VLC 3.0.23 benchmark package (Inno Setup 7). Functional contract:
; ../contract.md.

#define MyAppName "VLC media player"
#define MyAppVersion "3.0.23"
#define MyAppPublisher "VideoLAN (repackaged for the TigerSetup benchmark)"
#define MyAppExeName "vlc.exe"
#define PayloadDir "..\..\..\canonical\VLC\payload"

[Setup]
AppId={{7A8B9C01-3D4E-4F50-9A1B-2C3D4E5F6A01}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={commonpf}\VideoLAN\VLC
DefaultGroupName=VideoLAN
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=true
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\..\artifacts\vlc
OutputBaseFilename=VLC-InnoSetup

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "contextmenu"; Description: "Add ""Play with VLC"" to the Explorer context menu"; Flags: checkedonce

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\VLC media player"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\VLC media player"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#MyAppExeName}"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExeName}"; Flags: uninsdeletekey

; Representative file-association subset (contract.md): mp3, flac, wav, mp4,
; mkv, avi. Written out per extension -- Inno's preprocessor loops are
; fragile for this shape and six near-identical blocks read more reliably
; than a macro that generates them.
Root: HKLM; Subkey: "Software\Classes\.mp3"; ValueType: string; ValueName: ""; ValueData: "VLC.mp3"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Classes\VLC.mp3"; ValueType: string; ValueName: ""; ValueData: "MP3 audio file"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\VLC.mp3\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "Software\Classes\VLC.mp3\shell\PlayWithVLC"; ValueType: string; ValueName: ""; ValueData: "Play with VLC"; Tasks: contextmenu
Root: HKLM; Subkey: "Software\Classes\VLC.mp3\shell\PlayWithVLC\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

Root: HKLM; Subkey: "Software\Classes\.flac"; ValueType: string; ValueName: ""; ValueData: "VLC.flac"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Classes\VLC.flac"; ValueType: string; ValueName: ""; ValueData: "FLAC audio file"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\VLC.flac\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "Software\Classes\VLC.flac\shell\PlayWithVLC"; ValueType: string; ValueName: ""; ValueData: "Play with VLC"; Tasks: contextmenu
Root: HKLM; Subkey: "Software\Classes\VLC.flac\shell\PlayWithVLC\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

Root: HKLM; Subkey: "Software\Classes\.wav"; ValueType: string; ValueName: ""; ValueData: "VLC.wav"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Classes\VLC.wav"; ValueType: string; ValueName: ""; ValueData: "WAV audio file"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\VLC.wav\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "Software\Classes\VLC.wav\shell\PlayWithVLC"; ValueType: string; ValueName: ""; ValueData: "Play with VLC"; Tasks: contextmenu
Root: HKLM; Subkey: "Software\Classes\VLC.wav\shell\PlayWithVLC\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

Root: HKLM; Subkey: "Software\Classes\.mp4"; ValueType: string; ValueName: ""; ValueData: "VLC.mp4"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Classes\VLC.mp4"; ValueType: string; ValueName: ""; ValueData: "MP4 video file"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\VLC.mp4\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "Software\Classes\VLC.mp4\shell\PlayWithVLC"; ValueType: string; ValueName: ""; ValueData: "Play with VLC"; Tasks: contextmenu
Root: HKLM; Subkey: "Software\Classes\VLC.mp4\shell\PlayWithVLC\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

Root: HKLM; Subkey: "Software\Classes\.mkv"; ValueType: string; ValueName: ""; ValueData: "VLC.mkv"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Classes\VLC.mkv"; ValueType: string; ValueName: ""; ValueData: "Matroska video file"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\VLC.mkv\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "Software\Classes\VLC.mkv\shell\PlayWithVLC"; ValueType: string; ValueName: ""; ValueData: "Play with VLC"; Tasks: contextmenu
Root: HKLM; Subkey: "Software\Classes\VLC.mkv\shell\PlayWithVLC\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

Root: HKLM; Subkey: "Software\Classes\.avi"; ValueType: string; ValueName: ""; ValueData: "VLC.avi"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Classes\VLC.avi"; ValueType: string; ValueName: ""; ValueData: "AVI video file"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\VLC.avi\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "Software\Classes\VLC.avi\shell\PlayWithVLC"; ValueType: string; ValueName: ""; ValueData: "Play with VLC"; Tasks: contextmenu
Root: HKLM; Subkey: "Software\Classes\VLC.avi\shell\PlayWithVLC\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

Root: HKLM; Subkey: "Software\Classes\Directory\Background\shell\PlayWithVLC"; ValueType: string; ValueName: ""; ValueData: "Play with VLC"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\Directory\Background\shell\PlayWithVLC\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%V"""; Tasks: contextmenu
