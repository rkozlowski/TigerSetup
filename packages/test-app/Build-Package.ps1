#Requires -Version 7.0
<#
    .SYNOPSIS
    Builds both TigerSetupTestApp installers: payloads, the embedded
    prerequisite, the action programs, and `tiger-setup build` for 1.0.0 and
    1.1.0.

    .DESCRIPTION
    Generates the deterministic payloads (New-TestAppPayload.ps1), copies the
    release build's TigerSetupTestPrereq.exe — the controlled prerequisite the
    package embeds — into each version's dependencies/ directory, copies the
    release build's TigerSetupTestAction.exe and the committed action scripts
    (../actions) into each version's actions/ directory, and builds both
    installers with the release tiger-setup.exe, whose engine is the release
    tigersetup-setup.exe beside it. A lab row measures the engine embedded in
    the installer, so the release binaries must be built first
    (LESSONS_LEARNED.md); -SkipBuild reuses the ones already there.

    .EXAMPLE
    pwsh -File packages\test-app\Build-Package.ps1
    pwsh -File packages\test-app\Build-Package.ps1 -Fast -SkipBuild   # iteration only
#>
[CmdletBinding()]
param(
    # The release directory holding the built binaries; defaults to this
    # workspace's release target.
    [string] $ReleaseDirectory,
    # Where the installers go; defaults to artifacts\test-app under the
    # repository root.
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
    $OutputDirectory = Join-Path $repoRoot 'artifacts\test-app'
}
$builder = Join-Path $ReleaseDirectory 'tiger-setup.exe'
$engine = Join-Path $ReleaseDirectory 'tigersetup-setup.exe'
$prereq = Join-Path $ReleaseDirectory 'TigerSetupTestPrereq.exe'
$actionProgram = Join-Path $ReleaseDirectory 'TigerSetupTestAction.exe'
$actionScripts = Join-Path $packageRoot 'actions'

if (-not $SkipBuild) {
    Write-Host 'Building the release binaries...'
    Push-Location $repoRoot
    try { cargo build --release; if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed ($LASTEXITCODE)." } }
    finally { Pop-Location }
}
foreach ($binary in @($builder, $engine, $prereq, $actionProgram)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "The release binary $binary is missing; run without -SkipBuild."
    }
}

& pwsh -File (Join-Path $packageRoot 'New-TestAppPayload.ps1') -Root $packageRoot
if ($LASTEXITCODE -ne 0) { throw "New-TestAppPayload.ps1 failed ($LASTEXITCODE)." }

$null = New-Item -ItemType Directory -Path $OutputDirectory -Force
foreach ($version in @('1.0.0', '1.1.0')) {
    $dependencies = Join-Path $packageRoot $version 'dependencies'
    $null = New-Item -ItemType Directory -Path $dependencies -Force
    Copy-Item -LiteralPath $prereq -Destination (Join-Path $dependencies 'TigerSetupTestPrereq.exe') -Force
    $actions = Join-Path $packageRoot $version 'actions'
    $null = New-Item -ItemType Directory -Path $actions -Force
    Copy-Item -LiteralPath $actionProgram -Destination (Join-Path $actions 'TigerSetupTestAction.exe') -Force
    Copy-Item -Path (Join-Path $actionScripts '*') -Destination $actions -Force
    $manifest = Join-Path $packageRoot $version 'TigerSetup.toml'
    $arguments = @('build', $manifest, '--output', $OutputDirectory)
    if ($Fast) { $arguments += '--fast' }
    Write-Host "Building TigerSetupTestApp $version ($(if ($Fast) { '--fast' } else { 'release' }))..."
    & $builder @arguments
    if ($LASTEXITCODE -ne 0) { throw "tiger-setup build failed for $version ($LASTEXITCODE)." }
}
