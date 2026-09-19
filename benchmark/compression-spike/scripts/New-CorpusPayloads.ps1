#Requires -Version 7.0
<#
    .SYNOPSIS
    Materializes every corpus application's payload tree under
    benchmark/compression-spike/corpus/<App>/payload and records what was
    done in results/corpus-payloads.json.

    .DESCRIPTION
    Per results/corpus.json:
      canonical            the four benchmark payloads are used in place
                           (benchmark/canonical/<App>/payload), never copied;
      download             the pinned artifact (Acquire-Corpus.ps1) is
                           extracted with 7-Zip and normalized as the entry
                           below records;
      unavailable          recorded with the reason, no payload;
      local-tree           a release staging tree on this machine is copied
                           with the installer's own include/exclude rules;
      tigersetup-installer the shipped TigerSetup installer is decomposed
                           with `tiger-setup inspect --output-zip` and the
                           ZIP is expanded.
    Every normalization is recorded in the result file. Application
    repositories are read, never written.
#>
[CmdletBinding()]
param(
    [string] $App,
    [switch] $Force,
    [string] $SevenZip = 'C:\Program Files\7-Zip\7z.exe',
    [string] $TigerSetupBuilder
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $root '..\..')).Path
$manifest = Get-Content -LiteralPath (Join-Path $root 'results\corpus.json') -Raw | ConvertFrom-Json
$downloads = Join-Path $root 'corpus\downloads'
$corpus = Join-Path $root 'corpus'
if (-not $TigerSetupBuilder) { $TigerSetupBuilder = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe' }

function Reset-Directory([string] $Path) {
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
    $null = New-Item -ItemType Directory -Path $Path -Force
}

function Invoke-SevenZip([string[]] $Arguments) {
    $output = & $SevenZip @Arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "7-Zip failed: $output" }
}

function Measure-Tree([string] $Path) {
    $files = @(Get-ChildItem -LiteralPath $Path -Recurse -File)
    [ordered]@{ files = $files.Count; bytes = [int64] (($files | Measure-Object Length -Sum).Sum) }
}

$results = @()
foreach ($entry in $manifest.apps) {
    if ($App -and $entry.name -ne $App) { continue }
    $name = $entry.name
    $payload = Join-Path $corpus "$name\payload"
    $normalizations = @()
    $source = $null

    switch ($entry.source) {
        'unavailable' {
            Write-Host "$name : unavailable - $($entry.reason)"
            $results += [ordered]@{ app = $name; version = $entry.version; license = $entry.license; source = $null; payload = $null; files = 0; bytes = 0; normalizations = @($entry.reason) }
            $payload = $null
        }
        'canonical' {
            $payload = Join-Path $repoRoot "benchmark\canonical\$name\payload"
            if (-not (Test-Path -LiteralPath $payload)) { throw "$name : canonical payload missing at $payload (benchmark\scripts\New-CanonicalPayload.ps1)" }
            $source = "benchmark/canonical/$name/payload (benchmark/results/canonical-$name.json)"
            $normalizations += 'Reused the installer-technology benchmark''s canonical payload exactly, in place.'
        }
        'download' {
            $archive = Join-Path $downloads $entry.filename
            if (-not (Test-Path -LiteralPath $archive)) { throw "$name : $archive missing; run Acquire-Corpus.ps1" }
            $source = "$($entry.filename) (sha256 $($entry.sha256_published))"
            if ((Test-Path -LiteralPath $payload) -and -not $Force) {
                Write-Host "$name : payload already present"
                $normalizations += '(existing payload reused; run with -Force to rebuild)'
                break
            }
            $extracted = Join-Path $corpus "$name\_extracted"
            Reset-Directory $extracted
            Write-Host "$name : extracting $($entry.filename)"
            switch ($entry.kind) {
                'zip' { Invoke-SevenZip @('x', "-o$extracted", '-y', $archive) }
                '7z' { Invoke-SevenZip @('x', "-o$extracted", '-y', $archive) }
                '7z-sfx' { Invoke-SevenZip @('x', "-o$extracted", '-y', $archive) }
                'nsis' { Invoke-SevenZip @('x', "-o$extracted", '-y', $archive) }
                default { throw "$name : unknown kind $($entry.kind)" }
            }
            Reset-Directory $payload
            switch ($name) {
                'GitForWindows' {
                    # The SFX unpacks the tree itself (bin/, cmd/, mingw64/, usr/, ...).
                    Copy-Item -Path (Join-Path $extracted '*') -Destination $payload -Recurse -Force
                    $normalizations += 'Unpacked the PortableGit 7z-SFX with 7-Zip without running its stub; the tree is used as is.'
                }
                'VSCode' {
                    Copy-Item -Path (Join-Path $extracted '*') -Destination $payload -Recurse -Force
                    $normalizations += 'The archive has no wrapping folder; the tree is used as is (Code.exe, bin/, resources under the commit-named folder).'
                }
                'Wireshark' {
                    Get-ChildItem -LiteralPath $extracted -Force | Where-Object {
                        $_.Name -ne '$PLUGINSDIR' -and $_.Name -ne 'uninstall.exe' -and
                        $_.Name -notlike 'npcap-*.exe' -and $_.Name -notlike 'USBPcapSetup-*.exe'
                    } | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $payload -Recurse -Force }
                    $normalizations += 'Payload extracted from the official NSIS installer with 7-Zip''s NSIS reader (no official archive exists).'
                    $normalizations += 'Excluded $PLUGINSDIR (NSIS scratch), uninstall.exe (NSIS''s generated uninstaller), and the bundled third-party dependency installers npcap-*.exe and USBPcapSetup-*.exe (wireshark.nsi File directives; dependencies, not application files).'
                }
                'WinSCP' {
                    Copy-Item -Path (Join-Path $extracted '*') -Destination $payload -Recurse -Force
                    $normalizations += 'Four files at the archive root, used as is. This is WinSCP''s x86 build: the stable line ships no x64 binary.'
                }
                'Inkscape' {
                    $inner = @(Get-ChildItem -LiteralPath $extracted -Directory)
                    if ($inner.Count -eq 1) {
                        Copy-Item -Path (Join-Path $inner[0].FullName '*') -Destination $payload -Recurse -Force
                        $normalizations += "Flattened the archive's single nested `"$($inner[0].Name)\`" directory (archive packaging, not part of the app tree)."
                    }
                    else {
                        Copy-Item -Path (Join-Path $extracted '*') -Destination $payload -Recurse -Force
                        $normalizations += 'The archive has no single wrapping folder; the tree is used as is.'
                    }
                }
                'NotepadPlusPlus' {
                    Copy-Item -Path (Join-Path $extracted '*') -Destination $payload -Recurse -Force
                    $normalizations += 'The archive has no wrapping folder; the tree is used as is (updater/ is part of the shipped application).'
                }
                default { throw "$name : no extraction rule" }
            }
        }
        'local-tree' {
            $tree = $entry.path
            if (-not (Test-Path -LiteralPath $tree)) { throw "$name : $tree missing" }
            $source = $tree
            Reset-Directory $payload
            $includes = if ($entry.PSObject.Properties.Match('include').Count -gt 0) { @($entry.include) } else { @('*') }
            foreach ($inc in $includes) {
                Copy-Item -Path (Join-Path $tree $inc) -Destination $payload -Recurse -Force
            }
            if ($entry.PSObject.Properties.Match('exclude_patterns').Count -gt 0) {
                foreach ($pattern in $entry.exclude_patterns) {
                    Get-ChildItem -LiteralPath $payload -Recurse -File -Filter $pattern | Remove-Item -Force
                }
                $normalizations += "Excluded $($entry.exclude_patterns -join ', ') as the shipped installer's [Files] section does."
            }
            if ($entry.PSObject.Properties.Match('include').Count -gt 0) {
                $normalizations += "Copied only what the shipped installer's [Files] section names: $($entry.include -join ', ')."
            }
            $normalizations += $entry.note
        }
        'tigersetup-installer' {
            $installer = $entry.path
            if (-not (Test-Path -LiteralPath $installer)) { throw "$name : $installer missing" }
            if (-not (Test-Path -LiteralPath $TigerSetupBuilder)) { throw "tiger-setup.exe not found at $TigerSetupBuilder (cargo build --release)" }
            $source = "$installer (sha256 $((Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()))"
            $extracted = Join-Path $corpus "$name\_extracted"
            Reset-Directory $extracted
            $zip = Join-Path $extracted 'payload.zip'
            $null = & $TigerSetupBuilder inspect --json --output-zip $zip $installer
            if ($LASTEXITCODE -ne 0) { throw "$name : tiger-setup inspect failed on $installer" }
            Reset-Directory $payload
            Invoke-SevenZip @('x', "-o$payload", '-y', $zip)
            $normalizations += 'Payload is the ZIP payload block of the shipped TigerSetup installer, expanded (entry names are the install-relative paths; .tigersetup/ entries are the packaged actions the installer carries).'
            $normalizations += $entry.note
        }
        default { throw "$name : unknown source $($entry.source)" }
    }

    if ($null -eq $payload) { continue }
    $measure = Measure-Tree $payload
    Write-Host ("{0,-16} {1,6} files {2,14:N0} bytes  {3}" -f $name, $measure.files, $measure.bytes, $payload)
    $results += [ordered]@{
        app            = $name
        version        = $entry.version
        license        = $entry.license
        source         = $source
        payload        = $payload
        files          = $measure.files
        bytes          = $measure.bytes
        normalizations = $normalizations
    }
}

$out = Join-Path $root 'results\corpus-payloads.json'
$existing = @()
if ((Test-Path -LiteralPath $out) -and $App) {
    $existing = @(Get-Content -LiteralPath $out -Raw | ConvertFrom-Json | Where-Object { $_.app -ne $App })
}
@($existing + $results) | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $out -Encoding utf8
Write-Host "Wrote $out"
