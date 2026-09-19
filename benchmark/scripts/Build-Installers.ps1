#Requires -Version 7.0
<#
    .SYNOPSIS
    Builds the benchmark's 15 installers — the minimal package and the four
    applications, each with TigerSetup, Inno Setup and NSIS — from the
    package definitions in benchmark/packages, records every build's
    wall-clock time, and measures every installer (bytes, SHA-256) into
    benchmark/results.

    .DESCRIPTION
    The TigerSetup installers are built with the release builder beside the
    release engine (target\x86_64-pc-windows-msvc\release), at release
    quality — never --fast, whose bytes are not the ones a benchmark measures.
    Inno Setup and NSIS are located through their winget installs (the
    default paths) or -InnoSetupCompiler / -NsisCompiler.

    Every build starts from an emptied artifacts directory, so the measured
    set is exactly what the definitions in the repository produce; the
    engine identity the TigerSetup installers carry is recorded beside the
    builder's version so a measurement can never be attributed to the wrong
    engine (results/toolchain.json).

    .EXAMPLE
    pwsh -File benchmark\scripts\Build-Installers.ps1
    pwsh -File benchmark\scripts\Build-Installers.ps1 -Only minimal,qbittorrent
#>
[CmdletBinding()]
param(
    [string[]] $Only,
    [string] $BuilderPath,
    [string] $InnoSetupCompiler = 'C:\Program Files\Inno Setup 7\ISCC.exe',
    [string] $NsisCompiler = 'C:\Program Files (x86)\NSIS\makensis.exe'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$benchmarkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $benchmarkRoot '..')).Path
$packagesRoot = Join-Path $benchmarkRoot 'packages'
$artifactsRoot = Join-Path $benchmarkRoot 'artifacts'
$resultsRoot = Join-Path $benchmarkRoot 'results'
$null = New-Item -ItemType Directory -Path $resultsRoot -Force

if ([string]::IsNullOrWhiteSpace($BuilderPath)) {
    $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe'
}
foreach ($tool in @($BuilderPath, $InnoSetupCompiler, $NsisCompiler)) {
    if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw "Compiler '$tool' does not exist." }
}
$enginePath = Join-Path (Split-Path -Parent $BuilderPath) 'tigersetup-setup.exe'
if (-not (Test-Path -LiteralPath $enginePath -PathType Leaf)) { throw "The engine '$enginePath' is not beside the builder." }

# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Only = @($Only | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim().ToLowerInvariant() } | Where-Object { $_ })

# App directory name -> (installer base name, artifacts directory, result file).
$packages = @(
    [pscustomobject]@{ dir = 'minimal';     base = 'Minimal';     results = 'minimal-installers.json' }
    [pscustomobject]@{ dir = 'sharex';      base = 'ShareX';      results = 'sharex-installers.json' }
    [pscustomobject]@{ dir = 'winmerge';    base = 'WinMerge';    results = 'winmerge-installers.json' }
    [pscustomobject]@{ dir = 'qbittorrent'; base = 'qBittorrent'; results = 'qbittorrent-installers.json' }
    [pscustomobject]@{ dir = 'vlc';         base = 'VLC';         results = 'vlc-installers.json' }
)
if ($Only.Count -gt 0) { $packages = @($packages | Where-Object { $_.dir -in $Only }) }

function Invoke-Timed {
    <# Runs one compiler, fails on a non-zero exit, and returns the wall-clock seconds. #>
    param([string] $Label, [string] $Executable, [string[]] $Arguments, [string] $LogPath)
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $output = & $Executable @Arguments 2>&1 | Out-String
    $stopwatch.Stop()
    [System.IO.File]::WriteAllText($LogPath, $output, [System.Text.UTF8Encoding]::new($false))
    if ($LASTEXITCODE -ne 0) { throw "$Label failed (exit $LASTEXITCODE); see $LogPath" }
    Write-Host ("  {0,-28} {1,8:N1} s" -f $Label, $stopwatch.Elapsed.TotalSeconds)
    [math]::Round($stopwatch.Elapsed.TotalSeconds, 1)
}

$builds = [System.Collections.Generic.List[object]]::new()
$builderVersion = (& $BuilderPath --version 2>&1 | Out-String).Trim()
Write-Host "Builder: $builderVersion ($BuilderPath)"
Write-Host "Engine:  $((Get-FileHash -LiteralPath $enginePath -Algorithm SHA256).Hash.ToLowerInvariant()) ($enginePath)"

foreach ($package in $packages) {
    $outDir = Join-Path $artifactsRoot $package.dir
    if (Test-Path -LiteralPath $outDir) { Remove-Item -LiteralPath $outDir -Recurse -Force }
    $null = New-Item -ItemType Directory -Path $outDir -Force
    $logDir = Join-Path $outDir 'build-logs'
    $null = New-Item -ItemType Directory -Path $logDir -Force
    Write-Host "== $($package.base)"

    $tsManifest = Join-Path $packagesRoot "$($package.dir)\tigersetup\TigerSetup.toml"
    $tsOutput = Join-Path $outDir "$($package.base)-TigerSetup.exe"
    $seconds = Invoke-Timed -Label "TigerSetup ($($package.base))" -Executable $BuilderPath `
        -Arguments @('build', $tsManifest, '--output', $tsOutput) -LogPath (Join-Path $logDir 'tigersetup.log')
    $builds.Add([pscustomobject][ordered]@{ app = $package.base; technology = 'TigerSetup'; installer = $tsOutput; definition = $tsManifest; buildSeconds = $seconds })

    $iss = Get-ChildItem -LiteralPath (Join-Path $packagesRoot "$($package.dir)\innosetup") -Filter '*.iss' | Select-Object -First 1
    $seconds = Invoke-Timed -Label "Inno Setup ($($package.base))" -Executable $InnoSetupCompiler `
        -Arguments @('/Q', $iss.FullName) -LogPath (Join-Path $logDir 'innosetup.log')
    $builds.Add([pscustomobject][ordered]@{ app = $package.base; technology = 'InnoSetup'; installer = (Join-Path $outDir "$($package.base)-InnoSetup.exe"); definition = $iss.FullName; buildSeconds = $seconds })

    $nsi = Get-ChildItem -LiteralPath (Join-Path $packagesRoot "$($package.dir)\nsis") -Filter '*.nsi' | Select-Object -First 1
    $seconds = Invoke-Timed -Label "NSIS ($($package.base))" -Executable $NsisCompiler `
        -Arguments @('/V2', $nsi.FullName) -LogPath (Join-Path $logDir 'nsis.log')
    $builds.Add([pscustomobject][ordered]@{ app = $package.base; technology = 'NSIS'; installer = (Join-Path $outDir "$($package.base)-NSIS.exe"); definition = $nsi.FullName; buildSeconds = $seconds })

    foreach ($build in @($builds | Where-Object { $_.app -eq $package.base })) {
        if (-not (Test-Path -LiteralPath $build.installer -PathType Leaf)) { throw "Expected installer '$($build.installer)' was not produced." }
    }
    & (Join-Path $PSScriptRoot 'Measure-StaticProperties.ps1') -InstallerDirectory $outDir -OutputJson (Join-Path $resultsRoot $package.results)
}

# Every TigerSetup installer must carry the engine beside the builder; a
# measurement of any other engine is a measurement of the wrong product.
$engineSha = (Get-FileHash -LiteralPath $enginePath -Algorithm SHA256).Hash.ToLowerInvariant()
foreach ($build in @($builds | Where-Object { $_.technology -eq 'TigerSetup' })) {
    $inspected = & $BuilderPath inspect $build.installer --json | ConvertFrom-Json
    $carried = [string] $inspected.package.engine.engine_sha256
    if ($carried -ne $engineSha) { throw "'$($build.installer)' carries engine $carried, not the engine beside the builder ($engineSha)." }
}

$record = [ordered]@{
    builtAt = [DateTimeOffset]::Now.ToString('o')
    builder = $builderVersion
    engineSha256 = $engineSha
    note = 'Wall-clock seconds around each compiler invocation on the build machine, one run each: an engineering observation, not a benchmark of the compilers.'
    builds = @($builds)
}
$outJson = Join-Path $resultsRoot 'build.json'
if ($Only.Count -gt 0 -and (Test-Path -LiteralPath $outJson)) {
    # A partial rebuild replaces only the packages it built.
    $existing = Get-Content -LiteralPath $outJson -Raw | ConvertFrom-Json
    $kept = @($existing.builds | Where-Object { ($_.app.ToLowerInvariant()) -notin @($packages | ForEach-Object { $_.base.ToLowerInvariant() }) })
    $record.builds = @($kept + $builds)
}
$record | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $outJson -Encoding utf8
Write-Host "Wrote $outJson ($($record.builds.Count) build(s))"

# The toolchain as it really is on this machine: the exact engine the
# TigerSetup installers carry, the exact Inno Setup x64 engine and loader the
# compiler embeds, and the exact NSIS exehead the chosen compressor uses.
function Get-FileFacts {
    param([string] $Path)
    $item = Get-Item -LiteralPath $Path
    [ordered]@{ path = $item.FullName; bytes = $item.Length; sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$innoDir = Split-Path -Parent $InnoSetupCompiler
$nsisDir = Split-Path -Parent $NsisCompiler
$nsisVersion = ((& $NsisCompiler /VERSION 2>&1 | Out-String).Trim() -replace '^v', '')
$innoVersion = ''
try { $innoVersion = [string] (winget list --id JRSoftware.InnoSetup.7 --exact 2>$null | Select-String -Pattern 'JRSoftware\.InnoSetup\.7\s+(\S+)' | ForEach-Object { $_.Matches[0].Groups[1].Value } | Select-Object -First 1) } catch { $innoVersion = '' }
if ([string]::IsNullOrWhiteSpace($innoVersion)) { $innoVersion = (Get-Item -LiteralPath $InnoSetupCompiler).VersionInfo.ProductVersion }
$gitCommit = ''
try { $gitCommit = (git -C $repoRoot rev-parse --short HEAD 2>$null | Out-String).Trim() } catch { $gitCommit = '' }
$gitDirty = $false
try { $gitDirty = -not [string]::IsNullOrWhiteSpace((git -C $repoRoot status --porcelain 2>$null | Out-String)) } catch { $gitDirty = $false }
$toolchain = [ordered]@{
    captured_utc = [DateTimeOffset]::UtcNow.ToString('yyyy-MM-dd')
    host = 'the build machine (Windows 11 x64)'
    tigersetup = [ordered]@{
        version = ($builderVersion -replace '^tiger-setup\s+', '')
        git_commit = $gitCommit
        working_tree_dirty = $gitDirty
        note = 'Frozen for the benchmark; built with cargo build --release. The engine is embedded verbatim (uncompressed) in every generated installer.'
        engine = (Get-FileFacts $enginePath)
        builder = (Get-FileFacts $BuilderPath)
    }
    inno_setup = [ordered]@{
        product = 'Inno Setup 7'
        winget_id = 'JRSoftware.InnoSetup.7'
        version = $innoVersion
        install_path = $innoDir
        compiler = (Get-FileFacts $InnoSetupCompiler)
        native_x64_engine = (Get-FileFacts (Join-Path $innoDir 'Setup.e64'))
        loader_stub = (Get-FileFacts (Join-Path $innoDir 'SetupLdr.e64'))
        note = 'Setup.e64 is the native x64 setup engine an x64-target installer embeds (LZMA2-compressed, behind the SetupLdr.e64 loader); the minimal installer is therefore smaller than the uncompressed engine image.'
        compression = 'Compression=lzma2/max, SolidCompression=yes'
    }
    nsis = [ordered]@{
        product = 'NSIS (Nullsoft Install System)'
        winget_id = 'NSIS.NSIS'
        version = $nsisVersion
        install_path = $nsisDir
        compiler = (Get-FileFacts $NsisCompiler)
        exehead = (Get-FileFacts (Join-Path $nsisDir 'Stubs\lzma_solid-x86-unicode'))
        note = 'Stock NSIS ships only x86 (32-bit) exeheads; every NSIS installer here is a 32-bit process installing a 64-bit payload. The exehead named is the one SetCompressor /SOLID lzma with Unicode true selects; makensis additionally compiles the script into an "EXE header" (bytecode, strings, resources) it reports in its build log.'
        compression = 'SetCompressor /SOLID lzma'
    }
}
$toolchainJson = Join-Path $resultsRoot 'toolchain.json'
$toolchain | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $toolchainJson -Encoding utf8
Write-Host "Wrote $toolchainJson"
