<#
    .SYNOPSIS
    Builds the TigerSetupTestCompletion installer the PowerShell completion lab
    rows install.

    .DESCRIPTION
    Stages the release build's TigerSetupTestAction.exe
    (crates/tigersetup-test-action) as stage\bin\, then builds the installer
    with the release tiger-setup.exe, whose engine is the release
    tigersetup-setup.exe beside it; the build runs the staged program's
    `--completion pwsh` to generate the completion script it packages. A lab
    row measures the engine embedded in the installer, so the release binaries
    must be built first (LESSONS_LEARNED.md); -SkipBuild reuses the ones
    already there.

    .EXAMPLE
    pwsh -File packages\test-completion\Build-Package.ps1
    pwsh -File packages\test-completion\Build-Package.ps1 -Fast -SkipBuild   # iteration only
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    # The release directory holding the built binaries; defaults to this
    # workspace's release target.
    [string] $ReleaseDirectory,
    # Where the installer goes; defaults to artifacts\test-completion under
    # the repository root.
    [string] $OutputDirectory,
    # Skip `cargo build --release`; reuse the binaries already built.
    [switch] $SkipBuild,
    # Iteration-loop build: skip the compression search. Not for acceptance.
    [switch] $Fast
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$packageRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent (Split-Path -Parent $packageRoot)
if ([string]::IsNullOrWhiteSpace($ReleaseDirectory)) {
    $ReleaseDirectory = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release'
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot 'artifacts\test-completion'
}
$builder = Join-Path $ReleaseDirectory 'tiger-setup.exe'
$engine = Join-Path $ReleaseDirectory 'tigersetup-setup.exe'
$program = Join-Path $ReleaseDirectory 'TigerSetupTestAction.exe'

if (-not $SkipBuild) {
    Write-Host 'Building the release binaries...'
    Push-Location $repoRoot
    try { cargo build --release; if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed ($LASTEXITCODE)." } }
    finally { Pop-Location }
}
foreach ($binary in @($builder, $engine, $program)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "The release binary $binary is missing; run without -SkipBuild."
    }
}

$stage = Join-Path $packageRoot 'stage'
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
$null = New-Item -ItemType Directory -Path (Join-Path $stage 'bin') -Force
Copy-Item -LiteralPath $program -Destination (Join-Path $stage 'bin\TigerSetupTestAction.exe') -Force

$null = New-Item -ItemType Directory -Path $OutputDirectory -Force
$arguments = @('build', (Join-Path $packageRoot 'TigerSetup.toml'), '--output', $OutputDirectory)
if ($Fast) { $arguments += '--fast' }
Write-Host "Building TigerSetupTestCompletion ($(if ($Fast) { '--fast' } else { 'release' }))..."
& $builder @arguments
if ($LASTEXITCODE -ne 0) { throw "tiger-setup build failed ($LASTEXITCODE)." }
