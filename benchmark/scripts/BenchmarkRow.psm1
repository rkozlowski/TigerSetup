#Requires -Version 7.0
<#
    .SYNOPSIS
    What every benchmark lab driver does with a row's evidence: reads the
    guest reader's result (lab\guest\Invoke-SetupCommands.ps1) strict-mode
    safely, compares an installed tree with a canonical inventory file by
    file, places TigerSetup's engine log inside the measured process
    lifetime, and summarizes a timed command.

    .DESCRIPTION
    One implementation for the two drivers — Invoke-BenchmarkLab.ps1 (the
    functional-contract campaign of the first benchmark) and
    Invoke-BroadBenchmarkLab.ps1 (the broad-corpus campaign) — so a row's
    payload verdict and its timing summary mean the same thing in both. The
    functions judge nothing: they read evidence and return facts; each
    driver decides what a pass is.
#>

Set-StrictMode -Version Latest

function Get-Prop {
    <# Strict-mode-safe property read: $null in, $null out, instead of throwing. #>
    param([object] $Object, [string] $Name)
    if ($null -eq $Object) { return $null }
    if ($Object.PSObject.Properties.Match($Name).Count -eq 0) { return $null }
    $Object.$Name
}

function Split-CommandLine {
    <#
        Splits a Windows-style command line into argv, respecting quotes --
        including a quoted run glued to an unquoted prefix like
        /DIR="C:\some\path" (Inno Setup's own switch style), where only the
        quoted segment is unquoted, not the whole token.
    #>
    param([string] $Line)
    $tokens = [System.Text.RegularExpressions.Regex]::Matches($Line, '(?:"[^"]*"|[^\s"])+')
    @($tokens | ForEach-Object {
        [System.Text.RegularExpressions.Regex]::Replace($_.Value, '"([^"]*)"', '$1')
    })
}

function Test-JobOk {
    <# A job is only trustworthy when the entry point itself reported OK and wrote a result. #>
    param([object] $JobRun)
    ($null -ne $JobRun) -and ([string] $JobRun.status -eq 'OK') -and ($null -ne $JobRun.result)
}

function Get-JobFailureReason {
    param([object] $JobRun)
    if ($null -eq $JobRun) { return 'Invoke-TigerSetupGuestCommands returned nothing.' }
    "status=$($JobRun.status) exitCode=$($JobRun.exitCode) outerTimedOut=$($JobRun.outerTimedOut) (see $($JobRun.stderrPath))"
}

function Get-Evidence {
    <# The guest reader's own result document, or $null where the job did not complete. #>
    param([object] $JobRun)
    if (-not (Test-JobOk $JobRun)) { return $null }
    $JobRun.result.result
}

function Get-Record {
    <# The first record of a collection whose field holds the value (the guest's `requested` path, a command's `name`). #>
    param([object] $Evidence, [string] $Collection, [string] $Field, [string] $Value)
    if ($null -eq $Evidence) { return $null }
    @(Get-Prop $Evidence $Collection) | Where-Object { $null -ne $_ -and $_.$Field -eq $Value } | Select-Object -First 1
}

function Get-RegistryValue {
    <# The text the guest holds for a value ('' for the default value), or $null when the key or the value is absent. #>
    param([object] $Evidence, [string] $Key, [string] $Name)
    $record = Get-Record $Evidence 'registry' 'requested' $Key
    if ($null -eq $record -or -not [bool] (Get-Prop $record 'exists')) { return $null }
    $values = Get-Prop $record 'values'
    $label = $(if ($Name -eq '') { '(default)' } else { $Name })
    if ($null -eq $values -or $values.PSObject.Properties.Match($label).Count -eq 0) { return $null }
    [string] $values.$label
}

function Test-KeyExists {
    param([object] $Evidence, [string] $Key)
    $record = Get-Record $Evidence 'registry' 'requested' $Key
    ($null -ne $record) -and [bool] (Get-Prop $record 'exists')
}

function Get-ValueNames {
    param([object] $Evidence, [string] $Key)
    $record = Get-Record $Evidence 'registry' 'requested' $Key
    if ($null -eq $record -or -not [bool] (Get-Prop $record 'exists')) { return @() }
    $values = Get-Prop $record 'values'
    if ($null -eq $values) { return @() }
    @($values.PSObject.Properties | ForEach-Object { $_.Name })
}

function Compare-InstalledPayload {
    <#
        The installed files against the canonical inventory: every canonical
        file present with its hash, and what else the technology left under
        the root (its own bookkeeping, with its bytes). Without hashes in the
        evidence (an older guest reader) the comparison is reported as not
        verified. -Exclude names canonical entries no package installs (a
        glob against the inventory path); they are neither expected nor
        counted as extras.
    #>
    param([object] $Inventory, [object] $Canonical, [string[]] $Exclude = @())
    $hashes = Get-Prop $Inventory 'hashes'
    $result = [ordered]@{ verified = $false; exact = $null; expected = 0; matched = 0; missing = @(); differing = @(); extras = @(); extraBytes = [long] 0 }
    if ($null -eq $hashes) { return $result }
    $installed = @{}
    foreach ($entry in @($hashes)) { $installed[([string] $entry.path).ToLowerInvariant()] = $entry }
    $missing = [System.Collections.Generic.List[string]]::new()
    $differing = [System.Collections.Generic.List[string]]::new()
    $seen = [System.Collections.Generic.HashSet[string]]::new()
    $matched = 0
    $expected = 0
    foreach ($entry in @($Canonical.inventory)) {
        $path = [string] $entry.path
        if (@($Exclude | Where-Object { $path -like $_ }).Count -gt 0) { continue }
        $expected++
        $key = $path.ToLowerInvariant()
        $null = $seen.Add($key)
        if (-not $installed.ContainsKey($key)) { $missing.Add($path); continue }
        $found = $installed[$key]
        if ([long] $found.bytes -ne [long] $entry.bytes -or [string] $found.sha256 -ne ([string] $entry.sha256).ToLowerInvariant()) { $differing.Add($path) } else { $matched++ }
    }
    $extraKeys = @($installed.Keys | Where-Object { -not $seen.Contains($_) } | Sort-Object)
    $extraBytes = [long] 0
    foreach ($key in $extraKeys) { $extraBytes += [long] $installed[$key].bytes }
    $result.verified = $true
    $result.expected = $expected
    $result.matched = $matched
    $result.missing = @($missing)
    $result.differing = @($differing)
    $result.extras = @($extraKeys | ForEach-Object { [string] $installed[$_].path })
    $result.extraBytes = $extraBytes
    $result.exact = ($missing.Count -eq 0 -and $differing.Count -eq 0)
    $result
}

function Get-EngineSpan {
    <#
        TigerSetup's engine log placed inside the process lifetime the guest
        measured: the span from its first event (run_started) to its last
        (run_finished), what ran before the first event (the loader:
        locating the footer, decompressing and verifying the engine,
        starting it) and after the last (the engine's exit and the loader's
        clean-up). $null where there is no such log or the timestamps do not
        parse; the log is the copy the job brought back.
    #>
    param([object] $JobRun, [string] $LogLeaf, [object] $Command)
    if (-not (Test-JobOk $JobRun)) { return $null }
    $outputPath = [string] (Get-Prop $JobRun.result 'outputPath')
    if ([string]::IsNullOrWhiteSpace($outputPath)) { return $null }
    $logPath = Join-Path $outputPath $LogLeaf
    if (-not (Test-Path -LiteralPath $logPath -PathType Leaf)) { return $null }
    $lines = @(Get-Content -LiteralPath $logPath | Where-Object { $_ -match '^\d{4}-\d{2}-\d{2}T\S+Z \[' })
    if ($lines.Count -eq 0) { return $null }
    $stamp = { param($line) [DateTimeOffset]::Parse(($line -split ' ', 2)[0], [cultureinfo]::InvariantCulture) }
    $event = { param($line) if ($line -match '\[([^\]]+)\]') { $Matches[1] } else { '' } }
    $first = & $stamp $lines[0]
    $last = & $stamp $lines[-1]
    # ConvertFrom-Json has already turned the guest's ISO timestamps into
    # [DateTime] values (Kind Utc); a string is parsed with its own offset.
    $toOffset = { param($value) if ($null -eq $value) { $null } elseif ($value -is [DateTime]) { [DateTimeOffset]::new($value.ToUniversalTime()) } elseif (-not [string]::IsNullOrWhiteSpace([string] $value)) { [DateTimeOffset]::Parse([string] $value, [cultureinfo]::InvariantCulture) } else { $null } }
    $started = & $toOffset (Get-Prop $Command 'startedAtUtc')
    $finished = & $toOffset (Get-Prop $Command 'finishedAtUtc')
    [ordered]@{
        log = $logPath
        lines = $lines.Count
        firstEvent = (& $event $lines[0])
        lastEvent = (& $event $lines[-1])
        spanSeconds = [math]::Round(($last - $first).TotalSeconds, 3)
        beforeSeconds = $(if ($null -ne $started) { [math]::Round(($first - $started).TotalSeconds, 3) } else { $null })
        afterSeconds = $(if ($null -ne $finished) { [math]::Round(($finished - $last).TotalSeconds, 3) } else { $null })
    }
}

function Get-CommandSummary {
    <# A timed command's outcome: exit code, its process lifetime, the completion wait and their sum. #>
    param([object] $Evidence, [string] $Name)
    $command = Get-Record $Evidence 'commands' 'name' $Name
    $completion = Get-Prop $command 'completion'
    [ordered]@{
        exitCode        = (Get-Prop $command 'exitCode')
        durationSeconds = (Get-Prop $command 'durationSeconds')
        processSeconds  = (Get-Prop $command 'processSeconds')
        completionWaitSeconds = $(if ($null -ne $completion) { Get-Prop $completion 'waitedSeconds' } else { 0 })
        completionSatisfied = $(if ($null -ne $completion) { [bool] (Get-Prop $completion 'satisfied') } else { $true })
        timedOut        = (Get-Prop $command 'timedOut')
        error           = (Get-Prop $command 'error')
    }
}

Export-ModuleMember -Function Get-Prop, Split-CommandLine, Test-JobOk, Get-JobFailureReason, Get-Evidence, Get-Record, Get-RegistryValue, Test-KeyExists, Get-ValueNames, Compare-InstalledPayload, Get-EngineSpan, Get-CommandSummary
