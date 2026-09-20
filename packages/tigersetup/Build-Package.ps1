#Requires -Version 7.0
<#
    .SYNOPSIS
    Builds TigerSetup's own installer with TigerSetup itself.

    .DESCRIPTION
    Stages the three release binaries the release ships — tiger-setup.exe (the
    builder), tigersetup-setup.exe (the engine) and tigersetup-loader.exe (the
    loader every generated Setup.exe begins with) — into stage/ beside this
    script, then runs `tiger-setup build` on TigerSetup.toml. The engine and
    the loader that build the installer are the release binaries beside the
    builder, so TigerSetup packages itself with its own current release
    (TigerSetup-Design.md §8.4). stage/ is ignored by Git.

    The default build is release quality — the zstd-19-w27 profile, the bytes
    a published installer is built with. Pass -Fast only for an
    edit-build-test loop; a released or validated self-installer is always the
    default build.

    .EXAMPLE
    pwsh -File packages\tigersetup\Build-Package.ps1
    pwsh -File packages\tigersetup\Build-Package.ps1 -Fast   # iteration only
#>
[CmdletBinding()]
param(
    # The release directory holding the built binaries; defaults to this
    # workspace's release target.
    [string] $ReleaseDirectory,
    # Where the installer goes; defaults to artifacts\tigersetup under the
    # repository root.
    [string] $OutputDirectory,
    # Skip `cargo build --release`; reuse the binaries already built.
    [switch] $SkipBuild,
    # Iteration-loop build: the fast compression level. Not for a release.
    [switch] $Fast
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$packageRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent (Split-Path -Parent $packageRoot)
$stageRoot = Join-Path $packageRoot 'stage'

if ([string]::IsNullOrWhiteSpace($ReleaseDirectory)) {
    $ReleaseDirectory = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release'
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot 'artifacts\tigersetup'
}

$builder = Join-Path $ReleaseDirectory 'tiger-setup.exe'
$engine = Join-Path $ReleaseDirectory 'tigersetup-setup.exe'
$loader = Join-Path $ReleaseDirectory 'tigersetup-loader.exe'

if (-not $SkipBuild) {
    Write-Host 'Building the release binaries...'
    Push-Location $repoRoot
    try { cargo build --release; if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed ($LASTEXITCODE)." } }
    finally { Pop-Location }
}

foreach ($binary in @($builder, $engine, $loader)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "The release binary $binary is missing; run without -SkipBuild."
    }
}

# Stage exactly the three binaries the release ships, nothing else from the
# release directory.
if (Test-Path -LiteralPath $stageRoot) { Remove-Item -LiteralPath $stageRoot -Recurse -Force }
$null = New-Item -ItemType Directory -Path $stageRoot -Force
Copy-Item -LiteralPath $builder -Destination (Join-Path $stageRoot 'tiger-setup.exe')
Copy-Item -LiteralPath $engine -Destination (Join-Path $stageRoot 'tigersetup-setup.exe')
Copy-Item -LiteralPath $loader -Destination (Join-Path $stageRoot 'tigersetup-loader.exe')

$null = New-Item -ItemType Directory -Path $OutputDirectory -Force
$manifest = Join-Path $packageRoot 'TigerSetup.toml'

$arguments = @('build', $manifest, '--output', $OutputDirectory)
if ($Fast) { $arguments += '--fast' }
Write-Host "Building the self-installer ($(if ($Fast) { '--fast' } else { 'release' }))..."
& $builder @arguments
if ($LASTEXITCODE -ne 0) { throw "tiger-setup build failed ($LASTEXITCODE)." }
