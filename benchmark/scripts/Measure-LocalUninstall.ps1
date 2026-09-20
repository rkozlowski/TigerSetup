#Requires -Version 7.0
<#
    .SYNOPSIS
    Times install and uninstall of synthetic payloads on this machine, for
    profiling the engine's transaction cost without a lab.

    .DESCRIPTION
    Builds one package per payload shape from generated files, installs it
    per-user with --quiet, times the install, uninstalls it from the state
    directory's uninstaller (as Add/Remove Programs does), times the
    uninstall, and prints both with the per-operation event timings the
    engine log records. The shapes mirror the applications the
    installer-technology benchmark measured: many small files (ShareX-like),
    few large files (qBittorrent-like), and a small application.

    Local wall clock is an engineering measurement for comparing two engines
    on one machine, not the benchmark's number: the benchmark measures the
    same thing on the lab's clean VM (benchmark/README.md).

    .EXAMPLE
    pwsh -File benchmark\scripts\Measure-LocalUninstall.ps1 -Label before
#>
[CmdletBinding()]
param(
    # Names this measurement in the output.
    [string] $Label = 'run',
    # The release directory holding tiger-setup.exe, tigersetup-setup.exe and
    # tigersetup-loader.exe.
    [string] $ReleaseDirectory,
    # Where payloads, packages and results go; reused across runs so that
    # the generated payloads are built once.
    [string] $WorkRoot,
    # Which shapes to measure.
    [string[]] $Shapes = @('many-small', 'few-large', 'small-app'),
    # Repetitions per shape; the median is reported.
    [int] $Repeat = 3
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if ([string]::IsNullOrWhiteSpace($ReleaseDirectory)) {
    $ReleaseDirectory = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release'
}
if ([string]::IsNullOrWhiteSpace($WorkRoot)) {
    $WorkRoot = Join-Path $repoRoot 'target\local-uninstall'
}
$builder = Join-Path $ReleaseDirectory 'tiger-setup.exe'
if (-not (Test-Path -LiteralPath $builder)) { throw "$builder is missing; cargo build --release first." }

# (file count, bytes per file) per shape; deterministic pseudo-random
# content so that the payload compresses like real binaries do not.
$definitions = @{
    'many-small' = @{ files = 1200; size = 450KB; dirs = 24 }
    'few-large'  = @{ files = 38; size = 6MB; dirs = 4 }
    'small-app'  = @{ files = 60; size = 120KB; dirs = 6 }
}

function New-Payload {
    param([string] $Root, [hashtable] $Definition)
    if (Test-Path -LiteralPath $Root) { return }
    $null = New-Item -ItemType Directory -Path $Root -Force
    $random = [System.Random]::new(20260920)
    $buffer = [byte[]]::new($Definition.size)
    for ($i = 0; $i -lt $Definition.files; $i++) {
        $dir = Join-Path $Root ("d{0:00}" -f ($i % $Definition.dirs))
        if (-not (Test-Path -LiteralPath $dir)) { $null = New-Item -ItemType Directory -Path $dir }
        $random.NextBytes($buffer)
        [System.IO.File]::WriteAllBytes((Join-Path $dir ("f{0:0000}.bin" -f $i)), $buffer)
    }
}

function Get-LogSpanSeconds {
    # The engine's own span: first to last timestamp of its log, which
    # excludes the loader's extraction, the harness and the shell.
    param([string] $Path)
    $lines = @(Get-Content -LiteralPath $Path)
    if ($lines.Count -lt 2) { return 0 }
    $first = [datetime]::Parse(($lines[0] -split ' ')[0], $null, 'RoundtripKind')
    $last = [datetime]::Parse(($lines[-1] -split ' ')[0], $null, 'RoundtripKind')
    return ($last - $first).TotalSeconds
}

function Get-Median {
    param([double[]] $Values)
    $sorted = @($Values | Sort-Object)
    if ($sorted.Count -eq 0) { return 0 }
    return $sorted[[int][math]::Floor(($sorted.Count - 1) / 2)]
}

$results = @()
foreach ($shape in $Shapes) {
    $definition = $definitions[$shape]
    $shapeRoot = Join-Path $WorkRoot $shape
    $payload = Join-Path $shapeRoot 'payload'
    New-Payload -Root $payload -Definition $definition
    $manifest = Join-Path $shapeRoot 'TigerSetup.toml'
    $productId = "ItTiger.Profile$($shape -replace '[^a-z]', '')"
    $productName = "Profile-$shape"
    @"
[package]
id = "$productId"
name = "$productName"
version = "1.0.0"
publisher = "IT Tiger"

[install]
scopes = ["user"]

[[files]]
source = "payload/**"
"@ | Set-Content -LiteralPath $manifest -Encoding UTF8
    $out = Join-Path $shapeRoot "out-$Label"
    if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Recurse -Force }
    & $builder build $manifest --output $out --fast | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "build failed for $shape" }
    $installer = Get-ChildItem -LiteralPath $out -Filter '*.exe' | Select-Object -First 1

    $installTimes = @(); $uninstallTimes = @(); $installLog = ''; $uninstallLog = ''
    $installSpans = @(); $uninstallSpans = @()
    for ($r = 0; $r -lt $Repeat; $r++) {
        $installLog = Join-Path $shapeRoot "install-$Label-$r.log"
        $uninstallLog = Join-Path $shapeRoot "uninstall-$Label-$r.log"
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $installer.FullName install --quiet --log $installLog | Out-Null
        $sw.Stop()
        if ($LASTEXITCODE -ne 0) { throw "install failed for $shape ($LASTEXITCODE)" }
        $installTimes += $sw.Elapsed.TotalSeconds
        $installSpans += Get-LogSpanSeconds -Path $installLog
        $uninstaller = Join-Path $env:LOCALAPPDATA "TigerSetup\$productId\uninstall.exe"
        if (-not (Test-Path -LiteralPath $uninstaller)) { throw "no uninstaller for $shape" }
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $uninstaller uninstall --quiet --log $uninstallLog | Out-Null
        $sw.Stop()
        if ($LASTEXITCODE -ne 0) { throw "uninstall failed for $shape ($LASTEXITCODE)" }
        $uninstallTimes += $sw.Elapsed.TotalSeconds
        $uninstallSpans += Get-LogSpanSeconds -Path $uninstallLog
    }
    $results += [pscustomobject]@{
        label = $Label
        shape = $shape
        files = $definition.files
        bytes = $definition.files * $definition.size
        installer_bytes = $installer.Length
        install_s = [math]::Round((Get-Median $installTimes), 2)
        install_engine_s = [math]::Round((Get-Median $installSpans), 2)
        uninstall_s = [math]::Round((Get-Median $uninstallTimes), 2)
        uninstall_engine_s = [math]::Round((Get-Median $uninstallSpans), 2)
        install_log = $installLog
        uninstall_log = $uninstallLog
    }
}
$results | Format-Table -AutoSize
$results | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $WorkRoot "results-$Label.json") -Encoding UTF8
