<#
    .SYNOPSIS
    Runs TigerSetup installer commands inside a TigerWinLab guest and records
    what they returned.

    .DESCRIPTION
    A TigerWinLab guest job entry script, carried in the job payload beside a
    request.json that names: files to stage outside the job workspace (so a
    later job or a recovery scenario can still find them), commands to run with
    their arguments and timeouts, log files to collect, directories to
    inventory, registry keys to read and whether to read both PATH values.
    Every command's exit code, stdout and stderr are recorded; stdout that
    parses as JSON is also embedded parsed, because TigerSetup's --json output
    is the evidence the caller keys on.

    Every command names where it runs, as "runAs" on the command or, for all of
    them, on the request: "job" is the job account (LabAdmin, session 0, an
    administrator with no desktop), "interactiveUser" the signed-in standard
    user's desktop session, and "elevatedUser" an elevated agent on that same
    desktop. That is how a user-scope row runs Setup.exe as the standard user,
    and how a row that upgrades a running GUI application runs the application
    and the elevated installer where a person would run both: the Restart
    Manager lists a holder from its open file handles across sessions, but it
    closes a window by messaging it, which does not cross one.

    The desktop sessions belong to the lab, which established them because the
    job asked for -Desktop or -ElevatedDesktop and handed them over as
    $TigerWinLabDesktop and $TigerWinLabElevatedDesktop. This payload starts
    none of its own and carries no copy of the lab's desktop client: a payload
    that staged its own copy would be running last week's lab inside this
    week's one.

    Nothing here judges the outcome. The host-side row decides what a pass is.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$inputRoot = $env:TIGERWINLAB_JOB_INPUT
$artifactRoot = $env:TIGERWINLAB_JOB_ARTIFACTS
$request = Get-Content -LiteralPath (Join-Path $inputRoot 'request.json') -Raw | ConvertFrom-Json

function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}

# When commands run on the desktop, a placeholder in a path means the signed-in
# user's folder, not the job account's. This job runs as an administrator in
# session 0, so expanding %LOCALAPPDATA% here would silently name the wrong
# profile — the request would install as one user and then look for the result
# under another. The signed-in user is the one a user-scope row means, so the
# folders follow that account even where a command runs elevated beside it.
$script:UserFolders = @{}

function Set-InteractiveUserFolders {
    param([string] $Sid)
    if ([string]::IsNullOrWhiteSpace($Sid)) { return }
    $key = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$Sid"
    if (-not (Test-Path -LiteralPath $key)) { return }
    $profilePath = [string] (Get-ItemProperty -LiteralPath $key -Name 'ProfileImagePath').ProfileImagePath
    $profilePath = [Environment]::ExpandEnvironmentVariables($profilePath)
    if ([string]::IsNullOrWhiteSpace($profilePath)) { return }
    $script:UserFolders = @{
        'USERPROFILE'   = $profilePath
        'LOCALAPPDATA'  = (Join-Path $profilePath 'AppData\Local')
        'APPDATA'       = (Join-Path $profilePath 'AppData\Roaming')
        'TEMP'          = (Join-Path $profilePath 'AppData\Local\Temp')
        'TMP'           = (Join-Path $profilePath 'AppData\Local\Temp')
        'USERNAME'      = (Split-Path -Leaf $profilePath)
    }
}

function Expand-GuestPath {
    param([string] $Path)
    $text = [string] $Path
    foreach ($name in $script:UserFolders.Keys) {
        $text = [regex]::Replace($text, [regex]::Escape("%$name%"), [string] $script:UserFolders[$name], 'IgnoreCase')
    }
    [Environment]::ExpandEnvironmentVariables($text)
}

function ConvertTo-ArgumentString {
    <#
        Builds a Windows command line the way CommandLineToArgvW will split it
        back into the same arguments. The guest runs Windows PowerShell 5.1,
        whose ProcessStartInfo has no ArgumentList.
    #>
    param([string[]] $Arguments)
    $quoted = foreach ($argument in $Arguments) {
        if ($argument -eq '' -or $argument -match '[\s"]') {
            '"' + (($argument -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
        }
        else {
            $argument
        }
    }
    $quoted -join ' '
}

# The lab supplies its sessions as variables in the scope this script is
# invoked from. They are read with Get-Variable because under Set-StrictMode an
# unset variable throws something far less useful than the message below, and a
# session that is not there is a failure to report rather than something to work
# around by starting one here.
$script:LabSessions = @{
    'interactiveUser' = @{ variable = 'TigerWinLabDesktop'; option = '-Desktop'; session = $null; account = $null }
    'elevatedUser' = @{ variable = 'TigerWinLabElevatedDesktop'; option = '-ElevatedDesktop'; session = $null; account = $null }
}

function Resolve-CommandSession {
    <#
        Answers with the desktop session a command runs in, or $null when it
        runs as the job account itself. The account a session runs as is asked
        of its agent once and remembered: it is the same answer for every
        command that uses it, and the record of every command names it.
    #>
    param([string] $RunAs)

    if ($RunAs -eq 'job') { return $null }
    if (-not $script:LabSessions.ContainsKey($RunAs)) {
        throw "Unknown runAs '$RunAs'; it must be 'job', 'interactiveUser' or 'elevatedUser'."
    }
    $slot = $script:LabSessions[$RunAs]
    if ($null -eq $slot.session) {
        $slot.session = Get-Variable -Name $slot.variable -ValueOnly -ErrorAction SilentlyContinue
        if ($null -eq $slot.session) {
            throw "The job provided no '$RunAs' desktop session; it must be started with $($slot.option)."
        }
        $slot.account = [string] (Invoke-DesktopCommand -Session $slot.session -Command 'info').userName
    }
    $slot.session
}

function Get-CommandAccount {
    <# The account a command's session runs as, for its record. #>
    param([string] $RunAs)

    if ($RunAs -eq 'job') { return [Environment]::UserName }
    [string] $script:LabSessions[$RunAs].account
}

$requestRunAs = [string] (Get-Member2 $request 'runAs')
if ([string]::IsNullOrWhiteSpace($requestRunAs)) { $requestRunAs = 'job' }
$usesDesktop = @(@(Get-Member2 $request 'commands') | ForEach-Object {
        if ($null -eq $_) { return }
        $named = [string] (Get-Member2 $_ 'runAs')
        if ([string]::IsNullOrWhiteSpace($named)) { $requestRunAs } else { $named }
    } | Where-Object { $_ -ne 'job' }).Count -gt 0

$sessionInfo = $null
if ($usesDesktop) {
    # The signed-in account owns the desktop whichever of the two sessions a
    # command uses, and its SID is what addresses that account's registry hive
    # under HKEY_USERS from this (administrator) job.
    $info = Invoke-DesktopCommand -Session (Resolve-CommandSession -RunAs 'interactiveUser') -Command 'info'
    $sid = ([System.Security.Principal.NTAccount] ([string] $info.userName)).Translate([System.Security.Principal.SecurityIdentifier]).Value
    $sessionInfo = [pscustomobject]@{ userName = [string] $info.userName; sid = $sid; isElevated = [bool] $info.isElevated }
    Set-InteractiveUserFolders -Sid $sid
}

# The desktop sessions belong to the lab, which set them up and tears them down
# with the job, so nothing here starts or stops one; only the commands run
# between those two moments are this payload's.
$staged = [System.Collections.Generic.List[object]]::new()
foreach ($item in @(Get-Member2 $request 'stage')) {
    if ($null -eq $item) { continue }
    $source = Join-Path $inputRoot ([string] $item.source)
    $destination = Expand-GuestPath ([string] $item.destination)
    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force
    Copy-Item -LiteralPath $source -Destination $destination -Force
    # A staged file may be read after a reboot or a power cut, so it has to be on
    # the disk, not in the write cache.
    $stream = [System.IO.File]::Open($destination, 'Open', 'ReadWrite', 'None')
    try { $stream.Flush($true) } finally { $stream.Dispose() }
    $staged.Add([pscustomobject]@{ source = [string] $item.source; destination = $destination; length = (Get-Item -LiteralPath $destination).Length })
}
if ($staged.Count -gt 0 -and $usesDesktop) {
    # The interactive account must be able to run what was staged.
    foreach ($item in $staged) {
        $null = & icacls.exe (Split-Path -Parent $item.destination) '/grant' 'Users:(OI)(CI)RX' '/T' '/Q' 2>&1
    }
}

$commands = [System.Collections.Generic.List[object]]::new()
foreach ($command in @(Get-Member2 $request 'commands')) {
    if ($null -eq $command) { continue }
    $executable = Expand-GuestPath ([string] $command.executable)
    $arguments = @(@(Get-Member2 $command 'arguments') | ForEach-Object { Expand-GuestPath ([string] $_) })
    $timeoutSeconds = 600
    if ($null -ne (Get-Member2 $command 'timeoutSeconds')) { $timeoutSeconds = [int] $command.timeoutSeconds }
    $commandRunAs = [string] (Get-Member2 $command 'runAs')
    if ([string]::IsNullOrWhiteSpace($commandRunAs)) { $commandRunAs = $requestRunAs }
    $session = Resolve-CommandSession -RunAs $commandRunAs

    $record = [ordered]@{
        name = [string] $command.name
        executable = $executable
        arguments = $arguments
        session = $commandRunAs
        runAs = Get-CommandAccount -RunAs $commandRunAs
        exitCode = $null
        timedOut = $false
        durationSeconds = $null
        stdout = ''
        stderr = ''
        json = $null
        error = $null
    }
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf) -and -not (Get-Command $executable -ErrorAction SilentlyContinue)) {
        $record.error = "The executable '$executable' does not exist."
        $commands.Add([pscustomobject] $record)
        continue
    }

    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    if ($null -ne $session) {
        # The desktop agent runs the command as the signed-in user and waits
        # for it, returning its exit code and output.
        $run = Invoke-DesktopCommand -Session $session -Command 'run' -Parameters @{
            filePath = $executable
            arguments = @($arguments)
            timeoutSeconds = $timeoutSeconds
        } -TimeoutSeconds ([math]::Min(3600, $timeoutSeconds + 60)) -PassThruError
        $stopwatch.Stop()
        if ($run.ok) {
            $record.exitCode = [int] $run.result.exitCode
            $record.stdout = (@($run.result.stdout) | ForEach-Object { [string] $_ }) -join "`n"
            $record.stderr = (@($run.result.stderr) | ForEach-Object { [string] $_ }) -join "`n"
            $record.timedOut = [bool] (Get-Member2 $run.result 'timedOut')
        }
        else {
            $record.error = [string] $run.error
        }
    }
    else {
        $info = [System.Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $executable
        $info.Arguments = ConvertTo-ArgumentString -Arguments $arguments
        $info.UseShellExecute = $false
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $info.CreateNoWindow = $true

        $process = [System.Diagnostics.Process]::Start($info)
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $exited = $process.WaitForExit($timeoutSeconds * 1000)
        if (-not $exited) {
            try { & taskkill.exe /PID $process.Id /T /F 2>&1 | Out-Null } catch { }
            try { $process.Kill() } catch { }
            $record.timedOut = $true
        }
        $process.WaitForExit()
        $stopwatch.Stop()
        $record.exitCode = if ($exited) { $process.ExitCode } else { $null }
        $record.stdout = $stdoutTask.Result
        $record.stderr = $stderrTask.Result
    }
    $record.durationSeconds = [math]::Round($stopwatch.Elapsed.TotalSeconds, 2)
    if (-not [string]::IsNullOrWhiteSpace($record.stdout)) {
        try { $record.json = $record.stdout | ConvertFrom-Json } catch { $record.json = $null }
    }
    $commands.Add([pscustomobject] $record)
    Write-Host ("[{0}] exit {1} after {2}s" -f $record.name, $record.exitCode, $record.durationSeconds)
}

$logs = [ordered]@{}
foreach ($logPath in @(Get-Member2 $request 'logs')) {
    if ([string]::IsNullOrWhiteSpace([string] $logPath)) { continue }
    $expanded = Expand-GuestPath ([string] $logPath)
    if (Test-Path -LiteralPath $expanded -PathType Leaf) {
        $lines = @(Get-Content -LiteralPath $expanded -ErrorAction SilentlyContinue | ForEach-Object { [string] $_ })
        $logs[$expanded] = @($lines | Select-Object -Last 400)
        Copy-Item -LiteralPath $expanded -Destination (Join-Path $artifactRoot (Split-Path -Leaf $expanded)) -Force
    }
    else {
        $logs[$expanded] = $null
    }
}

$inventory = [System.Collections.Generic.List[object]]::new()
foreach ($path in @(Get-Member2 $request 'inventory')) {
    if ([string]::IsNullOrWhiteSpace([string] $path)) { continue }
    $expanded = Expand-GuestPath ([string] $path)
    if (Test-Path -LiteralPath $expanded) {
        $files = @(Get-ChildItem -LiteralPath $expanded -Recurse -File -Force -ErrorAction SilentlyContinue)
        $inventory.Add([pscustomobject]@{ requested = [string] $path; path = $expanded; exists = $true; fileCount = $files.Count; totalBytes = $(if ($files.Count -gt 0) { [long] ($files | Measure-Object -Property Length -Sum).Sum } else { [long] 0 }); files = @($files | Select-Object -First 200 | ForEach-Object { $_.FullName.Substring($expanded.TrimEnd('\').Length + 1) }) })
    }
    else {
        $inventory.Add([pscustomobject]@{ requested = [string] $path; path = $expanded; exists = $false; fileCount = 0; totalBytes = 0; files = @() })
    }
}

# Registry keys to read, as "HKLM\..." or "HKCU\..." paths. HKCU means the
# job account's hive unless the commands ran as the interactive user, in
# which case that user's hive is read through HKEY_USERS.
$registry = [System.Collections.Generic.List[object]]::new()
foreach ($keyPath in @(Get-Member2 $request 'registry')) {
    if ([string]::IsNullOrWhiteSpace([string] $keyPath)) { continue }
    $text = [string] $keyPath
    $provider = $text
    if ($text -match '^HKCU\\(.*)$') {
        $provider = if ($null -ne $sessionInfo -and -not [string]::IsNullOrWhiteSpace([string] (Get-Member2 $sessionInfo 'sid'))) { "Registry::HKEY_USERS\$($sessionInfo.sid)\$($Matches[1])" } else { "HKCU:\$($Matches[1])" }
    }
    elseif ($text -match '^HKLM\\(.*)$') {
        $provider = "HKLM:\$($Matches[1])"
    }
    if (Test-Path -LiteralPath $provider) {
        $properties = Get-ItemProperty -LiteralPath $provider
        $values = [ordered]@{}
        foreach ($name in @($properties.PSObject.Properties.Name | Where-Object { $_ -notlike 'PS*' })) {
            $values[$name] = [string] $properties.$name
        }
        $registry.Add([pscustomobject]@{ requested = $text; path = $provider; exists = $true; values = [pscustomobject] $values })
    }
    else {
        $registry.Add([pscustomobject]@{ requested = $text; path = $provider; exists = $false; values = $null })
    }
}

$pathValues = $null
if ([bool] (Get-Member2 $request 'pathValues')) {
    $machineKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Control\Session Manager\Environment')
    $machinePath = $machineKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $machineKind = [string] $machineKey.GetValueKind('Path')
    $userHive = [Microsoft.Win32.Registry]::CurrentUser
    if ($null -ne $sessionInfo -and -not [string]::IsNullOrWhiteSpace([string] (Get-Member2 $sessionInfo 'sid'))) {
        $userHive = [Microsoft.Win32.Registry]::Users.OpenSubKey([string] $sessionInfo.sid)
    }
    $userKey = $userHive.OpenSubKey('Environment')
    $userPath = ''
    $userKind = ''
    if ($null -ne $userKey) {
        $names = @($userKey.GetValueNames())
        if ($names -contains 'Path') {
            $userPath = $userKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $userKind = [string] $userKey.GetValueKind('Path')
        }
    }
    $pathValues = [pscustomobject]@{
        machine = [pscustomobject]@{ raw = [string] $machinePath; kind = $machineKind; entries = @(([string] $machinePath) -split ';') }
        user = [pscustomobject]@{ raw = [string] $userPath; kind = $userKind; entries = @(([string] $userPath) -split ';') }
    }
}

$result = [pscustomobject][ordered]@{
    collectedAt = [DateTimeOffset]::Now.ToString('o')
    runAs = $(if ($null -ne $sessionInfo) { [pscustomobject]@{ userName = $sessionInfo.userName; sid = (Get-Member2 $sessionInfo 'sid') } } else { [pscustomobject]@{ userName = [Environment]::UserName; sid = $null } })
    staged = @($staged)
    commands = @($commands)
    logs = [pscustomobject] $logs
    inventory = @($inventory)
    registry = @($registry)
    pathValues = $pathValues
}
$json = $result | ConvertTo-Json -Depth 12
[System.IO.File]::WriteAllText($env:TIGERWINLAB_JOB_RESULT, $json, [System.Text.UTF8Encoding]::new($false))
[System.IO.File]::WriteAllText((Join-Path $artifactRoot 'setup-commands.json'), $json, [System.Text.UTF8Encoding]::new($false))
exit 0
