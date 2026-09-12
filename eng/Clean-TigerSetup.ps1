<#
    .SYNOPSIS
    Measures and removes the disposable state TigerSetup generates outside
    Cargo's normal build output.

    .DESCRIPTION
    `cargo clean` removes the workspace's build output, but the lab rows, the
    package builds and the synthetic test payload leave gigabytes elsewhere.
    This script knows which TigerSetup-generated locations are disposable and
    cleans exactly those, by an explicit policy rather than by `.gitignore`:
    an ignored path is not automatically garbage.

    Three categories, each named in the report:

      routine    removed by default - Cargo's target directory (through
                 `cargo clean`, so a configured target directory is honored),
                 .is_screenshots\, lab\results\, packages\test-app\*\payload\,
                 packages\TigerMarkView\source\ and publish\, and
                 packages\tigersetup\stage\;
      retained   artifacts\ - removed only with -All, because it may hold the
                 verified release installers a developer wants to keep;
      kept       .claude\worktrees\ - measured, never removed: a worktree
                 there may be active work.

    Everything else is out of scope: .git\, tracked sources, documentation,
    package definitions, .cargo\config.toml, the agent definitions, and
    anything outside the repository root. A target that is a junction or
    symbolic link, that lies behind one, or that contains one is skipped with
    a message and is never followed; the report then says the cleanup was not
    complete and the exit code is 1.

    -TestState additionally removes the registry namespace the Rust tests
    relocate their registry roots under, HKCU\Software\TigerSetupTests. It is
    machine state rather than repository state, so it is never touched unless
    asked for, and only that key is removed.

    -Measure deletes nothing and runs no Cargo command; it reports the size of
    every location above and the number of subkeys under the test namespace.
    -WhatIf reports what a cleanup would remove and removes nothing.

    Each target ends in one state: removed, absent, kept, skipped, partial or
    failed. Reclaimed space is measured - the size before minus what remains -
    so a locked file is reported as remaining rather than as reclaimed.
    Independent targets continue after one fails; the exit code is 0 only when
    every requested target was removed or was already absent.

    .PARAMETER Measure
    Report sizes only. Nothing is deleted and Cargo is not invoked.

    .PARAMETER All
    Also remove the retained artifacts\ directory.

    .PARAMETER TestState
    Also remove HKCU\Software\TigerSetupTests, the registry namespace the
    Rust tests write under.

    .PARAMETER RepositoryRoot
    The TigerSetup checkout to clean. Defaults to the parent of the directory
    holding this script; a root without Cargo.toml and AGENTS.md is refused.

    .PARAMETER PassThru
    Also emit the result object (targets, sizes, states, reclaimed bytes,
    success) to the pipeline, for scripts and tests.

    .EXAMPLE
    pwsh -File eng\Clean-TigerSetup.ps1 -Measure

    .EXAMPLE
    pwsh -File eng\Clean-TigerSetup.ps1

    .EXAMPLE
    pwsh -File eng\Clean-TigerSetup.ps1 -All -TestState

    .EXAMPLE
    pwsh -File eng\Clean-TigerSetup.ps1 -All -WhatIf
#>
#Requires -Version 7.0
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [switch] $Measure,
    [switch] $All,
    [switch] $TestState,
    [string] $RepositoryRoot,
    [switch] $PassThru
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The cleanup policy. Paths are repository-relative; `glob` entries name the
# directory whose immediate subdirectories each hold one `child`. This list,
# not .gitignore, decides what is disposable.
$policy = @(
    [pscustomobject]@{ name = 'target'; category = 'routine'; kind = 'cargo'; note = 'Cargo target directory, cleaned with cargo clean' }
    [pscustomobject]@{ name = '.is_screenshots'; category = 'routine'; kind = 'directory'; path = '.is_screenshots' }
    [pscustomobject]@{ name = 'lab/results'; category = 'routine'; kind = 'directory'; path = 'lab\results' }
    [pscustomobject]@{ name = 'packages/test-app/*/payload'; category = 'routine'; kind = 'glob'; path = 'packages\test-app'; child = 'payload' }
    [pscustomobject]@{ name = 'packages/TigerMarkView/source'; category = 'routine'; kind = 'directory'; path = 'packages\TigerMarkView\source' }
    [pscustomobject]@{ name = 'packages/TigerMarkView/publish'; category = 'routine'; kind = 'directory'; path = 'packages\TigerMarkView\publish' }
    [pscustomobject]@{ name = 'packages/tigersetup/stage'; category = 'routine'; kind = 'directory'; path = 'packages\tigersetup\stage' }
    [pscustomobject]@{ name = 'artifacts'; category = 'retained'; kind = 'directory'; path = 'artifacts'; note = 'kept unless -All: may hold verified release installers' }
    [pscustomobject]@{ name = '.claude/worktrees'; category = 'kept'; kind = 'directory'; path = '.claude\worktrees'; note = 'never removed: may hold active worktrees' }
)

# The registry namespace the Rust tests relocate their registry roots under
# (crates/tigersetup-engine/src/win/registry.rs, `Roots::relocated`).
$testRegistryPath = 'HKCU:\Software\TigerSetupTests'
$testRegistryName = 'HKCU\Software\TigerSetupTests'

function Format-Bytes {
    param([Parameter(Mandatory)] [long] $Bytes)
    if ($Bytes -ge 1GB) { return ('{0:N1} GB' -f ($Bytes / 1GB)) }
    if ($Bytes -ge 1MB) { return ('{0:N1} MB' -f ($Bytes / 1MB)) }
    if ($Bytes -ge 1KB) { return ('{0:N1} KB' -f ($Bytes / 1KB)) }
    return "$Bytes B"
}

function Test-ReparsePoint {
    param([Parameter(Mandatory)] [string] $Path)
    $item = [System.IO.FileInfo]::new($Path)
    if (-not $item.Exists) { $item = [System.IO.DirectoryInfo]::new($Path) }
    if (-not $item.Exists) { return $false }
    return ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0
}

function Get-TreeSize {
    <#
        The bytes under a directory, without following reparse points: a
        junction or symbolic link anywhere in the tree is listed rather than
        entered, and the caller decides what that means.
    #>
    param([Parameter(Mandatory)] [string] $Path)

    $bytes = [long] 0
    $files = 0
    $reparsePoints = [System.Collections.Generic.List[string]]::new()
    if (-not [System.IO.Directory]::Exists($Path)) {
        return [pscustomobject]@{ bytes = $bytes; files = $files; reparsePoints = $reparsePoints }
    }

    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push($Path)
    while ($pending.Count -gt 0) {
        $directory = [System.IO.DirectoryInfo]::new($pending.Pop())
        foreach ($entry in $directory.EnumerateFileSystemInfos()) {
            if (($entry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                $reparsePoints.Add($entry.FullName)
                continue
            }
            if ($entry -is [System.IO.DirectoryInfo]) {
                $pending.Push($entry.FullName)
            }
            else {
                $bytes += $entry.Length
                $files++
            }
        }
    }
    [pscustomobject]@{ bytes = $bytes; files = $files; reparsePoints = $reparsePoints }
}

function Test-InsideRoot {
    <# True when a full path is strictly below the repository root. #>
    param([Parameter(Mandatory)] [string] $Root, [Parameter(Mandatory)] [string] $Path)
    $full = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    return $full.StartsWith($Root + '\', [System.StringComparison]::OrdinalIgnoreCase)
}

function Get-LinkOnPath {
    <#
        The first junction or symbolic link on the way from the root down to
        a repository-relative path, or $null. A target behind a link may
        resolve anywhere, so it is not cleaned.
    #>
    param([Parameter(Mandatory)] [string] $Root, [Parameter(Mandatory)] [string] $RelativePath)
    $current = $Root
    foreach ($part in $RelativePath.Split('\', [System.StringSplitOptions]::RemoveEmptyEntries)) {
        $current = Join-Path $current $part
        if (Test-ReparsePoint -Path $current) { return $current }
    }
    return $null
}

function Get-CargoTargetDirectory {
    <#
        Cargo's own answer for the target directory, so a directory
        configured through .cargo/config.toml or CARGO_TARGET_DIR is measured
        and cleaned rather than an assumed `target`.
    #>
    param([Parameter(Mandatory)] [string] $Root)
    $cargo = Get-Command cargo -CommandType Application -ErrorAction SilentlyContinue
    if (-not $cargo) { return [pscustomobject]@{ path = $null; error = 'cargo is not on PATH' } }
    Push-Location -LiteralPath $Root
    try {
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { $output = @(& cargo metadata --no-deps --format-version 1 2>&1) }
        finally { $ErrorActionPreference = $previous }
        # stdout carries the JSON; stderr (error records here) carries warnings.
        $stdout = @($output | Where-Object { $_ -isnot [System.Management.Automation.ErrorRecord] } | ForEach-Object { "$_" })
        $stderr = @($output | Where-Object { $_ -is [System.Management.Automation.ErrorRecord] } | ForEach-Object { "$_" })
        if ($LASTEXITCODE -ne 0) {
            return [pscustomobject]@{ path = $null; error = "cargo metadata failed ($LASTEXITCODE): $(($stderr | Select-Object -Last 1))" }
        }
        $metadata = ($stdout -join "`n") | ConvertFrom-Json
        return [pscustomobject]@{ path = [System.IO.Path]::GetFullPath($metadata.target_directory).TrimEnd('\'); error = $null }
    }
    finally { Pop-Location }
}

function Invoke-CargoClean {
    <# `cargo clean` from the repository root; the exit code is the verdict. #>
    param([Parameter(Mandatory)] [string] $Root)
    Push-Location -LiteralPath $Root
    try {
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { $output = & cargo clean 2>&1 | ForEach-Object { "$_" } }
        finally { $ErrorActionPreference = $previous }
        return [pscustomobject]@{ exitCode = $LASTEXITCODE; output = @($output) }
    }
    finally { Pop-Location }
}

function Resolve-CleanupTarget {
    <#
        One policy entry resolved against the root: the absolute paths it
        stands for (existing ones only), their size, and anything that makes
        the entry unsafe to remove. Discovery only; nothing is deleted here.
    #>
    param([Parameter(Mandatory)] [string] $Root, [Parameter(Mandatory)] [pscustomobject] $Entry)

    $target = [pscustomobject]@{
        name = $Entry.name
        category = $Entry.category
        kind = $Entry.kind
        note = $(if ($Entry.PSObject.Properties['note']) { $Entry.note } else { '' })
        paths = @()
        bytesBefore = [long] 0
        bytesAfter = [long] 0
        files = 0
        state = 'absent'
        message = ''
    }

    $candidates = @()
    switch ($Entry.kind) {
        'cargo' {
            $resolved = Get-CargoTargetDirectory -Root $Root
            if ($resolved.error) {
                $target.state = 'unresolved'
                $target.message = $resolved.error
                return $target
            }
            $target.note = "Cargo target directory $($resolved.path)"
            if (-not (Test-InsideRoot -Root $Root -Path $resolved.path)) {
                $target.note = "Cargo target directory $($resolved.path) is outside the repository; cargo clean is not run on it"
                $target.paths = @()
                $target.state = 'skipped'
                $target.message = 'the Cargo target directory lies outside the repository root'
                $size = Get-TreeSize -Path $resolved.path
                $target.bytesBefore = $size.bytes
                $target.bytesAfter = $size.bytes
                return $target
            }
            $candidates = @($resolved.path)
        }
        'directory' { $candidates = @(Join-Path $Root $Entry.path) }
        'glob' {
            $parent = Join-Path $Root $Entry.path
            if ([System.IO.Directory]::Exists($parent)) {
                foreach ($version in [System.IO.DirectoryInfo]::new($parent).EnumerateDirectories()) {
                    $candidates += Join-Path $version.FullName $Entry.child
                }
            }
        }
    }

    $existing = @()
    $problems = [System.Collections.Generic.List[string]]::new()
    foreach ($candidate in $candidates) {
        if (-not (Test-InsideRoot -Root $Root -Path $candidate)) {
            $problems.Add("$candidate is outside the repository root")
            continue
        }
        $relative = [System.IO.Path]::GetFullPath($candidate).Substring($Root.Length + 1)
        $link = Get-LinkOnPath -Root $Root -RelativePath $relative
        if ($link) {
            if (-not [System.IO.Directory]::Exists($candidate) -and -not [System.IO.File]::Exists($candidate)) { continue }
            $problems.Add("$link is a junction or symbolic link and is not followed")
            continue
        }
        if ([System.IO.File]::Exists($candidate)) {
            $problems.Add("$candidate is a file where a directory was expected")
            continue
        }
        if (-not [System.IO.Directory]::Exists($candidate)) { continue }
        $size = Get-TreeSize -Path $candidate
        foreach ($point in $size.reparsePoints) { $problems.Add("$point is a junction or symbolic link inside the target and is not followed") }
        $target.bytesBefore += $size.bytes
        $target.files += $size.files
        $existing += $candidate
    }

    $target.paths = $existing
    $target.bytesAfter = $target.bytesBefore
    if ($problems.Count -gt 0) {
        $target.state = 'skipped'
        $target.message = ($problems -join '; ')
    }
    elseif ($existing.Count -gt 0) {
        $target.state = 'present'
    }
    return $target
}

function Remove-CleanupTarget {
    <#
        Removes one resolved target that Resolve-CleanupTarget found safe and
        present, then re-measures what is left so the reclaimed size is what
        actually went away. Honors -WhatIf through the caller's ShouldProcess.
    #>
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [pscustomobject] $Target,
        [Parameter(Mandatory)] [System.Management.Automation.PSCmdlet] $Cmdlet
    )

    if ($Target.state -ne 'present') { return }

    $errors = [System.Collections.Generic.List[string]]::new()
    $attempted = $false
    if ($Target.kind -eq 'cargo') {
        if ($Cmdlet.ShouldProcess($Target.paths[0], 'cargo clean')) {
            $attempted = $true
            $run = Invoke-CargoClean -Root $Root
            foreach ($line in $run.output) { if ($line) { Write-Host "    cargo: $line" } }
            if ($run.exitCode -ne 0) { $errors.Add("cargo clean exited with $($run.exitCode)") }
        }
    }
    else {
        foreach ($path in $Target.paths) {
            if (-not $Cmdlet.ShouldProcess($path, 'Remove directory')) { continue }
            $attempted = $true
            try { Remove-Item -LiteralPath $path -Recurse -Force }
            catch { $errors.Add($_.Exception.Message) }
        }
    }

    if (-not $attempted) {
        $Target.state = 'whatif'
        return
    }

    $remaining = [long] 0
    foreach ($path in $Target.paths) { $remaining += (Get-TreeSize -Path $path).bytes }
    $Target.bytesAfter = $remaining
    $stillThere = @($Target.paths | Where-Object { [System.IO.Directory]::Exists($_) -or [System.IO.File]::Exists($_) })

    if ($errors.Count -gt 0) {
        $Target.state = $(if ($remaining -lt $Target.bytesBefore) { 'partial' } else { 'failed' })
        $Target.message = ($errors -join '; ')
    }
    elseif ($stillThere.Count -gt 0 -and $remaining -gt 0) {
        $Target.state = 'partial'
        $Target.message = 'files remain after removal'
    }
    else {
        # An empty directory shell left behind by a tool that reported success
        # holds no bytes; the space is reclaimed.
        $Target.state = 'removed'
    }
}

function Get-TestRegistryState {
    param([Parameter(Mandatory)] [string] $Path)
    if (-not (Test-Path -LiteralPath $Path)) { return [pscustomobject]@{ present = $false; subkeys = 0 } }
    $subkeys = (Get-ChildItem -LiteralPath $Path -ErrorAction SilentlyContinue | Measure-Object).Count
    [pscustomobject]@{ present = $true; subkeys = $subkeys }
}

# ---------------------------------------------------------------------------

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) { $RepositoryRoot = Split-Path -Parent $PSScriptRoot }
$root = [System.IO.Path]::GetFullPath((Resolve-Path -LiteralPath $RepositoryRoot).Path).TrimEnd('\')
foreach ($marker in @('Cargo.toml', 'AGENTS.md')) {
    if (-not [System.IO.File]::Exists((Join-Path $root $marker))) {
        throw "$root is not a TigerSetup repository root ($marker is missing); pass -RepositoryRoot."
    }
}

$mode = $(if ($Measure) { 'measure' } elseif ($WhatIfPreference) { 'whatif' } else { 'clean' })
Write-Host "TigerSetup cleanup ($mode)  $root"
Write-Host ''

$targets = @(foreach ($entry in $policy) { Resolve-CleanupTarget -Root $root -Entry $entry })
$registry = Get-TestRegistryState -Path $testRegistryPath
$registryState = $(if ($registry.present) { 'present' } else { 'absent' })
$registryMessage = ''

$requested = @($targets | Where-Object {
        $_.category -eq 'routine' -or ($All -and $_.category -eq 'retained')
    })

if (-not $Measure) {
    foreach ($target in $requested) {
        Remove-CleanupTarget -Root $root -Target $target -Cmdlet $PSCmdlet
    }
    $requestedNames = @($requested | ForEach-Object { $_.name })
    foreach ($target in $targets) {
        if ($target.state -eq 'present' -and $requestedNames -notcontains $target.name) { $target.state = 'kept' }
    }

    if ($TestState) {
        if (-not $registry.present) {
            $registryState = 'absent'
        }
        elseif ($PSCmdlet.ShouldProcess($testRegistryName, 'Remove registry key')) {
            try {
                Remove-Item -LiteralPath $testRegistryPath -Recurse -Force
                $registryState = $(if (Test-Path -LiteralPath $testRegistryPath) { 'failed' } else { 'removed' })
                if ($registryState -eq 'failed') { $registryMessage = 'the key still exists after removal' }
            }
            catch {
                $registryState = 'failed'
                $registryMessage = $_.Exception.Message
            }
        }
        else {
            $registryState = 'whatif'
        }
    }
}
else {
    foreach ($target in $targets) { if ($target.state -eq 'present') { $target.state = 'measured' } }
}

# --- the report -------------------------------------------------------------

$nameWidth = ($targets | ForEach-Object { $_.name.Length } | Measure-Object -Maximum).Maximum
$nameWidth = [Math]::Max($nameWidth, $testRegistryName.Length)
foreach ($target in $targets) {
    $before = $(if ($target.state -eq 'absent') { '-' } else { Format-Bytes $target.bytesBefore })
    $line = '  {0} {1,10}' -f $target.name.PadRight($nameWidth), $before
    if ($mode -eq 'clean') {
        $after = $(if ($target.state -in @('removed', 'partial', 'failed')) { Format-Bytes $target.bytesAfter } else { '' })
        $line += ' -> {0,-8}' -f $after
    }
    $line += '  {0,-9} {1}' -f $target.category, $target.state
    if ($target.note -and ($mode -eq 'measure' -or $target.state -in @('kept', 'skipped', 'unresolved'))) { $line += "  ($($target.note))" }
    Write-Host $line
    if ($target.message) { Write-Host "      $($target.message)" }
}

$registrySummary = $(if ($registry.present) { "$($registry.subkeys) subkey(s)" } else { '-' })
$registryLine = '  {0} {1,10}{2}  {3,-9} {4}' -f $testRegistryName.PadRight($nameWidth), $registrySummary, $(if ($mode -eq 'clean') { ' ' * 12 } else { '' }), 'test', $registryState
if (-not $TestState) { $registryLine += '  (cleaned only with -TestState)' }
Write-Host $registryLine
if ($registryMessage) { Write-Host "      $registryMessage" }

Write-Host ('  ' + ('-' * ($nameWidth + 40)))
$routineBytes = [long] (($targets | Where-Object { $_.category -eq 'routine' } | Measure-Object -Property bytesBefore -Sum).Sum)
$retainedBytes = [long] (($targets | Where-Object { $_.category -eq 'retained' } | Measure-Object -Property bytesBefore -Sum).Sum)
$keptBytes = [long] (($targets | Where-Object { $_.category -eq 'kept' } | Measure-Object -Property bytesBefore -Sum).Sum)
Write-Host ('  {0} {1,10}' -f 'Routine disposable'.PadRight($nameWidth), (Format-Bytes $routineBytes))
Write-Host ('  {0} {1,10}  (removed only with -All)' -f 'Retained artifacts'.PadRight($nameWidth), (Format-Bytes $retainedBytes))
if ($keptBytes -gt 0) {
    Write-Host ('  {0} {1,10}  (never removed)' -f 'Kept'.PadRight($nameWidth), (Format-Bytes $keptBytes))
}

$reclaimed = [long] 0
foreach ($target in $requested) {
    if ($target.state -in @('removed', 'partial', 'failed')) { $reclaimed += $target.bytesBefore - $target.bytesAfter }
}
if ($mode -eq 'clean') {
    Write-Host ('  {0} {1,10}' -f 'Reclaimed'.PadRight($nameWidth), (Format-Bytes $reclaimed))
}

$incomplete = @($requested | Where-Object { $_.state -in @('skipped', 'partial', 'failed', 'unresolved') })
$registryIncomplete = ($TestState -and $registryState -eq 'failed')
$succeeded = ($incomplete.Count -eq 0) -and -not $registryIncomplete
if ($mode -eq 'measure') {
    # A measurement is complete when every location could be measured.
    $succeeded = @($requested | Where-Object { $_.state -eq 'unresolved' }).Count -eq 0
}

Write-Host ''
if ($succeeded) {
    Write-Host $(switch ($mode) {
            'measure' { 'Measured; nothing was removed.' }
            'whatif' { 'What-if; nothing was removed.' }
            default { "Cleanup complete: $(Format-Bytes $reclaimed) reclaimed." }
        })
}
else {
    $names = @($incomplete | ForEach-Object { "$($_.name) ($($_.state))" })
    if ($registryIncomplete) { $names += "$testRegistryName (failed)" }
    Write-Host "Cleanup incomplete: $($names -join ', ')."
}

if ($PassThru) {
    [pscustomobject]@{
        root = $root
        mode = $mode
        targets = $targets
        registry = [pscustomobject]@{ path = $testRegistryName; present = $registry.present; subkeys = $registry.subkeys; state = $registryState; message = $registryMessage }
        reclaimed = $reclaimed
        succeeded = $succeeded
    }
}

exit $(if ($succeeded) { 0 } else { 1 })
