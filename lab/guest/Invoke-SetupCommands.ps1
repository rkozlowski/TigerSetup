<#
    .SYNOPSIS
    Runs TigerSetup installer commands inside a TigerWinLab guest and records
    what they returned.

    .DESCRIPTION
    A TigerWinLab guest job entry script, carried in the job payload beside a
    request.json that names: files to stage outside the job workspace (so a
    later job or a recovery scenario can still find them), commands to run with
    their arguments and timeouts — and, where a program hands its work to
    another process before it exits, the completion the caller means: named
    processes that must have exited and paths that must be gone before the
    command counts as finished and its clock stops — log files to collect,
    directories to inventory, registry keys to read, whether to read both
    PATH values, and —
    for the resources 0.6 added — firewall rules to read by name, shortcuts to
    read through the shell (target, arguments, working directory, icon and
    AppUserModelID; the URL of an Internet shortcut) and environment
    variables to read as the registry holds them and as Windows resolves them.
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

    # What the command hands its work to: process names (without .exe) that
    # must have exited, and paths that must be gone, before the command is
    # complete. An NSIS uninstaller copies itself to %TEMP% and exits while
    # the copy does the work; a caller that times the original alone times
    # nothing. Only processes started after the command are waited for.
    $waitForProcesses = @(@(Get-Member2 $command 'waitForProcesses') | Where-Object { -not [string]::IsNullOrWhiteSpace([string] $_) } | ForEach-Object { ([string] $_) -replace '\.exe$', '' })
    $waitForAbsentPaths = @(@(Get-Member2 $command 'waitForAbsentPaths') | Where-Object { -not [string]::IsNullOrWhiteSpace([string] $_) } | ForEach-Object { Expand-GuestPath ([string] $_) })
    $record = [ordered]@{
        name = [string] $command.name
        executable = $executable
        arguments = $arguments
        session = $commandRunAs
        runAs = Get-CommandAccount -RunAs $commandRunAs
        exitCode = $null
        timedOut = $false
        durationSeconds = $null
        # The process's own lifetime, and what the completion wait added.
        processSeconds = $null
        completion = $null
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

    $commandStartedAt = [DateTime]::Now
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
    $record.processSeconds = [math]::Round($stopwatch.Elapsed.TotalSeconds, 2)
    if ($waitForProcesses.Count -gt 0 -or $waitForAbsentPaths.Count -gt 0) {
        # The command's clock keeps running until what it handed off has
        # finished, within the command's own timeout.
        $stopwatch.Start()
        $deadline = $commandStartedAt.AddSeconds($timeoutSeconds)
        $satisfied = $false
        $remainingProcesses = @()
        $remainingPaths = @()
        while ($true) {
            $remainingProcesses = @(foreach ($name in $waitForProcesses) {
                    Get-Process -Name $name -ErrorAction SilentlyContinue | Where-Object {
                        $started = $null
                        try { $started = $_.StartTime } catch { $started = $null }
                        $null -eq $started -or $started -ge $commandStartedAt
                    } | ForEach-Object { "$($_.ProcessName) ($($_.Id))" }
                })
            $remainingPaths = @($waitForAbsentPaths | Where-Object { Test-Path -LiteralPath $_ })
            if ($remainingProcesses.Count -eq 0 -and $remainingPaths.Count -eq 0) { $satisfied = $true; break }
            if ([DateTime]::Now -ge $deadline) { break }
            Start-Sleep -Milliseconds 200
        }
        $stopwatch.Stop()
        if (-not $satisfied) { $record.timedOut = $true }
        $record.completion = [pscustomobject][ordered]@{
            processes = @($waitForProcesses)
            absentPaths = @($waitForAbsentPaths)
            satisfied = $satisfied
            waitedSeconds = [math]::Round($stopwatch.Elapsed.TotalSeconds - $record.processSeconds, 2)
            remainingProcesses = @($remainingProcesses)
            remainingPaths = @($remainingPaths)
        }
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
        # A key with no values at all yields an object with no properties, and
        # enumerating `.Name` over nothing is an error under strict mode; the
        # key's own values are read one by one instead.
        $values = [ordered]@{}
        $key = Get-Item -LiteralPath $provider -ErrorAction SilentlyContinue
        if ($null -ne $key) {
            foreach ($name in @($key.GetValueNames())) {
                $label = $(if ($name -eq '') { '(default)' } else { [string] $name })
                $values[$label] = [string] $key.GetValue($name, '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            }
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

# Firewall rules by display name, as the firewall service answers: every
# rule of that name, with its program, direction, action, protocol and ports.
$firewallRules = [System.Collections.Generic.List[object]]::new()
foreach ($ruleName in @(Get-Member2 $request 'firewallRules')) {
    if ([string]::IsNullOrWhiteSpace([string] $ruleName)) { continue }
    $found = @(Get-NetFirewallRule -DisplayName ([string] $ruleName) -ErrorAction SilentlyContinue)
    $rules = @(foreach ($rule in $found) {
            $application = $rule | Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue
            $port = $rule | Get-NetFirewallPortFilter -ErrorAction SilentlyContinue
            [pscustomobject][ordered]@{
                name = [string] $rule.DisplayName
                group = [string] $rule.Group
                description = [string] $rule.Description
                enabled = ([string] $rule.Enabled -eq 'True')
                direction = ([string] $rule.Direction).ToLowerInvariant()
                action = ([string] $rule.Action).ToLowerInvariant()
                program = [string] $(if ($null -ne $application) { $application.Program } else { '' })
                protocol = [string] $(if ($null -ne $port) { $port.Protocol } else { '' })
                localPort = [string] $(if ($null -ne $port) { @($port.LocalPort) -join ',' } else { '' })
                profile = [string] $rule.Profile
            }
        })
    $firewallRules.Add([pscustomobject]@{ requested = [string] $ruleName; count = $rules.Count; rules = @($rules) })
}

function Get-LinkStoredTarget {
    <#
        The target path a .lnk file stores (MS-SHLLINK LinkInfo.LocalBasePath
        plus CommonPathSuffix), read from the bytes. The shell relocates a
        link's target through the known folder recorded in it, so a per-user
        link read from another account's session answers with that account's
        folder; the stored path says what the installer wrote. $null when the
        link stores no local path.
    #>
    param([string] $Path)
    try {
        $bytes = [System.IO.File]::ReadAllBytes($Path)
        if ($bytes.Length -lt 0x4C -or [BitConverter]::ToInt32($bytes, 0) -ne 0x4C) { return $null }
        $flags = [BitConverter]::ToUInt32($bytes, 0x14)
        $offset = 0x4C
        if ($flags -band 0x1) { $offset += 2 + [BitConverter]::ToUInt16($bytes, $offset) }
        if (-not ($flags -band 0x2)) { return $null }
        $info = $offset
        $headerSize = [BitConverter]::ToInt32($bytes, $info + 4)
        $infoFlags = [BitConverter]::ToUInt32($bytes, $info + 8)
        if (-not ($infoFlags -band 0x1)) { return $null }
        $readAnsi = {
            param([int] $at)
            $end = $at; while ($end -lt $bytes.Length -and $bytes[$end] -ne 0) { $end++ }
            [System.Text.Encoding]::Default.GetString($bytes, $at, $end - $at)
        }
        $readUnicode = {
            param([int] $at)
            $end = $at; while ($end + 1 -lt $bytes.Length -and -not ($bytes[$end] -eq 0 -and $bytes[$end + 1] -eq 0)) { $end += 2 }
            [System.Text.Encoding]::Unicode.GetString($bytes, $at, $end - $at)
        }
        if ($headerSize -ge 0x24) {
            $base = & $readUnicode ($info + [BitConverter]::ToInt32($bytes, $info + 0x1C))
            $suffix = & $readUnicode ($info + [BitConverter]::ToInt32($bytes, $info + 0x20))
        }
        else {
            $base = & $readAnsi ($info + [BitConverter]::ToInt32($bytes, $info + 0x10))
            $suffix = & $readAnsi ($info + [BitConverter]::ToInt32($bytes, $info + 0x18))
        }
        if ([string]::IsNullOrEmpty($suffix)) { return $base }
        return (Join-Path $base $suffix)
    }
    catch { return $null }
}

# Shortcuts as the shell reads them: what a double-click would run. The
# target is the stored one where the file has it (Get-LinkStoredTarget), and
# the shell's resolution beside it.
$shortcuts = [System.Collections.Generic.List[object]]::new()
$wscript = $null
$shellApplication = $null
foreach ($shortcutPath in @(Get-Member2 $request 'shortcuts')) {
    if ([string]::IsNullOrWhiteSpace([string] $shortcutPath)) { continue }
    $expanded = Expand-GuestPath ([string] $shortcutPath)
    $record = [ordered]@{ requested = [string] $shortcutPath; path = $expanded; exists = (Test-Path -LiteralPath $expanded -PathType Leaf); kind = $null; target = $null; resolvedTarget = $null; arguments = $null; workingDirectory = $null; icon = $null; appUserModelId = $null; url = $null }
    if ($record.exists) {
        if ($expanded -like '*.url') {
            $record.kind = 'url'
            $urlLine = @(Get-Content -LiteralPath $expanded | Where-Object { $_ -like 'URL=*' }) | Select-Object -First 1
            $record.url = [string] $(if ($null -ne $urlLine) { $urlLine.Substring(4) } else { '' })
        }
        else {
            $record.kind = 'link'
            if ($null -eq $wscript) { $wscript = New-Object -ComObject WScript.Shell }
            $link = $wscript.CreateShortcut($expanded)
            $record.resolvedTarget = [string] $link.TargetPath
            $stored = Get-LinkStoredTarget -Path $expanded
            $record.target = [string] $(if ([string]::IsNullOrEmpty($stored)) { $link.TargetPath } else { $stored })
            $record.arguments = [string] $link.Arguments
            $record.workingDirectory = [string] $link.WorkingDirectory
            $record.icon = [string] $link.IconLocation
            if ($null -eq $shellApplication) { $shellApplication = New-Object -ComObject Shell.Application }
            $folder = $shellApplication.Namespace((Split-Path -Parent $expanded))
            $item = $(if ($null -ne $folder) { $folder.ParseName((Split-Path -Leaf $expanded)) } else { $null })
            $record.appUserModelId = [string] $(if ($null -ne $item) { $item.ExtendedProperty('System.AppUserModel.ID') } else { '' })
        }
    }
    $shortcuts.Add([pscustomobject] $record)
}

# Environment variables: the registry value as written (unexpanded) in the
# user's and the machine's key, and what Windows resolves for the account.
$environmentVariables = [System.Collections.Generic.List[object]]::new()
foreach ($variableName in @(Get-Member2 $request 'environmentVariables')) {
    if ([string]::IsNullOrWhiteSpace([string] $variableName)) { continue }
    $name = [string] $variableName
    $userHive = [Microsoft.Win32.Registry]::CurrentUser
    if ($null -ne $sessionInfo -and -not [string]::IsNullOrWhiteSpace([string] (Get-Member2 $sessionInfo 'sid'))) {
        $userHive = [Microsoft.Win32.Registry]::Users.OpenSubKey([string] $sessionInfo.sid)
    }
    $userKey = $userHive.OpenSubKey('Environment')
    $userRaw = $null
    $userKind = $null
    if ($null -ne $userKey -and @($userKey.GetValueNames()) -contains $name) {
        $userRaw = [string] $userKey.GetValue($name, '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $userKind = [string] $userKey.GetValueKind($name)
    }
    $machineKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Control\Session Manager\Environment')
    $machineRaw = $null
    $machineKind = $null
    if ($null -ne $machineKey -and @($machineKey.GetValueNames()) -contains $name) {
        $machineRaw = [string] $machineKey.GetValue($name, '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $machineKind = [string] $machineKey.GetValueKind($name)
    }
    $environmentVariables.Add([pscustomobject][ordered]@{
            name = $name
            user = [pscustomobject]@{ raw = $userRaw; kind = $userKind }
            machine = [pscustomobject]@{ raw = $machineRaw; kind = $machineKind }
            # What a new process of this job account would see; a desktop
            # session's own view is the same registry read through its SID.
            resolvedMachine = [Environment]::GetEnvironmentVariable($name, 'Machine')
        })
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
    firewallRules = @($firewallRules)
    shortcuts = @($shortcuts)
    environmentVariables = @($environmentVariables)
}
$json = $result | ConvertTo-Json -Depth 12
[System.IO.File]::WriteAllText($env:TIGERWINLAB_JOB_RESULT, $json, [System.Text.UTF8Encoding]::new($false))
[System.IO.File]::WriteAllText((Join-Path $artifactRoot 'setup-commands.json'), $json, [System.Text.UTF8Encoding]::new($false))
exit 0
