#Requires -Version 7.0
<#
    .SYNOPSIS
    Extracts each pinned release artifact (benchmark/downloads, from
    Acquire-Downloads.ps1) and normalizes it into the app's canonical
    application payload (benchmark/canonical/<App>/payload/), writing a
    SHA-256 inventory of every file (benchmark/results/canonical-<App>.json).

    .DESCRIPTION
    Every normalization this script performs is recorded in the app's entry
    below and in its inventory file, so the report can state the complete,
    auditable set of adjustments rather than silent cleanup. For a given
    app, all three technologies install this payload byte for byte.

    ShareX      : the portable zip, minus the "Portable" marker file, which
                  only exists to put the portable build into portable
                  (settings-beside-exe) mode; a real installed copy never
                  has it.
    WinMerge    : the -exe.zip extracts one nested "WinMerge\" directory;
                  that nesting is flattened (a zip-packaging artifact, not
                  part of the application tree).
    VLC         : the win64 zip extracts one nested "vlc-3.0.23\" directory,
                  flattened for the same reason; its "msi\" directory (VLC's
                  own WiX/.wxs sources for building an MSI) is excluded as an
                  alternate installer technology's build inputs, not an
                  application file.
    qBittorrent : extracted directly from the official NSIS installer with
                  7-Zip's NSIS reader, because no portable archive exists.
                  $PLUGINSDIR is NSIS's own scratch directory and is never
                  written to $INSTDIR, so it is excluded; uninst.exe is
                  NSIS's generated uninstaller, not an application file, and
                  every benchmark technology generates its own uninstaller,
                  so it is excluded. qbittorrent.pdb IS kept: the upstream
                  installer.nsh globs the whole build output into $INSTDIR
                  and the file really is inside the official installer
                  beside qbittorrent.exe — observed upstream behaviour,
                  reported rather than corrected.

    Extraction is skipped for an app whose _extracted directory already
    exists unless -Force is given; the payload and the inventory are always
    rebuilt from it.

    .EXAMPLE
    pwsh -File benchmark\scripts\New-CanonicalPayload.ps1
    pwsh -File benchmark\scripts\New-CanonicalPayload.ps1 -App qBittorrent -Force
#>
[CmdletBinding()]
param(
    [string] $App,
    [switch] $Force,
    [string] $SevenZip = 'C:\Program Files\7-Zip\7z.exe'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'Get-AppsManifest.ps1')
$manifest = Get-BenchmarkAppsManifest
$benchmarkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$downloadRoot = Join-Path $benchmarkRoot 'downloads'
$canonicalRoot = Join-Path $benchmarkRoot 'canonical'
$resultsRoot = Join-Path $benchmarkRoot 'results'
$null = New-Item -ItemType Directory -Path $resultsRoot -Force

function Get-Artifact {
    param([string] $Name, [string] $Kind)
    $entry = @($manifest.apps | Where-Object { $_.name -eq $Name }) | Select-Object -First 1
    if ($null -eq $entry) { throw "apps.json names no application '$Name'." }
    $artifact = $entry.artifacts.$Kind
    $path = Join-Path $downloadRoot "$Name\$($artifact.filename)"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "'$path' is missing; run Acquire-Downloads.ps1 first." }
    $path
}

function Expand-Once {
    <# Extracts an archive into _extracted once; -Force starts over. #>
    param([string] $Name, [string] $Archive, [switch] $Nsis)
    $target = Join-Path $canonicalRoot "$Name\_extracted"
    if ((Test-Path -LiteralPath $target) -and -not $Force) {
        Write-Host "$Name : using the existing extraction at $target"
        return $target
    }
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Recurse -Force }
    $null = New-Item -ItemType Directory -Path $target -Force
    Write-Host "$Name : extracting $(Split-Path -Leaf $Archive)"
    if ($Nsis) {
        if (-not (Test-Path -LiteralPath $SevenZip -PathType Leaf)) { throw "7-Zip is needed to read an NSIS installer; '$SevenZip' does not exist (pass -SevenZip)." }
        $output = & $SevenZip x "-o$target" -y $Archive 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) { throw "7-Zip failed on '$Archive': $output" }
    }
    else {
        Expand-Archive -LiteralPath $Archive -DestinationPath $target -Force
    }
    $target
}

function New-Inventory {
    param([string] $PayloadRoot)
    $files = Get-ChildItem -LiteralPath $PayloadRoot -Recurse -File
    $entries = foreach ($file in ($files | Sort-Object { $_.FullName })) {
        $relative = $file.FullName.Substring($PayloadRoot.Length + 1) -replace '\\', '/'
        [PSCustomObject][ordered]@{
            path   = $relative
            bytes  = $file.Length
            sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    @($entries)
}

function Write-CanonicalResult {
    param([string] $Name, [string] $PayloadRoot, [string] $Source, [string[]] $Normalizations)
    $inventory = New-Inventory -PayloadRoot $PayloadRoot
    $totalBytes = ($inventory | Measure-Object -Property bytes -Sum).Sum
    $result = [ordered]@{
        app            = $Name
        source         = (Split-Path -Leaf $Source)
        sourceSha256   = (Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash.ToLowerInvariant()
        payloadRoot    = $PayloadRoot
        fileCount      = $inventory.Count
        totalBytes     = [int64] $totalBytes
        normalizations = $Normalizations
        inventory      = $inventory
    }
    $outJson = Join-Path $resultsRoot "canonical-$Name.json"
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $outJson -Encoding utf8
    Write-Host "$Name canonical payload: $($inventory.Count) files, $totalBytes bytes -> $outJson"
}

function Reset-Directory {
    param([string] $Path)
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
    $null = New-Item -ItemType Directory -Path $Path -Force
}

if (-not $App -or $App -eq 'ShareX') {
    $archive = Get-Artifact 'ShareX' 'portable'
    $src = Expand-Once -Name 'ShareX' -Archive $archive
    $payload = Join-Path $canonicalRoot 'ShareX\payload'
    Reset-Directory $payload
    Get-ChildItem -LiteralPath $src -Force | Where-Object { $_.Name -ne 'Portable' } |
        ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $payload -Recurse -Force }
    Write-CanonicalResult -Name 'ShareX' -PayloadRoot $payload -Source $archive -Normalizations @(
        'Removed the "Portable" marker file (portable-mode flag; a real install never has it).'
    )
}

if (-not $App -or $App -eq 'WinMerge') {
    $archive = Get-Artifact 'WinMerge' 'portable'
    $src = Join-Path (Expand-Once -Name 'WinMerge' -Archive $archive) 'WinMerge'
    $payload = Join-Path $canonicalRoot 'WinMerge\payload'
    Reset-Directory $payload
    Copy-Item -Path (Join-Path $src '*') -Destination $payload -Recurse -Force
    Write-CanonicalResult -Name 'WinMerge' -PayloadRoot $payload -Source $archive -Normalizations @(
        'Flattened the archive''s nested "WinMerge\" directory (zip-packaging artifact, not part of the app tree).'
    )
}

if (-not $App -or $App -eq 'VLC') {
    $archive = Get-Artifact 'VLC' 'portable'
    $src = Join-Path (Expand-Once -Name 'VLC' -Archive $archive) 'vlc-3.0.23'
    $payload = Join-Path $canonicalRoot 'VLC\payload'
    Reset-Directory $payload
    Copy-Item -Path (Join-Path $src '*') -Destination $payload -Recurse -Force
    Remove-Item -LiteralPath (Join-Path $payload 'msi') -Recurse -Force -ErrorAction SilentlyContinue
    Write-CanonicalResult -Name 'VLC' -PayloadRoot $payload -Source $archive -Normalizations @(
        'Flattened the archive''s nested "vlc-3.0.23\" directory (zip-packaging artifact, not part of the app tree).',
        'Excluded msi\ (VLC''s own WiX/.wxs source for building an MSI -- an alternate installer technology''s build sources shipped inside the zip, not an application file).'
    )
}

if (-not $App -or $App -eq 'qBittorrent') {
    $archive = Get-Artifact 'qBittorrent' 'installer'
    $src = Expand-Once -Name 'qBittorrent' -Archive $archive -Nsis
    $payload = Join-Path $canonicalRoot 'qBittorrent\payload'
    Reset-Directory $payload
    Get-ChildItem -LiteralPath $src -Force | Where-Object { $_.Name -notin @('$PLUGINSDIR', 'uninst.exe') } |
        ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $payload -Recurse -Force }
    Write-CanonicalResult -Name 'qBittorrent' -PayloadRoot $payload -Source $archive -Normalizations @(
        'Payload derived by extracting the official NSIS installer (7-Zip''s NSIS reader), the only available official artifact.',
        'Excluded $PLUGINSDIR (NSIS installer scratch space, never written to $INSTDIR).',
        'Excluded uninst.exe (NSIS''s generated uninstaller stub; each benchmark technology generates its own uninstaller natively).',
        'Kept qbittorrent.pdb (debug symbols): present in the official installer beside qbittorrent.exe, as the upstream installer.nsh File /r glob ships it -- observed upstream behaviour, not a benchmark artifact.'
    )
}
