#Requires -Version 7
<#
.SYNOPSIS
Generates the deterministic payload of the TigerSetupTestApp synthetic package.

.DESCRIPTION
Writes packages/test-app/<version>/payload (the core file set), payload-extras
(the `extras` component) and payload-tools (the `tools` PATH mode's files) for
1.0.0 and 1.1.0: about sixty core files in nested directories, several of
2-6 MB so that a write window is real, with content derived from a fixed seed
per path and version so that every machine produces identical bytes.

Version 1.1.0 overlaps 1.0.0: most core files are unchanged, a few change
content, a few are removed and a few are added, so that an upgrade between the
two versions exercises keep / replace / remove / install; the component files
are re-seeded, so an upgrade replaces them.

The embedded prerequisite (dependencies/TigerSetupTestPrereq.exe) is not a
payload file: Build-Package.ps1 copies it from the release build.

.PARAMETER Root
The packages/test-app directory. Defaults to the script's own directory.
#>
[CmdletBinding()]
param(
    [string]$Root = $PSScriptRoot
)

$ErrorActionPreference = 'Stop'

function Get-Seed {
    param([string]$Text)
    # FNV-1a over the UTF-8 bytes, folded to a 31-bit .NET Random seed.
    # (Reduce with modulo: a 0xFFFFFFFF literal is Int32 -1 in PowerShell.)
    [uint64]$hash = 2166136261
    foreach ($byte in [System.Text.Encoding]::UTF8.GetBytes($Text)) {
        $hash = $hash -bxor [uint64]$byte
        $hash = ($hash * 16777619) % 4294967296
    }
    return [int]($hash % 2147483648)
}

function Write-BinaryFile {
    param([string]$Path, [int]$Size, [string]$SeedText)
    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $random = [System.Random]::new((Get-Seed $SeedText))
    $bytes = [byte[]]::new($Size)
    $random.NextBytes($bytes)
    [System.IO.File]::WriteAllBytes($Path, $bytes)
}

function Write-TextFile {
    param([string]$Path, [int]$Lines, [string]$SeedText)
    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $builder = [System.Text.StringBuilder]::new()
    for ($i = 1; $i -le $Lines; $i++) {
        [void]$builder.AppendLine("TigerSetupTestApp $SeedText line $i of $Lines")
    }
    [System.IO.File]::WriteAllText($Path, $builder.ToString(), [System.Text.UTF8Encoding]::new($false))
}

# The 1.0.0 layout. Sorted by install-relative path, the files become journal
# operations 12 onwards (operations 1-11 create the root and ten
# directories), so data/big-4.bin is operation 30: the operation the lab's
# recovery scenarios interrupt.
$layout = [ordered]@{
    'CHANGELOG.md'            = @{ Kind = 'text'; Lines = 60 }
    'LICENSE.txt'             = @{ Kind = 'text'; Lines = 21 }
    'bin/TigerSetupTestApp.exe' = @{ Kind = 'binary'; Size = 180000 }
    'doc/readme.md'           = @{ Kind = 'text'; Lines = 120 }
    'doc/legacy/old-01.md'    = @{ Kind = 'text'; Lines = 50 }
    'doc/legacy/old-02.md'    = @{ Kind = 'text'; Lines = 52 }
    'locale/en-US.json'       = @{ Kind = 'text'; Lines = 80 }
    'locale/pl-PL.json'       = @{ Kind = 'text'; Lines = 88 }
}
1..8 | ForEach-Object { $layout["bin/lib-{0:D2}.dll" -f $_] = @{ Kind = 'binary'; Size = 20000 + $_ * 7000 } }
1..4 | ForEach-Object { $layout["bin/x64/native-$_.bin"] = @{ Kind = 'binary'; Size = $(if ($_ -eq 3) { 3MB } else { 60000 * $_ }) } }
$bigSizes = @(2MB, 3MB, 4MB, 6MB, 4MB)
1..5 | ForEach-Object { $layout["data/big-$_.bin"] = @{ Kind = 'binary'; Size = $bigSizes[$_ - 1] } }
$layout['data/tables/index.dat'] = @{ Kind = 'binary'; Size = 8000 }
1..20 | ForEach-Object { $layout["data/tables/t-{0:D2}.dat" -f $_] = @{ Kind = 'binary'; Size = 10000 + $_ * 2000 } }
1..10 | ForEach-Object { $layout["doc/manual/ch-{0:D2}.md" -f $_] = @{ Kind = 'text'; Lines = 100 + $_ * 6 } }
1..8 | ForEach-Object { $layout["lib/plugins/p-{0:D2}.plug" -f $_] = @{ Kind = 'binary'; Size = 30000 + $_ * 1000 } }

# 1.1.0: unchanged unless listed here. Removing doc/legacy/* empties a
# directory only 1.0.0 needed; data/extra/notes.txt needs a new one.
$changed = @('CHANGELOG.md', 'bin/TigerSetupTestApp.exe', 'bin/lib-03.dll', 'data/big-4.bin', 'doc/readme.md')
$removed = @('bin/lib-08.dll', 'doc/legacy/old-01.md', 'doc/legacy/old-02.md', 'doc/manual/ch-10.md', 'lib/plugins/p-08.plug')
$added = [ordered]@{
    'bin/lib-09.dll'       = @{ Kind = 'binary'; Size = 83000 }
    'data/big-6.bin'       = @{ Kind = 'binary'; Size = 3MB }
    'data/extra/notes.txt' = @{ Kind = 'text'; Lines = 30 }
    'doc/manual/ch-11.md'  = @{ Kind = 'text'; Lines = 166 }
    'locale/de-DE.json'    = @{ Kind = 'text'; Lines = 84 }
}

# The components: the same files in both versions, re-seeded per version.
$components = [ordered]@{
    'payload-extras' = [ordered]@{
        'extras/notes.txt'        = @{ Kind = 'text'; Lines = 40 }
        'extras/data/samples.bin' = @{ Kind = 'binary'; Size = 1MB }
    }
    'payload-tools' = [ordered]@{
        'tools/tsta-tool.exe' = @{ Kind = 'binary'; Size = 90000 }
        'tools/tsta-tool.txt' = @{ Kind = 'text'; Lines = 24 }
    }
}

function Write-Payload {
    param([string]$Version, [System.Collections.Specialized.OrderedDictionary]$Files, [string]$Directory = 'payload')
    $payloadRoot = Join-Path $Root $Version $Directory
    if (Test-Path $payloadRoot) { Remove-Item -Recurse -Force $payloadRoot }
    $total = 0
    foreach ($relative in $Files.Keys) {
        $spec = $Files[$relative]
        $seedVersion = if ($Directory -ne 'payload' -or ($Version -ne '1.0.0' -and ($changed -contains $relative -or -not $layout.Contains($relative)))) { $Version } else { '1.0.0' }
        $seedText = "$relative|$seedVersion"
        $path = Join-Path $payloadRoot ($relative -replace '/', [System.IO.Path]::DirectorySeparatorChar)
        if ($spec.Kind -eq 'binary') {
            Write-BinaryFile -Path $path -Size $spec.Size -SeedText $seedText
        } else {
            Write-TextFile -Path $path -Lines $spec.Lines -SeedText $seedText
        }
        $total += (Get-Item $path).Length
    }
    Write-Host ("{0}: {1} files, {2:N0} bytes -> {3}" -f $Version, $Files.Count, $total, $payloadRoot)
}

Write-Payload -Version '1.0.0' -Files $layout

$next = [ordered]@{}
foreach ($relative in $layout.Keys) {
    if ($removed -notcontains $relative) { $next[$relative] = $layout[$relative] }
}
foreach ($relative in $added.Keys) { $next[$relative] = $added[$relative] }
Write-Payload -Version '1.1.0' -Files $next

foreach ($version in @('1.0.0', '1.1.0')) {
    foreach ($directory in $components.Keys) {
        Write-Payload -Version $version -Files $components[$directory] -Directory $directory
    }
}
