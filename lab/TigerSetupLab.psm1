<#
    .SYNOPSIS
    TigerSetup's consumer side of the TigerWinLab contract.

    .DESCRIPTION
    TigerWinLab is a separate project and a generic provider. This module holds
    everything TigerSetup needs to drive it: locating the lab through the
    TigerAiCore configuration, invoking a lab entry point as a child process with
    a result file and an outer timeout, generating the specifications the lab
    scenarios read, running Setup.exe commands inside the guest through a plain
    job, and flattening lab checks into TigerSetup's own row results.

    Nothing here is copied into TigerWinLab, and nothing consumer-specific is
    read from it. Exit codes from every entry point: 0 OK, 1 failed, 2 BUSY,
    3 TIMEOUT; a missing or unreadable result file is a failure, never a pass.
#>

Set-StrictMode -Version Latest

$script:GuestScriptRoot = Join-Path $PSScriptRoot 'guest'
# The lab session every entry point this module invokes belongs to, or $null
# when the caller opened none. It is module state rather than a parameter
# because a session is a property of the run, not of one invocation, and every
# lab call goes through one function.
$script:LabSessionId = $null

function Get-TigerAiCoreRoot {
    <#
        .SYNOPSIS
        The TigerAiCore repository named by the machine configuration.

        .DESCRIPTION
        The configuration file named by TigerAiCoreConfig declares `core` in its
        managed block. That value is the only supported way to reach TigerAiCore's
        own tools, including the resource resolver. Nothing is discovered.
    #>
    [CmdletBinding()]
    param()

    $configPath = $env:TigerAiCoreConfig
    if ([string]::IsNullOrWhiteSpace($configPath) -or -not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
        throw 'TigerAiCoreConfig is not set, or does not name a readable file. Lab-backed verification is unavailable; pass -TigerWinLabRoot to point one run at a checkout.'
    }
    foreach ($line in Get-Content -LiteralPath $configPath) {
        if ($line.Trim() -match '^core\s*=\s*"(?<path>[^"]+)"\s*(?:#.*)?$') {
            # A TOML basic string escapes its backslashes; the path does not.
            return $Matches['path'].Replace('\\', '\')
        }
    }
    throw "The TigerAiCore configuration $configPath declares no 'core' path, so its resource resolver cannot be reached."
}

function Get-TigerSetupLabRoot {
    <#
        .SYNOPSIS
        Resolves TigerWinLab through TigerAiCore's resource resolver.

        .DESCRIPTION
        Repository layout is not topology. A registered Lab is resolved by
        `tools/Resolve-TigerAiCoreResource.ps1 -Lab TigerWinLab`, which reads the
        machine configuration named by TigerAiCoreConfig. TigerSetup keeps no
        discovery mechanism of its own: no sibling-directory guess, no filesystem
        scan, and no TigerSetup-specific environment variable.

        An explicit -TigerWinLabRoot overrides the resolved location for one run,
        which is an override rather than a second discovery system. It is accepted
        only when it actually holds the entry point that will be invoked.

        Resolver exit codes: 0 resolved, 1 unavailable on this machine,
        2 a broken configuration or an invalid registration.
    #>
    [CmdletBinding()]
    param(
        [string] $TigerWinLabRoot
    )

    if (-not [string]::IsNullOrWhiteSpace($TigerWinLabRoot)) {
        if (-not (Test-Path -LiteralPath (Join-Path $TigerWinLabRoot 'Invoke-TigerWinLabJob.ps1') -PathType Leaf)) {
            throw "-TigerWinLabRoot '$TigerWinLabRoot' does not hold Invoke-TigerWinLabJob.ps1."
        }
        return (Resolve-Path -LiteralPath $TigerWinLabRoot).Path
    }

    $resolver = Join-Path (Get-TigerAiCoreRoot) 'tools/Resolve-TigerAiCoreResource.ps1'
    if (-not (Test-Path -LiteralPath $resolver -PathType Leaf)) {
        throw "The TigerAiCore resource resolver was not found at $resolver."
    }

    $stdout = & (Get-Process -Id $PID).Path -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File $resolver -Lab TigerWinLab -Json 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($stdout | Out-String)
    switch ($exitCode) {
        0 { }
        1 { throw "TigerWinLab is not registered on this machine, so Lab-backed verification is unavailable: $($text.Trim())" }
        default { throw "Resolving TigerWinLab failed (exit $exitCode): $($text.Trim())" }
    }

    $registration = $text | ConvertFrom-Json
    if (-not $registration.PathExists) {
        throw "The registered TigerWinLab path '$($registration.Path)' does not exist (from $($registration.ConfigPath))."
    }
    if (-not (Test-Path -LiteralPath (Join-Path $registration.Path 'Invoke-TigerWinLabJob.ps1') -PathType Leaf)) {
        throw "The registered TigerWinLab path '$($registration.Path)' does not hold Invoke-TigerWinLabJob.ps1."
    }
    (Resolve-Path -LiteralPath $registration.Path).Path
}

function ConvertTo-CommandLineArgument {
    param([Parameter(Mandatory)] [string] $Value)

    if ($Value -match '[\s"]') {
        return '"' + ($Value -replace '"', '\"') + '"'
    }
    $Value
}

function Invoke-TigerWinLabEntryPoint {
    <#
        .SYNOPSIS
        Runs one TigerWinLab entry point as a child process and returns its result.

        .DESCRIPTION
        The lab's exit is a result, not the end of the caller. The result file
        path is chosen here and handed to the lab; stdout is kept as a log but
        never parsed. The child is bounded by an outer timeout generously beyond
        the lab's own, because the lab still restores and tears down after its
        timeout fires.

        While a lab session is open (see Enter-TigerSetupLabSession) every
        invocation carries its id: each invocation is one lease on the baseline
        VM inside that session, and the session is what lets one job preserve
        the VM for the next and what hands the VM back when the run ends.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [Parameter(Mandatory)] [string] $EntryPoint,
        [hashtable] $Parameters = @{},
        [Parameter(Mandatory)] [string] $ResultPath,
        [Parameter(Mandatory)] [string] $OutputRoot,
        [int] $TimeoutMinutes = 30,
        [int] $OuterTimeoutMinutes = 0
    )

    $script = Join-Path $LabRoot $EntryPoint
    if (-not (Test-Path -LiteralPath $script -PathType Leaf)) {
        throw "TigerWinLab entry point '$script' does not exist."
    }
    if ($OuterTimeoutMinutes -le 0) { $OuterTimeoutMinutes = $TimeoutMinutes + 20 }

    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $ResultPath) -Force
    $null = New-Item -ItemType Directory -Path $OutputRoot -Force
    Remove-Item -LiteralPath $ResultPath -Force -ErrorAction SilentlyContinue

    $arguments = [System.Collections.Generic.List[string]]::new()
    foreach ($item in @('-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File')) { $arguments.Add($item) }
    $arguments.Add((ConvertTo-CommandLineArgument $script))
    foreach ($name in $Parameters.Keys) {
        $value = $Parameters[$name]
        if ($null -eq $value -or ($value -is [string] -and [string]::IsNullOrWhiteSpace($value))) { continue }
        if ($value -is [bool] -or $value -is [switch]) {
            if ([bool] $value) { $arguments.Add("-$name") }
            continue
        }
        $arguments.Add("-$name")
        $arguments.Add((ConvertTo-CommandLineArgument ([string] $value)))
    }
    foreach ($pair in @(@('-ResultPath', $ResultPath), @('-OutputRoot', $OutputRoot), @('-TimeoutMinutes', [string] $TimeoutMinutes))) {
        $arguments.Add($pair[0]); $arguments.Add((ConvertTo-CommandLineArgument $pair[1]))
    }
    # Every operation of one run belongs to that run's session: a job's lease can
    # preserve the VM for the next job only inside a session, and ending the
    # session is what hands the VM back to the lab.
    if (-not [string]::IsNullOrWhiteSpace([string] $script:LabSessionId)) {
        $arguments.Add('-SessionId'); $arguments.Add((ConvertTo-CommandLineArgument ([string] $script:LabSessionId)))
    }

    $stdoutPath = [System.IO.Path]::ChangeExtension($ResultPath, '.stdout.log')
    $stderrPath = [System.IO.Path]::ChangeExtension($ResultPath, '.stderr.log')
    $pwsh = (Get-Process -Id $PID).Path
    $startedAt = [DateTimeOffset]::Now
    $process = Start-Process -FilePath $pwsh -ArgumentList $arguments.ToArray() -PassThru -NoNewWindow `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $exited = $process.WaitForExit($OuterTimeoutMinutes * 60 * 1000)
    if (-not $exited) {
        try { & taskkill.exe /PID $process.Id /T /F 2>&1 | Out-Null } catch { }
        try { $process.Kill($true) } catch { }
    }
    $completedAt = [DateTimeOffset]::Now

    $exitCode = if ($exited) { $process.ExitCode } else { $null }
    $status = switch ($exitCode) {
        0 { 'OK' }
        2 { 'BUSY' }
        3 { 'TIMEOUT' }
        $null { 'OUTER_TIMEOUT' }
        default { 'FAILED' }
    }
    $result = $null
    if (Test-Path -LiteralPath $ResultPath -PathType Leaf) {
        try {
            $result = Get-Content -LiteralPath $ResultPath -Raw | ConvertFrom-Json
        }
        catch {
            $status = 'UNREADABLE_RESULT'
        }
    }
    elseif ($status -eq 'OK') {
        $status = 'NO_RESULT'
    }

    [pscustomobject][ordered]@{
        entryPoint = $EntryPoint
        status = $status
        exitCode = $exitCode
        outerTimedOut = -not $exited
        durationSeconds = [math]::Round(($completedAt - $startedAt).TotalSeconds, 1)
        result = $result
        resultPath = $ResultPath
        stdoutPath = $stdoutPath
        stderrPath = $stderrPath
    }
}

function Enter-TigerSetupLabSession {
    <#
        .SYNOPSIS
        Opens the lab session the rest of this run's lab operations belong to.

        .DESCRIPTION
        A lab VM is shared. A session is one unit of consumer work: every job a
        run invokes takes its own lease on the baseline VM inside the session,
        a job can preserve the VM's state for the next job only within a
        session, and ending the session hands whatever the run still holds
        back to the lab. Every entry point invoked while the session is open
        carries its id, and the id is the consumer's own: it is what the lab
        and TigerHyperLab record, so there is no second numbering to correlate.

        An operation invoked with no session runs in a one-operation session of
        the lab's own, starting from the baseline and handing the VM back when
        it ends; a run that chains jobs opens one.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [Parameter(Mandatory)] [string] $SessionId,
        [string] $Description,
        [string] $ResultPath
    )

    $arguments = [System.Collections.Generic.List[string]]::new()
    foreach ($item in @('-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File')) { $arguments.Add($item) }
    $arguments.Add((ConvertTo-CommandLineArgument (Join-Path $LabRoot 'Open-TigerWinLabSession.ps1')))
    $arguments.Add('-SessionId'); $arguments.Add((ConvertTo-CommandLineArgument $SessionId))
    if (-not [string]::IsNullOrWhiteSpace($Description)) { $arguments.Add('-Description'); $arguments.Add((ConvertTo-CommandLineArgument $Description)) }
    if (-not [string]::IsNullOrWhiteSpace($ResultPath)) {
        $null = New-Item -ItemType Directory -Path (Split-Path -Parent $ResultPath) -Force
        $arguments.Add('-ResultPath'); $arguments.Add((ConvertTo-CommandLineArgument $ResultPath))
    }
    $pwsh = (Get-Process -Id $PID).Path
    $process = Start-Process -FilePath $pwsh -ArgumentList $arguments.ToArray() -PassThru -NoNewWindow -Wait
    if ($process.ExitCode -ne 0) { throw "Opening lab session '$SessionId' failed with exit code $($process.ExitCode)." }
    $script:LabSessionId = $SessionId
    $SessionId
}

function Set-TigerSetupLabSessionContext {
    <#
        .SYNOPSIS
        Names the open lab session the next entry points belong to.

        .DESCRIPTION
        Enter-TigerSetupLabSession makes the session it opens the current one,
        which is all a run with one session needs. A driver that keeps two
        sessions open at once - a lifecycle proof interleaving two consumers -
        switches between them here. An empty id leaves every session.
    #>
    [CmdletBinding()]
    param([AllowEmptyString()] [string] $SessionId)

    $script:LabSessionId = $(if ([string]::IsNullOrWhiteSpace($SessionId)) { $null } else { $SessionId })
}

function Exit-TigerSetupLabSession {
    <#
        .SYNOPSIS
        Ends the run's lab session and reports what its resources then did.

        .DESCRIPTION
        Ending the session hands every VM the run still holds - an active lease,
        or state a job preserved for a next job that never came - to the lab's
        own maintenance, which shuts it down, restores the baseline and leaves
        it Off, in a process of its own. The call returns as soon as that
        handoff is durable, so a run never waits for a guest to shut down; it
        is called on the failing paths too, and it reports rather than throws,
        so a row's own outcome is never replaced by the outcome of tidying up.

        **The handoff and the normalization are two transitions.** The lab says
        per VM what it was handed and whether its maintenance process started;
        whether that process converged is read afterwards with
        Get-TigerSetupLabVmState, which is how a driver that must know the VM
        is back at baseline - the next run, or a lifecycle proof - asks. A VM
        the lab could not start maintenance for stays owed, and the next lease
        on it performs the normalization first.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [string] $SessionId,
        [string] $ResultPath
    )

    if ([string]::IsNullOrWhiteSpace($SessionId)) { $SessionId = [string] $script:LabSessionId }
    if ([string]::IsNullOrWhiteSpace($SessionId)) { return $null }
    $arguments = [System.Collections.Generic.List[string]]::new()
    foreach ($item in @('-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File')) { $arguments.Add($item) }
    $arguments.Add((ConvertTo-CommandLineArgument (Join-Path $LabRoot 'Close-TigerWinLabSession.ps1')))
    $arguments.Add('-SessionId'); $arguments.Add((ConvertTo-CommandLineArgument $SessionId))
    if (-not [string]::IsNullOrWhiteSpace($ResultPath)) {
        $null = New-Item -ItemType Directory -Path (Split-Path -Parent $ResultPath) -Force
        $arguments.Add('-ResultPath'); $arguments.Add((ConvertTo-CommandLineArgument $ResultPath))
    }
    if ([string] $script:LabSessionId -eq $SessionId) { $script:LabSessionId = $null }
    $pwsh = (Get-Process -Id $PID).Path
    $exitCode = $null
    try {
        $process = Start-Process -FilePath $pwsh -ArgumentList $arguments.ToArray() -PassThru -NoNewWindow -Wait
        $exitCode = $process.ExitCode
    }
    catch {
        Write-Host "  the lab session '$SessionId' could not be ended: $($_.Exception.Message)"
        return [pscustomobject]@{ sessionId = $SessionId; ended = $false; exitCode = $null; resultPath = $ResultPath }
    }
    if ($exitCode -ne 0) { Write-Host "  ending the lab session '$SessionId' exited $exitCode." }

    # Per VM the run still held: what it held and that it is now the lab's to
    # normalize. The maintenance process either started or the VM stays owed;
    # both are reported, neither is a failure of the run.
    $handedOver = @()
    $maintenance = $null
    if (-not [string]::IsNullOrWhiteSpace($ResultPath) -and (Test-Path -LiteralPath $ResultPath -PathType Leaf)) {
        try {
            $report = Get-Content -LiteralPath $ResultPath -Raw -Encoding UTF8 | ConvertFrom-Json
            $handedOver = @(@($report.resources) | Where-Object { $null -ne $_ })
            $maintenance = $report.PSObject.Properties['maintenance'].Value
        }
        catch {
            Write-Host "  the lab session '$SessionId' report could not be read: $($_.Exception.Message)"
        }
    }
    foreach ($resource in $handedOver) {
        Write-Host "  handed to the lab: $($resource.name) held $($resource.held) - $($resource.handoff)"
    }
    if ($null -ne $maintenance -and -not [bool] $maintenance.started) {
        Write-Host "  OWED: the lab did not start maintenance - $($maintenance.message)"
    }
    [pscustomobject]@{
        sessionId = $SessionId
        ended = ($exitCode -eq 0)
        exitCode = $exitCode
        resultPath = $ResultPath
        handedOver = @($handedOver | ForEach-Object { [string] $_.name })
        maintenanceStarted = $(if ($null -eq $maintenance) { $null } else { [bool] $maintenance.started })
    }
}

function Get-TigerSetupRowStepPolicy {
    <#
        .SYNOPSIS
        The lease policies of one step of a chained row.

        .DESCRIPTION
        A row is several jobs on one VM state: install, then verify, then
        upgrade, then uninstall. The first step starts from the baseline and
        every step preserves its result for the next, within the run's
        session; the run's session end is what hands the VM back to the lab.
        A driver splats this into the job helpers instead of spelling the
        policies out at every step.
    #>
    [CmdletBinding()]
    param(
        # The step starts from the baseline rather than from what the previous step left.
        [switch] $FromBaseline
    )

    @{
        EntryPolicy = $(if ($FromBaseline) { 'Baseline' } else { 'DontCare' })
        ExitPolicy = 'PreserveUntilSessionEndOrNextLease'
    }
}

function Get-TigerSetupLabVmState {
    <#
        .SYNOPSIS
        Reads the lab's state of a baseline's VM: who holds it, or what the lab is doing with it.

        .DESCRIPTION
        Read-only and needs no session or lease. The state is the lab's own
        (Available, Leased, Preserved, Normalizing, Recovering, Faulted) with
        its reason and the VM's power state; a driver reads it to prove that a
        run left the VM back at baseline and Off, or to wait for that.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [Parameter(Mandatory)] [string] $Baseline,
        [Parameter(Mandatory)] [string] $ResultPath
    )

    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $ResultPath) -Force
    Remove-Item -LiteralPath $ResultPath -Force -ErrorAction SilentlyContinue
    $arguments = [System.Collections.Generic.List[string]]::new()
    foreach ($item in @('-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File')) { $arguments.Add($item) }
    $arguments.Add((ConvertTo-CommandLineArgument (Join-Path $LabRoot 'Get-TigerWinLabSession.ps1')))
    $arguments.Add('-ResultPath'); $arguments.Add((ConvertTo-CommandLineArgument $ResultPath))
    $pwsh = (Get-Process -Id $PID).Path
    $process = Start-Process -FilePath $pwsh -ArgumentList $arguments.ToArray() -PassThru -NoNewWindow -Wait `
        -RedirectStandardOutput ([System.IO.Path]::ChangeExtension($ResultPath, '.stdout.log'))
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $ResultPath -PathType Leaf)) {
        throw "Reading the lab session state failed with exit code $($process.ExitCode)."
    }
    $state = Get-Content -LiteralPath $ResultPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $resource = @(@($state.resources) | Where-Object { $null -ne $_ -and [string] $_.baseline -eq $Baseline }) | Select-Object -First 1
    if ($null -eq $resource) { throw "The lab reports no VM for the '$Baseline' baseline." }
    $resource
}

function Wait-TigerSetupLabVmAvailable {
    <#
        .SYNOPSIS
        Waits, bounded, until the lab reports a baseline's VM Available again.

        .DESCRIPTION
        After a session ends the lab normalizes the VM in a process of its own.
        This polls the lab's state on a slow cadence - a shutdown and a
        checkpoint restore take a minute, not a second - and returns the final
        resource, or throws with the state the VM was left in: a Faulted VM is
        the lab saying it could not, and never reports Available by accident.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [Parameter(Mandatory)] [string] $Baseline,
        [Parameter(Mandatory)] [string] $ResultPath,
        [int] $TimeoutMinutes = 10,
        [int] $PollSeconds = 15
    )

    $deadline = [DateTimeOffset]::Now.AddMinutes($TimeoutMinutes)
    while ($true) {
        $resource = Get-TigerSetupLabVmState -LabRoot $LabRoot -Baseline $Baseline -ResultPath $ResultPath
        if ([string] $resource.state -eq 'Available') { return $resource }
        if ([string] $resource.state -eq 'Faulted') {
            throw "The lab could not normalize the '$Baseline' VM: $($resource.reason)"
        }
        if ([DateTimeOffset]::Now -ge $deadline) {
            throw "The '$Baseline' VM was still '$($resource.state)' after $TimeoutMinutes minutes: $($resource.reason)"
        }
        Start-Sleep -Seconds $PollSeconds
    }
}

function New-TigerSetupRecoverySpec {
    <#
        .SYNOPSIS
        Writes a TigerWinLab recovery specification for a TigerSetup installer run.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]{0,31}$')] [string] $Name,
        [Parameter(Mandatory)] [string] $DisplayName,
        [Parameter(Mandatory)] [string] $InstallRoot,
        [Parameter(Mandatory)] [string] $InstallerPath,
        [string[]] $InstallerArguments = @(),
        [string] $LogArgumentTemplate = '--log {path}',
        [Parameter(Mandatory)] [ValidateSet('process', 'reboot', 'powerOff')] [string] $Method,
        [ValidateRange(1, 600)] [int] $AfterSeconds = 10,
        # A condition the lab waits for before it interrupts, instead of a
        # number of seconds after launch. TigerSetup's own fault injection
        # creates a file when it reaches a journal boundary, so a 'signal'
        # trigger cuts the machine while the run is provably sitting there.
        [hashtable] $Trigger,
        [string[]] $ProcessNames = @(),
        [string] $RegistrationKeyName,
        [string[]] $StatePaths = @(),
        [ValidateSet('installer', 'path')] [string] $RecoveryCommand = 'installer',
        [string] $RecoveryPath,
        [string[]] $RecoveryArguments = @(),
        [int[]] $RecoverySuccessExitCodes = @(0),
        [int] $RecoveryTimeoutMinutes = 10,
        [nullable[bool]] $ExpectInstallRootExists,
        [nullable[bool]] $ExpectRegistrationPresent,
        [nullable[int]] $ExpectMinimumFileCount,
        [Parameter(Mandatory)] [string] $OutputPath
    )

    if (-not (Test-Path -LiteralPath $InstallerPath -PathType Leaf)) {
        throw "The installer '$InstallerPath' does not exist."
    }
    if ($ProcessNames.Count -eq 0) {
        $ProcessNames = @([System.IO.Path]::GetFileNameWithoutExtension($InstallerPath))
    }
    if ([string]::IsNullOrWhiteSpace($RegistrationKeyName)) {
        # The lab's state inventory reads state.registrationKeyName unconditionally
        # (TigerWinLab 763169f, Get-RecoveryState.ps1 under strict mode), so a
        # product that registers nothing still names a key; the lab then reports
        # the registration absent, which is the truth.
        $RegistrationKeyName = "TigerSetup-none-$Name"
    }

    $installer = [ordered]@{
        kind = 'tigersetup'
        path = (Resolve-Path -LiteralPath $InstallerPath).Path
        arguments = @($InstallerArguments)
    }
    if (-not [string]::IsNullOrWhiteSpace($LogArgumentTemplate)) { $installer.logArgumentTemplate = $LogArgumentTemplate }

    $state = [ordered]@{ paths = @($StatePaths) }
    if (-not [string]::IsNullOrWhiteSpace($RegistrationKeyName)) { $state.registrationKeyName = $RegistrationKeyName }

    $recovery = [ordered]@{
        command = $RecoveryCommand
        arguments = @($RecoveryArguments)
        successExitCodes = @($RecoverySuccessExitCodes)
        timeoutMinutes = $RecoveryTimeoutMinutes
    }
    if ($RecoveryCommand -eq 'path') { $recovery.path = $RecoveryPath }

    $afterRecovery = [ordered]@{}
    if ($null -ne $ExpectInstallRootExists) { $afterRecovery.installRootExists = [bool] $ExpectInstallRootExists }
    if ($null -ne $ExpectRegistrationPresent) { $afterRecovery.registrationPresent = [bool] $ExpectRegistrationPresent }
    if ($null -ne $ExpectMinimumFileCount) { $afterRecovery.minimumFileCount = [int] $ExpectMinimumFileCount }

    $interruption = [ordered]@{ method = $Method; afterSeconds = $AfterSeconds; processNames = @($ProcessNames) }
    if ($null -ne $Trigger) { $interruption.trigger = $Trigger }

    $spec = [ordered]@{
        schemaVersion = 1
        name = $Name
        product = [ordered]@{ displayName = $DisplayName; installRoot = $InstallRoot }
        installer = $installer
        interruption = $interruption
        state = $state
        recovery = $recovery
        expected = [ordered]@{ afterRecovery = $afterRecovery }
    }

    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $OutputPath) -Force
    [System.IO.File]::WriteAllText($OutputPath, ($spec | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
    (Resolve-Path -LiteralPath $OutputPath).Path
}

function Invoke-TigerSetupGuestCommands {
    <#
        .SYNOPSIS
        Runs Setup.exe commands inside the guest through a plain TigerWinLab job.

        .DESCRIPTION
        The request describes files to stage outside the job workspace, the
        commands to run with their arguments and timeouts, the logs to collect
        and the directories to inventory. The guest entry script
        (guest\Invoke-SetupCommands.ps1) executes it and writes the structured
        result the job embeds. With -EntryPolicy DontCare the job runs against
        whatever state the previous job preserved, which is how rows are chained.

        A command runs as the job account unless it, or the request, names a
        desktop with "runAs": the job then asks the lab for the sessions those
        commands need, and the payload uses the lab's own rather than starting
        anything of its own.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [string] $Baseline,
        [Parameter(Mandatory)] [hashtable] $Request,
        [string[]] $PayloadFiles = @(),
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $ResultPath,
        [Parameter(Mandatory)] [string] $OutputRoot,
        # The lab's lease policies for this job: how the VM is taken and what becomes of it
        # afterwards. Omitted, the lab starts the job from the baseline and takes the VM back
        # when it ends; a chained row passes Get-TigerSetupRowStepPolicy instead.
        [ValidateSet('Baseline', 'DontCare')] [string] $EntryPolicy,
        [ValidateSet('DontCare', 'PreserveUntilSessionEndOrNextLease')] [string] $ExitPolicy,
        [string] $NetworkState,
        [string] $Language,
        [int] $ScalePercent = 0,
        [int] $TimeoutMinutes = 15
    )

    $payloadRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('TigerSetupLab-' + [Guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $payloadRoot -Force
    try {
        Copy-Item -LiteralPath (Join-Path $script:GuestScriptRoot 'Invoke-SetupCommands.ps1') -Destination $payloadRoot -Force
        # Where the commands run decides which sessions the job asks the lab to
        # establish. -ElevatedDesktop implies the interactive one, because the
        # elevated agent joins the signed-in user's desktop rather than one of
        # its own.
        $requestRunAs = $(if ($Request.ContainsKey('runAs')) { [string] $Request.runAs } else { 'job' })
        if ([string]::IsNullOrWhiteSpace($requestRunAs)) { $requestRunAs = 'job' }
        $sessionKinds = @(@($Request['commands']) | ForEach-Object {
                if ($null -eq $_) { return }
                $named = $(if ($_ -is [hashtable] -or $_ -is [System.Collections.Specialized.OrderedDictionary]) { [string] $_['runAs'] } else { [string] $_.runAs })
                if ([string]::IsNullOrWhiteSpace($named)) { $requestRunAs } else { $named }
            })
        $unknown = @($sessionKinds | Where-Object { $_ -notin @('job', 'interactiveUser', 'elevatedUser') } | Sort-Object -Unique)
        if ($unknown.Count -gt 0) { throw "Unknown runAs $($unknown -join ', '); use 'job', 'interactiveUser' or 'elevatedUser'." }
        $needsElevatedDesktop = $sessionKinds -contains 'elevatedUser'
        $needsDesktop = $needsElevatedDesktop -or ($sessionKinds -contains 'interactiveUser')
        foreach ($file in $PayloadFiles) {
            if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Payload file '$file' does not exist." }
            Copy-Item -LiteralPath $file -Destination (Join-Path $payloadRoot (Split-Path -Leaf $file)) -Force
        }
        [System.IO.File]::WriteAllText(
            (Join-Path $payloadRoot 'request.json'),
            ($Request | ConvertTo-Json -Depth 8),
            [System.Text.UTF8Encoding]::new($false))

        $parameters = @{
            PayloadPath = $payloadRoot
            EntryScript = 'Invoke-SetupCommands.ps1'
            Name = $Name
            EntryPolicy = $EntryPolicy
            ExitPolicy = $ExitPolicy
        }
        if ($needsDesktop) { $parameters.Desktop = $true }
        if ($needsElevatedDesktop) { $parameters.ElevatedDesktop = $true }
        if (-not [string]::IsNullOrWhiteSpace($Baseline)) { $parameters.Baseline = $Baseline }
        if (-not [string]::IsNullOrWhiteSpace($NetworkState)) { $parameters.NetworkState = $NetworkState }
        if (-not [string]::IsNullOrWhiteSpace($Language)) { $parameters.Language = $Language }
        if ($ScalePercent -gt 0) { $parameters.ScalePercent = $ScalePercent }
        Invoke-TigerWinLabEntryPoint -LabRoot $LabRoot -EntryPoint 'Invoke-TigerWinLabJob.ps1' -Parameters $parameters `
            -ResultPath $ResultPath -OutputRoot $OutputRoot -TimeoutMinutes $TimeoutMinutes
    }
    finally {
        Remove-Item -LiteralPath $payloadRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-TigerSetupPrepareDependencies {
    <#
        .SYNOPSIS
        Installs named runtimes in the guest with their vendors' installers,
        producing the "prepared" dependency state a matrix row starts from.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [string] $Baseline,
        # Each: @{ id; url; file; arguments; successExitCodes; detect = @{ kind = 'directory'|'registry'; path; pattern; keys; value } }
        [Parameter(Mandatory)] [hashtable[]] $Dependencies,
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $ResultPath,
        [Parameter(Mandatory)] [string] $OutputRoot,
        # The lab's lease policies for this job: how the VM is taken and what becomes of it
        # afterwards. Omitted, the lab starts the job from the baseline and takes the VM back
        # when it ends; a chained row passes Get-TigerSetupRowStepPolicy instead.
        [ValidateSet('Baseline', 'DontCare')] [string] $EntryPolicy,
        [ValidateSet('DontCare', 'PreserveUntilSessionEndOrNextLease')] [string] $ExitPolicy,
        [int] $TimeoutMinutes = 30
    )

    $payloadRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('TigerSetupLab-' + [Guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $payloadRoot -Force
    try {
        Copy-Item -LiteralPath (Join-Path $script:GuestScriptRoot 'Invoke-PrepareDependencies.ps1') -Destination $payloadRoot -Force
        [System.IO.File]::WriteAllText(
            (Join-Path $payloadRoot 'request.json'),
            (@{ dependencies = @($Dependencies) } | ConvertTo-Json -Depth 8),
            [System.Text.UTF8Encoding]::new($false))
        $parameters = @{
            PayloadPath = $payloadRoot
            EntryScript = 'Invoke-PrepareDependencies.ps1'
            Name = $Name
            EntryPolicy = $EntryPolicy
            ExitPolicy = $ExitPolicy
        }
        if (-not [string]::IsNullOrWhiteSpace($Baseline)) { $parameters.Baseline = $Baseline }
        Invoke-TigerWinLabEntryPoint -LabRoot $LabRoot -EntryPoint 'Invoke-TigerWinLabJob.ps1' -Parameters $parameters `
            -ResultPath $ResultPath -OutputRoot $OutputRoot -TimeoutMinutes $TimeoutMinutes
    }
    finally {
        Remove-Item -LiteralPath $payloadRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Assert-TigerSetupEngineIsCurrent {
    <#
        .SYNOPSIS
        Refuses to spend guest time validating an installer built from a stale
        engine.

        .DESCRIPTION
        A lab row measures the engine embedded in the installer, never the one
        in the workspace. Those two part company the moment somebody changes
        engine code and rebuilds the installer without rebuilding the engine
        first — and nothing about the result says so: the rows run, the
        screenshots look plausible, and every one of them is evidence about an
        engine that is no longer the product.

        The installer states the SHA-256 of the engine and of the loader it
        carries, and the builder takes both from `tigersetup-setup.exe` and
        `tigersetup-loader.exe` beside itself, so each can simply be compared.
        No binary beside the builder is not an error — the builder may be
        somewhere else entirely — but one that is there and does not match is.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $BuilderPath,
        [Parameter(Mandatory)] [object] $Facts,
        [Parameter(Mandatory)] [string] $InstallerPath
    )

    $builderDirectory = Split-Path -Parent (Resolve-Path -LiteralPath $BuilderPath).Path
    $short = { param($hash) if ($hash.Length -ge 16) { $hash.Substring(0, 16) } else { $hash } }
    foreach ($binary in @(
            @{ name = 'engine'; file = 'tigersetup-setup.exe'; embedded = [string] $Facts.engineSha256 },
            @{ name = 'loader'; file = 'tigersetup-loader.exe'; embedded = [string] $Facts.loaderSha256 })) {
        $path = Join-Path $builderDirectory $binary.file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
        $embedded = $binary.embedded
        if ([string]::IsNullOrWhiteSpace($embedded)) { continue }
        $current = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        if ($current -eq $embedded) { continue }
        throw ("'{0}' carries {1} {2}..., but the {1} beside the builder is {3}.... " -f $InstallerPath, $binary.name, (& $short $embedded), (& $short $current)) +
        "The rows would measure the $($binary.name) in the installer rather than the one that was just built. " +
        'Rebuild: cargo build --release, then rebuild the installers from their manifests.'
    }
}

function Get-TigerSetupPackageFacts {
    <#
        .SYNOPSIS
        Reads what a generated installer declares, through the builder's own
        inspection, so a lab specification is derived from the package rather
        than typed beside it.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $BuilderPath,
        [Parameter(Mandatory)] [string] $InstallerPath
    )

    $text = & $BuilderPath inspect $InstallerPath --json 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "tiger-setup inspect failed for '$InstallerPath': $text" }
    $json = $text | ConvertFrom-Json
    $package = $json.package
    $member = { param($object, $name) if ($null -ne $object -and $null -ne $object.PSObject.Properties[$name]) { $object.$name } else { $null } }
    $engine = & $member $package 'engine'
    $registration = & $member $json 'registration'
    $registrationKey = [string] (& $member $registration 'key_name')
    if ([string]::IsNullOrWhiteSpace($registrationKey)) { $registrationKey = [string] $package.id }
    [pscustomobject][ordered]@{
        id = [string] $package.id
        name = [string] $package.name
        version = [string] $package.version
        publisher = [string] $package.publisher
        # The engine and the loader the installer actually carries, which is
        # what a lab row measures — never the ones sitting in the workspace.
        engineSha256 = [string] (& $member $engine 'engine_sha256')
        loaderSha256 = [string] (& $member $engine 'loader_sha256')
        scopes = @($package.scopes | ForEach-Object { [string] $_ })
        userRoot = [string] $package.install_roots.user
        machineRoot = [string] $package.install_roots.machine
        files = @($json.files | ForEach-Object { [string] $_.path })
        registrationKey = $registrationKey
        shortcuts = @(& $member $json 'shortcuts' | Where-Object { $null -ne $_ })
        pathEntries = @(& $member $json 'path_entries' | Where-Object { $null -ne $_ })
        options = @(& $member $json 'options' | Where-Object { $null -ne $_ })
        dependencies = @(& $member $json 'dependencies' | Where-Object { $null -ne $_ })
        raw = $json
    }
}

function Test-TigerSetupOptionEnabled {
    param([object] $Facts, [string] $Option, [hashtable] $Options)
    if ([string]::IsNullOrWhiteSpace($Option)) { return $true }
    if ($null -ne $Options -and $Options.ContainsKey($Option)) { return [bool] $Options[$Option] }
    $declared = @($Facts.options | Where-Object { $null -ne $_ -and [string] $_.name -eq $Option }) | Select-Object -First 1
    if ($null -eq $declared) { return $false }
    [bool] $declared.default
}

function Get-TigerSetupExpectedShortcuts {
    <#
        .SYNOPSIS
        The lab's shortcut expectations for a scope, from the package's
        declared shortcuts and the options in effect.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object] $Facts,
        [Parameter(Mandatory)] [ValidateSet('user', 'machine')] [string] $Scope,
        [hashtable] $Options = @{},
        [Parameter(Mandatory)] [string] $InstallRoot
    )

    $expected = [System.Collections.Generic.List[object]]::new()
    foreach ($shortcut in @($Facts.shortcuts)) {
        if ($null -eq $shortcut) { continue }
        if (-not (Test-TigerSetupOptionEnabled -Facts $Facts -Option ([string] $shortcut.option) -Options $Options)) { continue }
        $location = [string] $shortcut.location
        $folder = if ($location -eq 'desktop') {
            if ($Scope -eq 'machine') { '%PUBLIC%\Desktop' } else { '%USERPROFILE%\Desktop' }
        }
        else {
            if ($Scope -eq 'machine') { '%ProgramData%\Microsoft\Windows\Start Menu\Programs' } else { '%APPDATA%\Microsoft\Windows\Start Menu\Programs' }
        }
        $subfolder = [string] $shortcut.folder
        if (-not [string]::IsNullOrWhiteSpace($subfolder)) { $folder = $folder + '\' + $subfolder.Replace('/', '\') }
        $target = $InstallRoot.TrimEnd('\') + '\' + ([string] $shortcut.target).Replace('/', '\')
        $expected.Add([ordered]@{ path = "$folder\$($shortcut.name).lnk"; target = $target; verifyParentFolder = (-not [string]::IsNullOrWhiteSpace($subfolder)) })
    }
    $expected.ToArray()
}

function Get-TigerSetupExpectedPathEntries {
    param([object] $Facts, [hashtable] $Options, [string] $InstallRoot)
    $entries = [System.Collections.Generic.List[string]]::new()
    foreach ($entry in @($Facts.pathEntries)) {
        if ($null -eq $entry) { continue }
        if (-not (Test-TigerSetupOptionEnabled -Facts $Facts -Option ([string] $entry.option) -Options $Options)) { continue }
        $relative = ([string] $entry.path).Replace('/', '\')
        $entries.Add($(if ([string]::IsNullOrWhiteSpace($relative)) { $InstallRoot } else { $InstallRoot.TrimEnd('\') + '\' + $relative }))
    }
    $entries.ToArray()
}

function Get-TigerSetupInstallRoot {
    param([object] $Facts, [string] $Scope)
    $root = if ($Scope -eq 'machine') { [string] $Facts.machineRoot } else { [string] $Facts.userRoot }
    # The lab expands %NAME% with the guest's environment, which spells the
    # standard names the way Windows does.
    $root -replace '%PROGRAMFILES%', '%ProgramFiles%' -replace '%PROGRAMDATA%', '%ProgramData%'
}

function New-TigerSetupInstallerSpec {
    <#
        .SYNOPSIS
        Writes a TigerWinLab installer-scenario specification for a generated
        installer, deriving the expectations from the package itself.

        .DESCRIPTION
        The lab replaces {path} in one template element and joins it into the
        command line with spaces, so the documented `--log <path>` form splits
        into the two arguments the installer expects. A log path containing a
        space would not survive that; the lab's log root has none.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [object] $Facts,
        [Parameter(Mandatory)] [string] $InstallerPath,
        [Parameter(Mandatory)] [ValidateSet('user', 'machine')] [string] $Scope,
        [string[]] $ExtraInstallArguments = @(),
        [hashtable] $Options = @{},
        [string] $UpgradeFromPath,
        [string] $UpgradeFromVersion,
        [string[]] $ExpectedFiles = @(),
        [int] $MinimumFileCount = 0,
        [string] $VersionFile,
        [object[]] $Smoke = @(),
        [hashtable] $Interactive,
        [int] $InstallTimeoutMinutes = 20,
        [Parameter(Mandatory)] [string] $OutputPath
    )

    $installRoot = Get-TigerSetupInstallRoot -Facts $Facts -Scope $Scope
    $installArguments = @('install', '--quiet', '--scope', $Scope) + @($ExtraInstallArguments)
    foreach ($key in $Options.Keys) { $installArguments += @('--option', $key, $(if ([bool] $Options[$key]) { 'on' } else { 'off' })) }

    $spec = [ordered]@{
        schemaVersion = 2
        name = $Name
        scope = $Scope
        product = [ordered]@{
            displayName = [string] $Facts.name
            appId = [string] $Facts.id
            publisher = [string] $Facts.publisher
            installRoot = $installRoot
        }
        installer = [ordered]@{
            kind = 'tigersetup'
            path = (Resolve-Path -LiteralPath $InstallerPath).Path
            version = [string] $Facts.version
            install = [ordered]@{ arguments = @($installArguments); successExitCodes = @(0); timeoutMinutes = $InstallTimeoutMinutes }
            uninstall = [ordered]@{ source = 'registration'; registrationValue = 'QuietUninstallString'; successExitCodes = @(0); timeoutMinutes = 10 }
            log = [ordered]@{ argumentTemplate = '--log {path}' }
        }
        registration = [ordered]@{ keyName = [string] $Facts.registrationKey; matchPrefix = [string] $Facts.registrationKey }
        expected = [ordered]@{
            files = @($ExpectedFiles)
            minimumFileCount = $(if ($MinimumFileCount -gt 0) { $MinimumFileCount } else { [math]::Max(1, [int] ($Facts.files.Count * 0.9)) })
            pathEntries = @(Get-TigerSetupExpectedPathEntries -Facts $Facts -Options $Options -InstallRoot $installRoot)
            shortcuts = @(Get-TigerSetupExpectedShortcuts -Facts $Facts -Scope $Scope -Options $Options -InstallRoot $installRoot)
            smoke = @($Smoke)
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($VersionFile)) { $spec.expected.versionFile = $VersionFile }
    if (-not [string]::IsNullOrWhiteSpace($UpgradeFromPath)) {
        $spec.upgradeFrom = [ordered]@{ path = (Resolve-Path -LiteralPath $UpgradeFromPath).Path; version = $UpgradeFromVersion }
    }
    if ($null -ne $Interactive) { $spec.interactive = $Interactive }

    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $OutputPath) -Force
    [System.IO.File]::WriteAllText($OutputPath, ($spec | ConvertTo-Json -Depth 10), [System.Text.UTF8Encoding]::new($false))
    (Resolve-Path -LiteralPath $OutputPath).Path
}

function New-TigerSetupWinGetSpec {
    <#
        .SYNOPSIS
        Writes a TigerWinLab WinGet-scenario specification for a prepared
        manifest set and the installer it names.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [object] $Facts,
        [Parameter(Mandatory)] [string] $InstallerPath,
        [Parameter(Mandatory)] [string] $ManifestDirectory,
        [Parameter(Mandatory)] [string] $ExpectedUrl,
        [Parameter(Mandatory)] [string] $Identifier,
        [string[]] $ExpectedFiles = @(),
        [int] $MinimumFileCount = 0,
        [string] $VersionFile,
        [string[]] $Commands = @(),
        [object[]] $Dependencies = @(),
        [object[]] $Smoke = @(),
        [Parameter(Mandatory)] [string] $OutputPath
    )

    $installRoot = Get-TigerSetupInstallRoot -Facts $Facts -Scope 'machine'
    $spec = [ordered]@{
        schemaVersion = 1
        name = $Name
        package = [ordered]@{ identifier = $Identifier; version = [string] $Facts.version; productCode = [string] $Facts.registrationKey }
        manifestDirectory = (Resolve-Path -LiteralPath $ManifestDirectory).Path
        installer = [ordered]@{
            path = (Resolve-Path -LiteralPath $InstallerPath).Path
            type = 'exe'
            scope = 'machine'
            architecture = 'x64'
            expectedUrl = $ExpectedUrl
            successExitCodes = @(0)
        }
        hashMismatchProbe = [ordered]@{ enabled = $true; expectedOutputPattern = '(?i)hash' }
        expected = [ordered]@{
            installRoot = $installRoot
            files = @($ExpectedFiles)
            minimumFileCount = $(if ($MinimumFileCount -gt 0) { $MinimumFileCount } else { [math]::Max(1, [int] ($Facts.files.Count * 0.9)) })
            machinePathEntries = @(Get-TigerSetupExpectedPathEntries -Facts $Facts -Options @{} -InstallRoot $installRoot)
            commands = @($Commands)
            dependencies = @($Dependencies)
            smoke = @($Smoke)
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($VersionFile)) { $spec.expected.versionFile = $VersionFile }
    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $OutputPath) -Force
    [System.IO.File]::WriteAllText($OutputPath, ($spec | ConvertTo-Json -Depth 10), [System.Text.UTF8Encoding]::new($false))
    (Resolve-Path -LiteralPath $OutputPath).Path
}

function Invoke-TigerSetupWizardCapture {
    <#
        .SYNOPSIS
        Captures every page of a wizard executable on the lab's interactive
        desktop at the requested display language and scale.

        .DESCRIPTION
        A plain job, started with -Desktop so that the lab establishes the
        interactive session and hands it to the payload. The payload carries the
        executable and nothing of the lab's own: a consumer that copied the
        lab's guest scripts into its payload would pin itself to whichever
        version of them it last copied.

        guest\Invoke-WizardCapture.ps1 launches the executable in that session,
        captures each page and its UI Automation tree, and advances with the
        given keys. Screenshots land under the lab's artifacts for the job; the
        result names them and reports the session's measured DPI and scale.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [string] $Baseline,
        # Each wizard: @{ ExecutablePath; Arguments; CaptureName; TitlePattern; MaxPages; AdvanceKeys;
        #                 AdvanceByPage; PageTimeoutSeconds;
        #                 GuestPath = $true when ExecutablePath names a program already in the guest }
        [Parameter(Mandatory)] [hashtable[]] $Wizards,
        [string] $Language,
        [int] $ScalePercent = 0,
        [ValidateSet('', 'light', 'dark')] [string] $Theme = '',
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $ResultPath,
        [Parameter(Mandatory)] [string] $OutputRoot,
        # The lab's lease policies for this job: how the VM is taken and what becomes of it
        # afterwards. Omitted, the lab starts the job from the baseline and takes the VM back
        # when it ends; a chained row passes Get-TigerSetupRowStepPolicy instead.
        [ValidateSet('Baseline', 'DontCare')] [string] $EntryPolicy,
        [ValidateSet('DontCare', 'PreserveUntilSessionEndOrNextLease')] [string] $ExitPolicy,
        [int] $TimeoutMinutes = 20
    )

    $payloadRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('TigerSetupLab-' + [Guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $payloadRoot -Force
    try {
        Copy-Item -LiteralPath (Join-Path $script:GuestScriptRoot 'Invoke-WizardCapture.ps1') -Destination $payloadRoot -Force
        $wizardRequests = foreach ($wizard in $Wizards) {
            $executable = [string] $wizard.ExecutablePath
            $guestPath = $wizard.ContainsKey('GuestPath') -and [bool] $wizard.GuestPath
            $captureName = [string] $wizard.CaptureName
            if ($captureName -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,40}$') { throw "'$captureName' is not a usable capture name." }
            if (-not $guestPath) {
                if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "The wizard executable '$executable' does not exist." }
                Copy-Item -LiteralPath $executable -Destination (Join-Path $payloadRoot (Split-Path -Leaf $executable)) -Force
            }
            @{
                executable = $(if ($guestPath) { $executable } else { Split-Path -Leaf $executable })
                arguments = @($(if ($wizard.ContainsKey('Arguments')) { $wizard.Arguments } else { @() }))
                captureName = $captureName
                titlePattern = $(if ($wizard.ContainsKey('TitlePattern')) { [string] $wizard.TitlePattern } else { '.' })
                maxPages = $(if ($wizard.ContainsKey('MaxPages')) { [int] $wizard.MaxPages } else { 8 })
                advanceKeys = @($(if ($wizard.ContainsKey('AdvanceKeys')) { $wizard.AdvanceKeys } else { @('Return') }))
                # AdvanceByPage: @{ '2' = @(@('Menu','A'), @('Return')) } — chords per page
                advanceByPage = $(if ($wizard.ContainsKey('AdvanceByPage')) { $wizard.AdvanceByPage } else { @{} })
                # How long one page may take to answer its keys. A page that
                # acquires a dependency is minutes of one page, and the capture
                # waits for the page to change rather than for a settle delay.
                pageTimeoutSeconds = $(if ($wizard.ContainsKey('PageTimeoutSeconds')) { [int] $wizard.PageTimeoutSeconds } else { 300 })
            }
        }
        $request = @{ wizards = @($wizardRequests) }
        [System.IO.File]::WriteAllText(
            (Join-Path $payloadRoot 'request.json'),
            ($request | ConvertTo-Json -Depth 6),
            [System.Text.UTF8Encoding]::new($false))

        $parameters = @{
            PayloadPath = $payloadRoot
            EntryScript = 'Invoke-WizardCapture.ps1'
            Name = $Name
            EntryPolicy = $EntryPolicy
            ExitPolicy = $ExitPolicy
            Desktop = $true
            # A capture is of the desktop as composited, so a menu or a window left
            # over from earlier work is in every picture this row takes. The lab
            # clears its own desktop before the payload starts; this says that a row
            # which cannot be given a clear one stops there instead of spending the
            # whole wizard producing captures that are not evidence.
            RequireClearDesktop = $true
        }
        if (-not [string]::IsNullOrWhiteSpace($Baseline)) { $parameters.Baseline = $Baseline }
        if (-not [string]::IsNullOrWhiteSpace($Language)) { $parameters.Language = $Language }
        if ($ScalePercent -gt 0) { $parameters.ScalePercent = $ScalePercent }
        if (-not [string]::IsNullOrWhiteSpace($Theme)) { $parameters.Theme = $Theme }
        Invoke-TigerWinLabEntryPoint -LabRoot $LabRoot -EntryPoint 'Invoke-TigerWinLabJob.ps1' -Parameters $parameters `
            -ResultPath $ResultPath -OutputRoot $OutputRoot -TimeoutMinutes $TimeoutMinutes
    }
    finally {
        Remove-Item -LiteralPath $payloadRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-TigerSetupElevationDrive {
    <#
        .SYNOPSIS
        Drives the all-users/elevation acceptance for one mode on the lab's
        interactive desktop and returns the lab run.

        .DESCRIPTION
        A plain job carrying the installer and guest\Invoke-ElevationAcceptance.ps1,
        started with -Desktop so the lab establishes the interactive session.

        The rows that raise a genuine UAC prompt (shield-refuse, complete-uac,
        complete-uac-admin) also ask for -HostConsole: the lab answers the
        prompt on the secure desktop from the host's console keyboard - the
        lab administrator's password into the credential prompt a standard
        user sees, the consent chord into the Yes/No prompt an administrator
        sees - so the transition is the real one and no policy is touched.
        complete-uac runs on the standard user's desktop, complete-uac-admin on
        an administrator's (-InteractiveKind administrator), which is the
        prompt a developer installing on their own machine gets. Both drive the
        elevated child through -ElevatedDesktop, because a high-integrity
        window cannot be clicked into from the standard session.
        complete-elevated is the already-elevated wizard on the lab's
        credential-backed session: no shield, no prompt.

        The payload writes the standard phase/check result the caller flattens.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $LabRoot,
        [string] $Baseline,
        [ValidateSet('shield-refuse', 'complete-uac', 'complete-uac-admin', 'complete-elevated', 'complete-user')] [string] $Mode,
        [Parameter(Mandatory)] [string] $ExecutablePath,
        [string] $Language,
        [int] $ScalePercent = 0,
        [ValidateSet('', 'light', 'dark')] [string] $Theme = '',
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $ResultPath,
        [Parameter(Mandatory)] [string] $OutputRoot,
        # The lab's lease policies for this job: how the VM is taken and what becomes of it
        # afterwards. Omitted, the lab starts the job from the baseline and takes the VM back
        # when it ends; a chained row passes Get-TigerSetupRowStepPolicy instead.
        [ValidateSet('Baseline', 'DontCare')] [string] $EntryPolicy,
        [ValidateSet('DontCare', 'PreserveUntilSessionEndOrNextLease')] [string] $ExitPolicy,
        [int] $TimeoutMinutes = 20
    )

    if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) { throw "The installer '$ExecutablePath' does not exist." }
    $payloadRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('TigerSetupLab-' + [Guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $payloadRoot -Force
    try {
        Copy-Item -LiteralPath (Join-Path $script:GuestScriptRoot 'Invoke-ElevationAcceptance.ps1') -Destination $payloadRoot -Force
        Copy-Item -LiteralPath $ExecutablePath -Destination (Join-Path $payloadRoot (Split-Path -Leaf $ExecutablePath)) -Force

        $request = @{
            # The administrator variant is the same acceptance on another desktop.
            mode = $(if ($Mode -eq 'complete-uac-admin') { 'complete-uac' } else { $Mode })
            executable = (Split-Path -Leaf $ExecutablePath)
            windowClass = 'TigerSetupWizard'
            questionClass = 'TigerSetupQuestion'
            stageRoot = 'C:\TigerSetupLab\elevation'
        }
        [System.IO.File]::WriteAllText(
            (Join-Path $payloadRoot 'request.json'),
            ($request | ConvertTo-Json -Depth 6),
            [System.Text.UTF8Encoding]::new($false))

        $parameters = @{
            PayloadPath = $payloadRoot
            EntryScript = 'Invoke-ElevationAcceptance.ps1'
            Name = $Name
            EntryPolicy = $EntryPolicy
            ExitPolicy = $ExitPolicy
            Desktop = $true
            # A shield capture is of the desktop as composited, so the row needs
            # a clear desktop; the completion modes tolerate one either way but
            # ask for the same precondition so every row is measured the same.
            RequireClearDesktop = ($Mode -in @('shield-refuse', 'complete-uac', 'complete-uac-admin'))
            # The elevated child and the already-elevated wizard are driven
            # through the lab's credential-elevated session.
            ElevatedDesktop = ($Mode -in @('complete-uac', 'complete-uac-admin', 'complete-elevated'))
            # A genuine prompt is answered from the host console.
            HostConsole = ($Mode -in @('shield-refuse', 'complete-uac', 'complete-uac-admin'))
            InteractiveKind = $(if ($Mode -eq 'complete-uac-admin') { 'administrator' } else { 'standard' })
        }
        if (-not [string]::IsNullOrWhiteSpace($Baseline)) { $parameters.Baseline = $Baseline }
        if (-not [string]::IsNullOrWhiteSpace($Language)) { $parameters.Language = $Language }
        if ($ScalePercent -gt 0) { $parameters.ScalePercent = $ScalePercent }
        if (-not [string]::IsNullOrWhiteSpace($Theme)) { $parameters.Theme = $Theme }
        Invoke-TigerWinLabEntryPoint -LabRoot $LabRoot -EntryPoint 'Invoke-TigerWinLabJob.ps1' -Parameters $parameters `
            -ResultPath $ResultPath -OutputRoot $OutputRoot -TimeoutMinutes $TimeoutMinutes
    }
    finally {
        Remove-Item -LiteralPath $payloadRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Get-TigerSetupCommandResult {
    <#
        .SYNOPSIS
        Picks one named command's record out of a guest-commands job result.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object] $JobRun,
        [Parameter(Mandatory)] [string] $CommandName
    )

    if ($null -eq $JobRun.result -or $null -eq $JobRun.result.PSObject.Properties['result'] -or $null -eq $JobRun.result.result) { return $null }
    $record = $JobRun.result.result
    if ($null -eq $record.PSObject.Properties['commands']) { return $null }
    $matches = @($record.commands | Where-Object { $_.name -eq $CommandName })
    if ($matches.Count -eq 0) { return $null }
    $matches[0]
}

function New-TigerSetupCheck {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $Code,
        [Parameter(Mandatory)] [ValidateSet('PASS', 'WARN', 'FAIL')] [string] $Status,
        [string] $Message = ''
    )

    [pscustomobject][ordered]@{ name = $Name; code = $Code; status = $Status; message = $Message }
}

function ConvertTo-TigerSetupCheck {
    <#
        .SYNOPSIS
        One lab check as a TigerSetup check, without assuming it carries every
        field.

        .DESCRIPTION
        Different lab entry points describe a check slightly differently — a
        WinGet scenario's checks have no `code`, for instance — and reading a
        field that is not there ends the row under `Set-StrictMode`. A row must
        not fail because the lab worded its evidence differently.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Prefix,
        [Parameter(Mandatory)] [object] $Check
    )

    $field = {
        param($name)
        if ($null -ne $Check -and $null -ne $Check.PSObject.Properties[$name]) { [string] $Check.$name } else { '' }
    }
    $name = & $field 'name'
    $code = & $field 'code'
    if ([string]::IsNullOrWhiteSpace($code)) {
        $code = ($name -replace '[^A-Za-z0-9]+', '.').Trim('.').ToLowerInvariant()
    }
    if ([string]::IsNullOrWhiteSpace($code)) { $code = 'unnamed' }
    $status = & $field 'status'
    if ([string]::IsNullOrWhiteSpace($status)) { $status = 'WARN' }
    New-TigerSetupCheck -Name "$Prefix/$name" -Code "$Prefix.$code" -Status $status -Message (& $field 'message')
}

function ConvertTo-TigerSetupFlattenedChecks {
    <#
        .SYNOPSIS
        Flattens a lab result's health and scenario checks under a prefix, so a
        lab check and a TigerSetup check with the same meaning keep one name.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Prefix,
        [Parameter(Mandatory)] [object] $LabRun
    )

    # A lab run costs minutes of guest time, so a caller that passes the wrong
    # thing is told what it passed instead of failing later on a missing
    # property.
    if ($null -eq $LabRun -or $null -eq $LabRun.PSObject.Properties['status'] -or $null -eq $LabRun.PSObject.Properties['result']) {
        throw "ConvertTo-TigerSetupFlattenedChecks '$Prefix' needs the object Invoke-TigerWinLabEntryPoint returns; got '$(if ($null -eq $LabRun) { '<null>' } else { $LabRun.GetType().FullName })'."
    }

    $checks = [System.Collections.Generic.List[object]]::new()
    $checks.Add((New-TigerSetupCheck -Name "$Prefix/lab run" -Code "$Prefix.lab.status" `
                -Status $(if ($LabRun.status -eq 'OK') { 'PASS' } else { 'FAIL' }) `
                -Message "TigerWinLab '$($LabRun.entryPoint)' ended with status $($LabRun.status) (exit $($LabRun.exitCode)) after $($LabRun.durationSeconds)s."))
    $result = $LabRun.result
    if ($null -eq $result) { return $checks.ToArray() }
    if ($null -ne $result.PSObject.Properties['health']) {
        foreach ($check in @($result.health)) {
            $checks.Add((ConvertTo-TigerSetupCheck -Prefix $Prefix -Check $check))
        }
    }
    if ($null -ne $result.PSObject.Properties['result'] -and $null -ne $result.result -and $null -ne $result.result.PSObject.Properties['phases']) {
        foreach ($phase in @($result.result.phases)) {
            foreach ($check in @($phase.checks)) {
                $checks.Add((ConvertTo-TigerSetupCheck -Prefix $Prefix -Check $check))
            }
        }
    }
    $checks.ToArray()
}

function Write-TigerSetupRowResult {
    <#
        .SYNOPSIS
        Writes one matrix row's result file and prints its summary.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Row,
        [Parameter(Mandatory)] [object[]] $Checks,
        [Parameter(Mandatory)] [string] $OutputPath,
        [object] $Environment,
        [hashtable] $Evidence = @{}
    )

    $counts = [ordered]@{
        checks = $Checks.Count
        pass = @($Checks | Where-Object status -eq 'PASS').Count
        warn = @($Checks | Where-Object status -eq 'WARN').Count
        fail = @($Checks | Where-Object status -eq 'FAIL').Count
    }
    $status = if ($counts.fail -gt 0) { 'FAIL' } elseif ($counts.warn -gt 0) { 'WARN' } else { 'PASS' }
    $record = [ordered]@{
        schemaVersion = 1
        row = $Row
        status = $status
        completedAt = [DateTimeOffset]::Now.ToString('o')
        counts = $counts
        environment = $Environment
        checks = @($Checks)
        evidence = $Evidence
    }
    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $OutputPath) -Force
    # Evidence is whatever a row gathered, and a row that spent minutes of
    # guest time must not lose its verdict because one piece of it will not
    # serialise. The verdict is written either way; the evidence is replaced
    # by the reason it could not be.
    $json = try { $record | ConvertTo-Json -Depth 12 } catch {
        $record.evidence = [ordered]@{ unserializable = $_.Exception.Message }
        $record | ConvertTo-Json -Depth 12
    }
    [System.IO.File]::WriteAllText($OutputPath, $json, [System.Text.UTF8Encoding]::new($false))

    Write-Host ""
    Write-Host ("== {0}: {1} ({2} PASS, {3} WARN, {4} FAIL)" -f $Row, $status, $counts.pass, $counts.warn, $counts.fail)
    foreach ($check in $Checks | Where-Object status -ne 'PASS') {
        Write-Host ("   {0,-4} {1}: {2}" -f $check.status, $check.name, $check.message)
    }
    [pscustomobject] $record
}

# Every public function is named for what it acts on, so the export list is
# that rule rather than a second list to keep in step with the first: a row
# that called a helper someone forgot to name here failed in the guest, minutes
# into a lab run. `ConvertTo-CommandLineArgument` is internal by not matching.
Export-ModuleMember -Function '*-TigerSetup*', 'Invoke-TigerWinLabEntryPoint'
