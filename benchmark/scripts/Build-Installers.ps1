#Requires -Version 7.0
<#
    .SYNOPSIS
    Builds the benchmark's 15 installers — the minimal package and the four
    applications, each with TigerSetup, Inno Setup and NSIS — from the
    package definitions in benchmark/packages, measures every build (wall
    clock, CPU time, peak memory of the compiler's whole process tree) and
    every installer (bytes, SHA-256), and records the toolchain.

    .DESCRIPTION
    The TigerSetup installers are built with the release builder beside the
    release engine (target\x86_64-pc-windows-msvc\release), at release
    quality — never --fast, whose bytes are not the ones a benchmark measures.
    Inno Setup and NSIS are located through their winget installs (the
    default paths) or -InnoSetupCompiler / -NsisCompiler.

    Every build runs under the process-tree meter (ProcessTreeMeter.psm1):
    the compiler is started suspended inside a job object of its own and
    resumed, so its children — NSIS's makensis.exe is a launcher for
    Bin\makensis.exe — are measured with it. Wall clock runs until the job
    is empty; CPU time and peak commit are the job's own kernel-maintained
    accounting; peak working set is sampled every -SampleIntervalMilliseconds
    (the module's documentation says what sampling can miss, and every
    record carries its sample and process counts). Builds are serial, one
    at a time, and the technology order rotates between applications so no
    technology always builds first (into a cold cache) or last.

    Every build starts from an emptied artifacts directory, so the measured
    set is exactly what the definitions in the repository produce; the
    engine identity the TigerSetup installers carry is verified after the
    build against the engine beside the builder and recorded with the
    builder's version, so a measurement can never be attributed to the wrong
    engine (toolchain.json). The host — Windows build, CPU, cores, memory,
    the drive under the repository — is recorded there too.

    The defaults write to benchmark/artifacts and benchmark/results. A
    campaign that must not touch an earlier campaign's results gives both
    -ArtifactsRoot and -ResultsRoot of its own.

    .EXAMPLE
    pwsh -File benchmark\scripts\Build-Installers.ps1
    pwsh -File benchmark\scripts\Build-Installers.ps1 -Only minimal,qbittorrent
    pwsh -File benchmark\scripts\Build-Installers.ps1 -ArtifactsRoot benchmark\artifacts\0.9.0 -ResultsRoot benchmark\results\0.9.0
#>
[CmdletBinding()]
param(
    [string[]] $Only,
    [string] $BuilderPath,
    [string] $InnoSetupCompiler = 'C:\Program Files\Inno Setup 7\ISCC.exe',
    [string] $NsisCompiler = 'C:\Program Files (x86)\NSIS\makensis.exe',
    [string] $ArtifactsRoot = (Join-Path $PSScriptRoot '..\artifacts'),
    [string] $ResultsRoot = (Join-Path $PSScriptRoot '..\results'),
    [int] $SampleIntervalMilliseconds = 20,
    [int] $BuildTimeoutMinutes = 60
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'ProcessTreeMeter.psm1') -Force

$benchmarkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $benchmarkRoot '..')).Path
$packagesRoot = Join-Path $benchmarkRoot 'packages'
$ArtifactsRoot = [System.IO.Path]::GetFullPath($ArtifactsRoot)
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ArtifactsRoot -Force
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force

if ([string]::IsNullOrWhiteSpace($BuilderPath)) {
    $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe'
}
foreach ($tool in @($BuilderPath, $InnoSetupCompiler, $NsisCompiler)) {
    if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw "Compiler '$tool' does not exist." }
}
$enginePath = Join-Path (Split-Path -Parent $BuilderPath) 'tigersetup-setup.exe'
if (-not (Test-Path -LiteralPath $enginePath -PathType Leaf)) { throw "The engine '$enginePath' is not beside the builder." }
# The C loader (0.9.0 and later) sits beside the builder as well; an older
# builder has none, and the toolchain record says so by omission.
$loaderPath = Join-Path (Split-Path -Parent $BuilderPath) 'tigersetup-loader.exe'
if (-not (Test-Path -LiteralPath $loaderPath -PathType Leaf)) { $loaderPath = $null }

# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Only = @($Only | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim().ToLowerInvariant() } | Where-Object { $_ })

function Get-FileFacts {
    param([string] $Path)
    $item = Get-Item -LiteralPath $Path
    [ordered]@{ path = $item.FullName; bytes = $item.Length; sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}

# --- the toolchain as it really is on this machine -------------------------
$builderVersion = (& $BuilderPath --version 2>&1 | Out-String).Trim()
$engineSha = (Get-FileHash -LiteralPath $enginePath -Algorithm SHA256).Hash.ToLowerInvariant()
$innoDir = Split-Path -Parent $InnoSetupCompiler
$nsisDir = Split-Path -Parent $NsisCompiler
$nsisVersion = ((& $NsisCompiler /VERSION 2>&1 | Out-String).Trim() -replace '^v', '')
$innoVersion = ''
try { $innoVersion = [string] (winget list --id JRSoftware.InnoSetup.7 --exact 2>$null | Select-String -Pattern 'JRSoftware\.InnoSetup\.7\s+(\S+)' | ForEach-Object { $_.Matches[0].Groups[1].Value } | Select-Object -First 1) } catch { $innoVersion = '' }
if ([string]::IsNullOrWhiteSpace($innoVersion)) { $innoVersion = (Get-Item -LiteralPath $InnoSetupCompiler).VersionInfo.ProductVersion }
# NSIS's makensis.exe in the install root is a launcher; Bin\makensis.exe is the compiler it starts.
$nsisCompilerBin = Join-Path $nsisDir 'Bin\makensis.exe'
if (-not (Test-Path -LiteralPath $nsisCompilerBin -PathType Leaf)) { $nsisCompilerBin = $null }

$tools = @{
    TigerSetup = [ordered]@{ version = ($builderVersion -replace '^tiger-setup\s+', ''); path = $BuilderPath; sha256 = (Get-FileHash -LiteralPath $BuilderPath -Algorithm SHA256).Hash.ToLowerInvariant() }
    InnoSetup  = [ordered]@{ version = $innoVersion; path = $InnoSetupCompiler; sha256 = (Get-FileHash -LiteralPath $InnoSetupCompiler -Algorithm SHA256).Hash.ToLowerInvariant() }
    NSIS       = [ordered]@{ version = $nsisVersion; path = $NsisCompiler; sha256 = (Get-FileHash -LiteralPath $NsisCompiler -Algorithm SHA256).Hash.ToLowerInvariant() }
}
Write-Host "Builder:    $builderVersion ($BuilderPath)"
Write-Host "Engine:     $engineSha ($enginePath)"
Write-Host "Inno Setup: $innoVersion ($InnoSetupCompiler)"
Write-Host "NSIS:       $nsisVersion ($NsisCompiler)"

# App directory name -> (installer base name, result file, technology order).
# The order rotates so no technology always builds first or last; it is the
# lab campaign's rotation for the four applications, and Minimal takes the
# order that balances the first position (two apps each start with
# TigerSetup and NSIS, one with Inno Setup).
$packages = @(
    [pscustomobject]@{ dir = 'minimal';     base = 'Minimal';     results = 'minimal-installers.json';     order = @('NSIS', 'TigerSetup', 'InnoSetup') }
    [pscustomobject]@{ dir = 'sharex';      base = 'ShareX';      results = 'sharex-installers.json';      order = @('TigerSetup', 'InnoSetup', 'NSIS') }
    [pscustomobject]@{ dir = 'winmerge';    base = 'WinMerge';    results = 'winmerge-installers.json';    order = @('InnoSetup', 'NSIS', 'TigerSetup') }
    [pscustomobject]@{ dir = 'qbittorrent'; base = 'qBittorrent'; results = 'qbittorrent-installers.json'; order = @('NSIS', 'TigerSetup', 'InnoSetup') }
    [pscustomobject]@{ dir = 'vlc';         base = 'VLC';         results = 'vlc-installers.json';         order = @('TigerSetup', 'InnoSetup', 'NSIS') }
)
if ($Only.Count -gt 0) { $packages = @($packages | Where-Object { $_.dir -in $Only }) }

function Invoke-Build {
    <#
        Runs one compiler under the meter, fails on a non-zero exit, and
        returns the build's record: identity, timing, memory, and the
        installer's bytes and hash.
    #>
    param([string] $App, [string] $Technology, [string] $Executable, [string[]] $Arguments, [string] $Definition, [string] $Installer, [string] $LogPath)
    $label = "$Technology ($App)"
    $measurement = Invoke-MeasuredProcess -FilePath $Executable -ArgumentList $Arguments -LogPath $LogPath `
        -SampleIntervalMilliseconds $SampleIntervalMilliseconds -TimeoutSeconds ($BuildTimeoutMinutes * 60)
    if ($measurement.timedOut) { throw "$label did not finish within $BuildTimeoutMinutes minutes; see $LogPath" }
    if ($measurement.exitCode -ne 0) { throw "$label failed (exit $($measurement.exitCode)); see $LogPath" }
    if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) { throw "Expected installer '$Installer' was not produced." }
    $item = Get-Item -LiteralPath $Installer
    Write-Host ("  {0,-28} {1,8:N1} s wall  {2,7:N1} s cpu  peak tree WS {3,7:N0} KB  peak commit {4,7:N0} KB  ({5} samples, {6} processes)  {7,13:N0} bytes" -f `
            $label, $measurement.wallSeconds, $measurement.cpuSeconds, ($measurement.peakTreeWorkingSetBytes / 1KB), ($measurement.peakJobCommitBytes / 1KB), $measurement.samples, $measurement.processesTotal, $item.Length)
    $tool = $tools[$Technology]
    [ordered]@{
        app = $App
        technology = $Technology
        toolVersion = $tool.version
        toolPath = $tool.path
        toolSha256 = $tool.sha256
        definition = $Definition
        installer = $Installer
        installerBytes = $item.Length
        installerSha256 = (Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash.ToLowerInvariant()
        success = $true
        # buildSeconds is the wall clock at the precision the first campaign recorded; wallSeconds is the same measurement unrounded.
        buildSeconds = [math]::Round([double] $measurement.wallSeconds, 1)
    } + $measurement
}

$builds = [System.Collections.Generic.List[object]]::new()
$campaignStarted = [DateTimeOffset]::Now
foreach ($package in $packages) {
    $outDir = Join-Path $ArtifactsRoot $package.dir
    if (Test-Path -LiteralPath $outDir) { Remove-Item -LiteralPath $outDir -Recurse -Force }
    $null = New-Item -ItemType Directory -Path $outDir -Force
    $logDir = Join-Path $outDir 'build-logs'
    $null = New-Item -ItemType Directory -Path $logDir -Force
    Write-Host "== $($package.base) ($($package.order -join ', '))"

    foreach ($technology in $package.order) {
        switch ($technology) {
            'TigerSetup' {
                $manifest = Join-Path $packagesRoot "$($package.dir)\tigersetup\TigerSetup.toml"
                $installer = Join-Path $outDir "$($package.base)-TigerSetup.exe"
                $builds.Add([pscustomobject] (Invoke-Build -App $package.base -Technology 'TigerSetup' -Executable $BuilderPath `
                            -Arguments @('build', $manifest, '--output', $installer) -Definition $manifest -Installer $installer -LogPath (Join-Path $logDir 'tigersetup.log')))
            }
            'InnoSetup' {
                $iss = Get-ChildItem -LiteralPath (Join-Path $packagesRoot "$($package.dir)\innosetup") -Filter '*.iss' | Select-Object -First 1
                $installer = Join-Path $outDir "$($package.base)-InnoSetup.exe"
                # The .iss names its output directory relative to itself; /O redirects it to this campaign's artifacts.
                $builds.Add([pscustomobject] (Invoke-Build -App $package.base -Technology 'InnoSetup' -Executable $InnoSetupCompiler `
                            -Arguments @('/Q', "/O$outDir", $iss.FullName) -Definition $iss.FullName -Installer $installer -LogPath (Join-Path $logDir 'innosetup.log')))
            }
            'NSIS' {
                $nsi = Get-ChildItem -LiteralPath (Join-Path $packagesRoot "$($package.dir)\nsis") -Filter '*.nsi' | Select-Object -First 1
                $installer = Join-Path $outDir "$($package.base)-NSIS.exe"
                # The .nsi's OutFile defaults to a path relative to itself and takes /DOUTFILE= for this campaign's artifacts.
                $builds.Add([pscustomobject] (Invoke-Build -App $package.base -Technology 'NSIS' -Executable $NsisCompiler `
                            -Arguments @('/V2', "/DOUTFILE=$installer", $nsi.FullName) -Definition $nsi.FullName -Installer $installer -LogPath (Join-Path $logDir 'nsis.log')))
            }
        }
    }
    & (Join-Path $PSScriptRoot 'Measure-StaticProperties.ps1') -InstallerDirectory $outDir -OutputJson (Join-Path $ResultsRoot $package.results)
}

# Every TigerSetup installer must carry the engine beside the builder; a
# measurement of any other engine is a measurement of the wrong product.
foreach ($build in @($builds | Where-Object { $_.technology -eq 'TigerSetup' })) {
    $inspected = & $BuilderPath inspect $build.installer --json | ConvertFrom-Json
    $carried = [string] $inspected.package.engine.engine_sha256
    if ($carried -ne $engineSha) { throw "'$($build.installer)' carries engine $carried, not the engine beside the builder ($engineSha)." }
}

$record = [ordered]@{
    builtAt = $campaignStarted.ToString('o')
    finishedAt = [DateTimeOffset]::Now.ToString('o')
    builder = $builderVersion
    engineSha256 = $engineSha
    method = [ordered]@{
        serial = $true
        order = 'technology order rotates between applications (each record carries its start time)'
        wall = 'from the compiler process resuming inside its job object until the job holds no process'
        cpu = 'the job object accounting: user + kernel time of every process that belonged to the job, exited ones included'
        peakCommit = 'the job object PeakJobMemoryUsed (peakJobCommitBytes: all processes together) and PeakProcessMemoryUsed (peakProcessCommitBytes: the largest single process) — exact, kernel-maintained'
        peakWorkingSet = "sampled every $SampleIntervalMilliseconds ms: peakTreeWorkingSetBytes is the largest sum of the job's processes' working sets seen in one sample; peakProcessWorkingSetBytes the largest Windows-tracked per-process peak read while the process was alive; a process that lives and dies between two samples is missed (samples and processesSeen against processesTotal say how much was covered)"
        repetitions = 'one build each on the build machine, the same host for every technology; single-run figures, interpret with the host record in toolchain.json'
    }
    note = 'Wall-clock, CPU and memory of each compiler invocation and its process tree on the build machine, one run each: an engineering observation, not a benchmark of the compilers.'
    builds = @($builds)
}
$outJson = Join-Path $ResultsRoot 'build.json'
if ($Only.Count -gt 0 -and (Test-Path -LiteralPath $outJson)) {
    # A partial rebuild replaces only the packages it built.
    $existing = Get-Content -LiteralPath $outJson -Raw | ConvertFrom-Json
    $kept = @($existing.builds | Where-Object { ($_.app.ToLowerInvariant()) -notin @($packages | ForEach-Object { $_.base.ToLowerInvariant() }) })
    $record.builds = @($kept + $builds)
}
$record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $outJson -Encoding utf8
Write-Host "Wrote $outJson ($($record.builds.Count) build(s))"

# --- the host and the toolchain, as they really are ------------------------
function Get-HostFacts {
    $facts = [ordered]@{}
    try {
        $os = Get-CimInstance -ClassName Win32_OperatingSystem
        $facts.windows = [ordered]@{ caption = [string] $os.Caption; version = [string] $os.Version; build = [string] $os.BuildNumber }
        try { $facts.windows.displayVersion = [string] (Get-ItemPropertyValue -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -Name DisplayVersion) } catch { }
    } catch { $facts.windows = "unavailable: $($_.Exception.Message)" }
    try {
        $cpu = @(Get-CimInstance -ClassName Win32_Processor)
        $facts.cpu = [ordered]@{ name = (($cpu | ForEach-Object { ([string] $_.Name).Trim() }) -join '; '); sockets = $cpu.Count; cores = [int] (($cpu | Measure-Object -Property NumberOfCores -Sum).Sum); logicalProcessors = [int] (($cpu | Measure-Object -Property NumberOfLogicalProcessors -Sum).Sum); maxClockMHz = [int] $cpu[0].MaxClockSpeed }
    } catch { $facts.cpu = "unavailable: $($_.Exception.Message)" }
    try { $facts.memoryBytes = [long] (Get-CimInstance -ClassName Win32_ComputerSystem).TotalPhysicalMemory } catch { $facts.memoryBytes = $null }
    try {
        $drive = [System.IO.Path]::GetPathRoot($repoRoot).TrimEnd('\', ':')
        $partition = Get-Partition -DriveLetter $drive -ErrorAction Stop | Select-Object -First 1
        $disk = Get-PhysicalDisk -ErrorAction Stop | Where-Object { $_.DeviceId -eq [string] $partition.DiskNumber } | Select-Object -First 1
        $facts.repositoryDrive = [ordered]@{ letter = "$($drive):"; model = [string] $disk.FriendlyName; mediaType = [string] $disk.MediaType; busType = [string] $disk.BusType; sizeBytes = [long] $disk.Size }
    } catch { $facts.repositoryDrive = "unavailable: $($_.Exception.Message)" }
    try { $facts.powershell = [string] $PSVersionTable.PSVersion } catch { }
    $facts
}

$gitCommit = ''
try { $gitCommit = (git -C $repoRoot rev-parse --short HEAD 2>$null | Out-String).Trim() } catch { $gitCommit = '' }
$gitDirty = $false
try { $gitDirty = -not [string]::IsNullOrWhiteSpace((git -C $repoRoot status --porcelain 2>$null | Out-String)) } catch { $gitDirty = $false }
$tigersetup = [ordered]@{
    version = $tools.TigerSetup.version
    git_commit = $gitCommit
    working_tree_dirty = $gitDirty
    note = 'Frozen for the benchmark; built with cargo build --release. Every TigerSetup installer built here was verified (tiger-setup inspect) to carry exactly this engine.'
    engine = (Get-FileFacts $enginePath)
    builder = (Get-FileFacts $BuilderPath)
}
if ($null -ne $loaderPath) { $tigersetup.loader = (Get-FileFacts $loaderPath) }
$nsisRecord = [ordered]@{
    product = 'NSIS (Nullsoft Install System)'
    winget_id = 'NSIS.NSIS'
    version = $nsisVersion
    install_path = $nsisDir
    compiler = (Get-FileFacts $NsisCompiler)
}
if ($null -ne $nsisCompilerBin) { $nsisRecord.compiler_bin = (Get-FileFacts $nsisCompilerBin) }
$nsisRecord.exehead = (Get-FileFacts (Join-Path $nsisDir 'Stubs\lzma_solid-x86-unicode'))
$nsisRecord.note = 'Stock NSIS ships only x86 (32-bit) exeheads; every NSIS installer here is a 32-bit process installing a 64-bit payload. The exehead named is the one SetCompressor /SOLID lzma with Unicode true selects; makensis additionally compiles the script into an "EXE header" (bytecode, strings, resources) it reports in its build log. makensis.exe in the install root is a launcher for Bin\makensis.exe, the compiler proper; the meter measures both.'
$nsisRecord.compression = 'SetCompressor /SOLID lzma'
$toolchain = [ordered]@{
    captured_utc = [DateTimeOffset]::UtcNow.ToString('yyyy-MM-dd')
    host = (Get-HostFacts)
    tigersetup = $tigersetup
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
    nsis = $nsisRecord
}
$toolchainJson = Join-Path $ResultsRoot 'toolchain.json'
$toolchain | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $toolchainJson -Encoding utf8
Write-Host "Wrote $toolchainJson"
