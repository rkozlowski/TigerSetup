#Requires -Version 7.0
<#
    .SYNOPSIS
    Builds the TigerMarkView installer from a pinned TigerMarkView commit.

    .DESCRIPTION
    Clones the TigerMarkView repository at the pinned commit into source/ beside
    this script (the checkout named by the TigerAiCore configuration is read,
    never written: dotnet publish writes intermediates under a project's own
    obj/), publishes the framework-dependent win-x64 tree into publish/, and
    runs tiger-setup build on TigerSetup.toml. Both directories are ignored by
    Git.

    -Version publishes and builds a version other than the pinned commit's
    own (a global MSBuild property the project honours), which is how the
    acceptance matrix gets a "next" version to upgrade to without a second
    commit.

    .EXAMPLE
    pwsh -File packages\TigerMarkView\Build-Package.ps1
    pwsh -File packages\TigerMarkView\Build-Package.ps1 -Version 0.8.2
#>
[CmdletBinding()]
param(
    # The TigerMarkView commit the package is built from.
    [string] $Commit = 'cfbf117',
    # Overrides the version the commit declares (MSBuild global property).
    [string] $Version,
    # The TigerMarkView repository to clone; derived from the TigerAiCore
    # configuration's [tools.TigerMarkView] entry when omitted.
    [string] $SourceRepository,
    # Where the installer goes; defaults to artifacts\TigerMarkView under the
    # repository root.
    [string] $OutputDirectory,
    # The builder and engine; default to the release build of this workspace.
    [string] $BuilderPath,
    [switch] $Offline,
    [switch] $SkipClone,
    [switch] $SkipPublish,
    # Clone and publish only; do not run the builder.
    [switch] $PublishOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$packageRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent (Split-Path -Parent $packageRoot)
$sourceRoot = Join-Path $packageRoot 'source'
$publishRoot = Join-Path $packageRoot 'publish'

function Get-TigerMarkViewRepository {
    <#
        Resolves the registered TigerMarkView tool through TigerAiCore's own
        resolver, which is the supported way to turn a registration name into a
        path. This script parses no configuration of its own: repository layout
        is not topology, and a hand-rolled reader is a second discovery system
        that drifts from the first. -SourceRepository overrides the resolved
        location for one run.
    #>
    if (-not [string]::IsNullOrWhiteSpace($SourceRepository)) { return $SourceRepository }
    Import-Module (Join-Path $repoRoot 'eng\TigerAiCore.psm1') -Force
    try { $toolPath = (Resolve-TigerAiCoreRegistration -Kind Tool -Name TigerMarkView).Path }
    catch { throw "$($_.Exception.Message) Pass -SourceRepository." }

    # The registration names the built CLI inside the checkout; the repository
    # root is the directory above it that carries Version.props.
    $candidate = Split-Path -Parent $toolPath
    while (-not [string]::IsNullOrWhiteSpace($candidate)) {
        if (Test-Path -LiteralPath (Join-Path $candidate 'Version.props') -PathType Leaf) { return $candidate }
        $candidate = Split-Path -Parent $candidate
    }
    throw "No Version.props above '$toolPath'; pass -SourceRepository."
}

if (-not $SkipClone) {
    $repository = Get-TigerMarkViewRepository
    Write-Host "Source:   $repository @ $Commit"
    if (Test-Path -LiteralPath $sourceRoot) { Remove-Item -LiteralPath $sourceRoot -Recurse -Force }
    & git clone --quiet --no-checkout $repository $sourceRoot
    if ($LASTEXITCODE -ne 0) { throw "git clone of '$repository' failed." }
    & git -C $sourceRoot checkout --quiet $Commit
    if ($LASTEXITCODE -ne 0) { throw "git checkout $Commit failed." }
}

$properties = @()
if (-not [string]::IsNullOrWhiteSpace($Version)) { $properties += "-p:Version=$Version" }

if (-not $SkipPublish) {
    if (Test-Path -LiteralPath $publishRoot) { Remove-Item -LiteralPath $publishRoot -Recurse -Force }
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $env:DOTNET_NOLOGO = '1'
    Write-Host "Publish:  $publishRoot"
    foreach ($project in @('src\TigerMarkView\TigerMarkView.csproj', 'src\TigerMarkView.Cli\TigerMarkView.Cli.csproj')) {
        & dotnet publish (Join-Path $sourceRoot $project) --configuration Release --runtime win-x64 --self-contained false --output $publishRoot -m:1 @properties
        if ($LASTEXITCODE -ne 0) { throw "dotnet publish of $project failed." }
    }
}

if ($PublishOnly) { return }

if ([string]::IsNullOrWhiteSpace($BuilderPath)) {
    $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe'
}
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) { throw "The builder '$BuilderPath' does not exist; run cargo build --release first." }
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot 'artifacts\TigerMarkView' }
$null = New-Item -ItemType Directory -Path $OutputDirectory -Force

$arguments = @('build', (Join-Path $packageRoot 'TigerSetup.toml'), '--output', $OutputDirectory)
if (-not [string]::IsNullOrWhiteSpace($Version)) { $arguments += @('--property', "Version=$Version") }
if ($Offline) { $arguments += '--offline' }
Write-Host "Build:    $BuilderPath $($arguments -join ' ')"
& $BuilderPath @arguments
if ($LASTEXITCODE -ne 0) { throw 'tiger-setup build failed.' }
