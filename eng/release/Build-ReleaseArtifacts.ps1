<#
    .SYNOPSIS
    Builds a TigerSetup release's artifacts from one commit and closes the set.

    .DESCRIPTION
    This is the build the release workflow runs, and it runs the same way on a
    developer machine. From a clean checkout of the release commit it:

      1. builds the release binaries and the self-installer on the
         release-quality path (packages\tigersetup\Build-Package.ps1);
      2. checks the installer against the binaries it was built from: it
         verifies, it is ItTiger.TigerSetup at this version, and it carries
         exactly the engine and loader this build produced;
      3. generates the WinGet manifest set from that exact installer and the
         URL it will be published at (`tiger-setup winget prepare`, then
         `finalize`), checks that its LicenseUrl and PrivacyUrl name this
         version's frozen terms, and packs it as TigerSetup-<version>-WinGet.zip;
      4. freezes the release-bound terms: LICENSE.txt (License) and PRIVACY.md
         (PrivacyStatement), the exact bytes the commit holds for them in Git;
      5. writes release-artifacts.json and SHA256SUMS.txt over the closed set.

    The output directory then holds exactly the release set. Only the
    set the release workflow builds and attaches to the draft GitHub Release is
    the release; a set built anywhere else is a candidate, however it was
    built (RELEASING.md).

    -Rehearsal builds from a working tree that is not a clean checkout of the
    commit - the preparation loop's way to prove this script before the commit
    exists. Its terms are the working tree's, as Git would commit them, and its
    record still names HEAD, so a rehearsal set is never a release: nothing
    publishes it.

    .EXAMPLE
    pwsh -File eng\release\Build-ReleaseArtifacts.ps1 -Version 0.12.0 -Rehearsal
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Version,
    # The commit being built; defaults to HEAD, and must be HEAD.
    [ValidatePattern('^([0-9a-fA-F]{40})?$')] [string] $CommitSha = '',
    # Defaults to artifacts\release-candidate\<version>; the workflow names its own.
    [string] $OutputDirectory,
    # tiger-mark.exe for the help PDF; defaults to the registered TigerMarkView tool.
    [string] $TigerMarkPath,
    # Reuse the release binaries already built from this tree (-Rehearsal only).
    [switch] $SkipBuild,
    # Allow a working tree that is not a clean checkout of the commit.
    [switch] $Rehearsal,
    # A GITHUB_OUTPUT file to append record_sha256 to.
    [string] $GitHubOutput
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'TigerSetupRelease.psm1')
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$facts = Get-TigerSetupReleaseFacts
$releaseDirectory = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release'
$builder = Join-Path $releaseDirectory 'tiger-setup.exe'

function Invoke-Checked {
    param([Parameter(Mandatory)] [string] $What, [Parameter(Mandatory)] [scriptblock] $Command)
    $errors = [IO.Path]::GetTempFileName()
    try {
        $output = & $Command 2>$errors | Out-String
        if ($LASTEXITCODE -ne 0) { throw "$What failed ($LASTEXITCODE): $output $(Get-Content -LiteralPath $errors -Raw)" }
    }
    finally { Remove-Item -LiteralPath $errors -Force -ErrorAction SilentlyContinue }
    $global:LASTEXITCODE = 0
    $output
}

# The commit and the tree.
if (-not (Test-TigerSetupReleaseVersion $Version)) { throw "'$Version' is not a <Major>.<Minor>.<Patch> version." }
$source = Get-TigerSetupSourceVersion -RepositoryRoot $repoRoot
if ($source -cne $Version) { throw "Cargo.toml records $source, not $Version." }
$head = (Invoke-TigerSetupGit $repoRoot @('rev-parse', 'HEAD')).output.ToLowerInvariant()
if (-not $CommitSha) { $CommitSha = $head }
$CommitSha = $CommitSha.ToLowerInvariant()
if ($SkipBuild -and -not $Rehearsal) { throw '-SkipBuild reuses binaries this run did not build from the commit; it is for -Rehearsal only.' }
if (-not $Rehearsal) {
    if ($head -cne $CommitSha) { throw "HEAD is $head, not the release commit $CommitSha." }
    $status = (Invoke-TigerSetupGit $repoRoot @('status', '--porcelain', '--untracked-files=normal')).output
    if ($status) { throw "The working tree is not a clean checkout of $CommitSha (use -Rehearsal to build a candidate):`n$status" }
}

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot "artifacts\release-candidate\$Version" }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$workDirectory = Join-Path $repoRoot "artifacts\release-work\$Version"
foreach ($directory in @($OutputDirectory, $workDirectory)) {
    if (Test-Path -LiteralPath $directory) {
        # Only a directory this script wrote is emptied: release assets and
        # its own working files, nothing else.
        $foreign = @(Get-ChildItem -LiteralPath $directory -Force | Where-Object {
                $_.Name -cnotin @(Get-TigerSetupReleaseAssetName -Version $Version) -and $_.Name -cnotin @("TigerSetup-$Version-Setup.exe", 'winget')
            })
        if ($foreign.Count) { throw "$directory holds files this script did not write: $($foreign.Name -join ', ')." }
        Remove-Item -LiteralPath $directory -Recurse -Force
    }
    $null = New-Item -ItemType Directory -Path $directory -Force
}

# 1. The binaries and the self-installer.
$packageArguments = @{ OutputDirectory = $workDirectory }
if ($SkipBuild) { $packageArguments.SkipBuild = $true }
if ($TigerMarkPath) { $packageArguments.TigerMarkPath = $TigerMarkPath }
& (Join-Path $repoRoot 'packages\tigersetup\Build-Package.ps1') @packageArguments
$installerName = "TigerSetup-$Version-Setup.exe"
$installer = Join-Path $workDirectory $installerName
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { throw "The package build produced no $installerName." }

# 2. The installer is this build's, at this version, and it verifies.
$inspect = (Invoke-Checked 'tiger-setup inspect' { & $builder inspect $installer --json }) | ConvertFrom-Json
$engine = $inspect.package.engine
$expected = [ordered]@{
    'verification'       = @([string] $inspect.verification.status, 'ok')
    'package id'         = @([string] $inspect.package.id, $facts.PackageIdentifier)
    'package version'    = @([string] $inspect.package.version, $Version)
    'engine version'     = @([string] $engine.tigersetup_version, $Version)
    'engine SHA-256'     = @([string] $engine.engine_sha256, (Get-TigerSetupFileSha256 (Join-Path $releaseDirectory 'tigersetup-setup.exe')))
    'loader SHA-256'     = @([string] $engine.loader_sha256, (Get-TigerSetupFileSha256 (Join-Path $releaseDirectory 'tigersetup-loader.exe')))
}
foreach ($name in $expected.Keys) {
    $actual, $wanted = $expected[$name]
    if ($actual -cne $wanted) { throw "The installer's $name is '$actual', not '$wanted'." }
}

# 3. The WinGet manifest set, from these exact bytes and their published URL.
$url = Get-TigerSetupInstallerUrl -Version $Version
$wingetDirectory = Join-Path $workDirectory 'winget'
$null = Invoke-Checked 'tiger-setup winget prepare' { & $builder winget prepare (Join-Path $repoRoot $facts.PackageManifest) --installer $installer --output $wingetDirectory }
$null = Invoke-Checked 'tiger-setup winget finalize' { & $builder winget finalize $wingetDirectory --url $url --installer $installer }
$termsUrlProblems = @(Test-TigerSetupWinGetTermsUrl -ManifestDirectory $wingetDirectory -Version $Version)
if ($termsUrlProblems.Count) { throw "The WinGet manifests do not name this version's frozen terms: $($termsUrlProblems -join '; ')." }
$wingetArchive = Join-Path $OutputDirectory "TigerSetup-$Version-WinGet.zip"
Add-Type -AssemblyName System.IO.Compression
$stream = [IO.File]::Open($wingetArchive, [IO.FileMode]::CreateNew)
try {
    $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in @(Get-ChildItem -LiteralPath $wingetDirectory -File | Sort-Object Name)) {
            $entry = $zip.CreateEntry($file.Name, [IO.Compression.CompressionLevel]::Optimal)
            $entry.LastWriteTime = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
            $writer = $entry.Open()
            try { $bytes = [IO.File]::ReadAllBytes($file.FullName); $writer.Write($bytes, 0, $bytes.Length) }
            finally { $writer.Dispose() }
        }
    }
    finally { $zip.Dispose() }
}
finally { $stream.Dispose() }

# 4. The release-bound terms, frozen from the commit.
$terms = @(Copy-TigerSetupReleaseTerms -RepositoryRoot $repoRoot -Directory $OutputDirectory -Version $Version -CommitSha $CommitSha -WorkingTree:$Rehearsal)

# 5. The closed set.
Copy-Item -LiteralPath $installer -Destination (Join-Path $OutputDirectory $installerName)
$recordSha256 = Write-TigerSetupReleaseRecord -Directory $OutputDirectory -Version $Version -CommitSha $CommitSha
$null = Assert-TigerSetupReleaseRecord -Directory $OutputDirectory -Version $Version -CommitSha $CommitSha -ExpectedRecordSha256 $recordSha256
if (-not $Rehearsal) { $null = Assert-TigerSetupReleaseTerms -RepositoryRoot $repoRoot -Directory $OutputDirectory -Version $Version -CommitSha $CommitSha }
if ($GitHubOutput) { "record_sha256=$recordSha256" | Out-File -LiteralPath $GitHubOutput -Encoding utf8 -Append }

$rustc = (& rustc --version 2>&1 | Out-String).Trim()
Write-Host ''
Write-Host "TigerSetup $Version release set$(if ($Rehearsal) { ' (REHEARSAL - a candidate, never a release)' }) at $OutputDirectory"
Write-Host "  source commit           $CommitSha"
Write-Host "  toolchain               $rustc"
foreach ($file in @(Get-ChildItem -LiteralPath $OutputDirectory -File | Sort-Object Name)) {
    Write-Host ('  {0,-40} {1,10}  {2}' -f $file.Name, $file.Length, (Get-TigerSetupFileSha256 $file.FullName))
}
Write-Host "  engine                  $($engine.engine_sha256)"
Write-Host "  loader                  $($engine.loader_sha256)"
Write-Host "  installer URL           $url"
foreach ($term in $terms) {
    $committed = Invoke-TigerSetupGit $repoRoot @('rev-parse', '--verify', '--quiet', "${CommitSha}:$($term.source)")
    $state = if ($committed.ok -and $committed.output -ceq $term.blob) { "as committed at $($CommitSha.Substring(0, 12))" } else { 'NOT YET COMMITTED - the working tree''s' }
    Write-Host ('  {0,-23} {1} {2}, {3}' -f $term.kind, $term.name, "blob $($term.blob)", $state)
    Write-Host "  $(' ' * 23) $(Get-TigerSetupReleaseAssetUrl -Version $Version -Name $term.name)"
}
Write-Host "  release-artifacts.json  sha256 $recordSha256"
exit 0
