<#
    .SYNOPSIS
    Builds tiger-mark, the TigerMarkView command line that renders the installed
    help's PDF, from one pinned TigerMarkView source commit.

    .DESCRIPTION
    A developer machine resolves tiger-mark as the registered TigerMarkView tool
    (eng\TigerAiCore.psm1). The release workflow's runner has no TigerAiCore
    configuration and installs no TigerMarkView release: it builds the command
    from source (Build-TigerSetupTigerMark in TigerSetupRelease.psm1).

      1. clone https://github.com/rkozlowski/TigerMarkView and check out
         -Commit, which must be a full 40-character SHA; the checkout's HEAD
         must be exactly that commit, clean, and contain TigerMarkView's
         active-content security fix;
      2. dotnet publish src\TigerMarkView.Cli alone, framework-dependent for
         win-x64, into <Directory>\tiger-mark;
      3. the published tiger-mark.exe must run and report the version that
         commit's Version.props declares.

    Every failure stops the build; there is no other source of tiger-mark. The
    path goes to GITHUB_OUTPUT as `path`, which the build passes to
    Build-ReleaseArtifacts.ps1 -TigerMarkPath.

    The build needs git and the .NET 10 SDK; tiger-mark itself needs the .NET 10
    Desktop Runtime and the WebView2 Runtime, and renders through a shown
    (off-screen) WebView2 window. GitHub's Windows images carry both runtimes
    and run jobs in an interactive session; the workflow sets up the SDK.

    .EXAMPLE
    pwsh -File eng\release\Build-TigerMark.ps1 -Commit <40-character SHA> -Directory $env:RUNNER_TEMP\tigermarkview
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    # The TigerMarkView commit, as the release workflow pins it.
    [Parameter(Mandatory)] [string] $Commit,
    # Empty or absent; the source and the published command go beneath it.
    [string] $Directory = (Join-Path ([IO.Path]::GetTempPath()) "tigermarkview-$([guid]::NewGuid().ToString('N'))"),
    # Another clone URL or path holding the same commit (the tests' stand-in).
    [string] $Repository,
    [string] $GitHubOutput
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'TigerSetupRelease.psm1')
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'

$arguments = @{ Commit = $Commit; Directory = $Directory }
if ($Repository) { $arguments.Repository = $Repository }
$tigerMark = Build-TigerSetupTigerMark @arguments
Write-Host "tiger-mark: $($tigerMark.path)"
Write-Host "  TigerMarkView $($tigerMark.version) at $($tigerMark.commit); --version: $($tigerMark.reported)"
if ($GitHubOutput) { "path=$($tigerMark.path)" | Out-File -LiteralPath $GitHubOutput -Encoding utf8 -Append }
exit 0
