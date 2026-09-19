; Minimal NSIS 3.12 installer: identity/metadata only, no application
; payload, no artificial filler. Used to measure fixed installer overhead
; alongside the raw NSIS exehead stub.
;
; Compression matches the benchmark's production-quality setting for all
; three technologies (see benchmark/README.md): LZMA, solid.

Unicode true
Name "BenchmarkMinimal"
OutFile "..\..\..\artifacts\minimal\Minimal-NSIS.exe"
InstallDir "$LOCALAPPDATA\Programs\BenchmarkMinimal"
RequestExecutionLevel user
SetCompressor /SOLID lzma

VIProductVersion "1.0.0.0"
VIAddVersionKey "ProductName" "BenchmarkMinimal"
VIAddVersionKey "ProductVersion" "1.0.0"
VIAddVersionKey "CompanyName" "TigerSetup Benchmark"
VIAddVersionKey "FileVersion" "1.0.0.0"

Section "Install"
SectionEnd
