; Minimal Inno Setup 7 installer: identity/metadata only, no application
; payload, no artificial filler. Used to measure fixed installer overhead
; alongside the raw Setup.e64 engine.
;
; Compression matches the benchmark's production-quality setting for all
; three technologies (see benchmark/README.md): LZMA2, "max" level, solid.

#define MyAppName "BenchmarkMinimal"
#define MyAppVersion "1.0.0"
#define MyAppPublisher "TigerSetup Benchmark"

[Setup]
AppId={{B3B7B6A0-6E7B-4E0B-9C10-0B8B7C0F4A11}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\..\artifacts\minimal
OutputBaseFilename=Minimal-InnoSetup
