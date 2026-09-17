<#
    .SYNOPSIS
    Proves the lab's VM session and lease lifecycle through TigerSetup's own
    consumer path: default isolation, preserved state, and the asynchronous
    session end.

    .DESCRIPTION
    TigerSetup never touches a VM itself; it invokes TigerWinLab entry points,
    which lease the baseline VM from TigerHyperLab. These rows prove that the
    contract those layers own produces the isolation a consumer relies on, by
    installing the synthetic TigerSetupTestApp package and reading the machine
    afterwards from a second, independent session:

    - `default`   — session A runs one job with the lease policies omitted, which
                    installs the package; the lab then returns the VM to its
                    baseline and leaves it Off. Session B, also with the
                    policies omitted, finds no trace of the installation.
    - `preserve`  — session A installs under a lease that preserves the VM for
                    its next job; a job from session B is refused BUSY while the
                    state is preserved; session A's next job continues the
                    preserved state and finds the installation, and releases
                    the VM with the default exit; session B then finds the
                    baseline.
    - `end`       — session A installs and preserves the VM, then ends the
                    session: the call returns after the durable handoff, the VM
                    is unavailable while the lab normalizes it in its own
                    process, and it becomes Available at the baseline and Off;
                    session B then finds the baseline.

    A row's checks are what the guest reported — the engine's exit code, the
    install root and the registration read after each job — plus the lab's own
    state of the VM (Available, Preserved, Normalizing, ...) read between the
    jobs. No caller-side reset or cleanup exists in this driver: the lab's
    policies are the whole lifecycle.

    .EXAMPLE
    pwsh -File lab\Invoke-LeaseLifecycleRows.ps1 -InstallerPath artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $InstallerPath,
    [string[]] $Rows = @('all'),
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    # The prefix of this run's sessions; each row opens its own A and B sessions from it.
    [string] $SessionId,
    [string] $GuestStageRoot = 'C:\TigerSetupLab',
    [string] $BuilderPath,
    [int] $JobTimeoutMinutes = 15,
    # How long the lab may take to normalize a VM after a session ended before the row fails.
    [int] $NormalizationTimeoutMinutes = 10
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

$AllRows = @('default', 'preserve', 'end')
$Rows = @($Rows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($Rows -contains 'all') { $Rows = $AllRows }
$unknown = @($Rows | Where-Object { $AllRows -notcontains $_ })
if ($unknown.Count -gt 0) { throw "Unknown row(s): $($unknown -join ', '). Known rows: $($AllRows -join ', ')." }

function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($BuilderPath)) { $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe' }
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$installerFile = Split-Path -Leaf $InstallerPath
$stagedInstaller = Join-Path $GuestStageRoot $installerFile
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) { throw "The builder $BuilderPath is missing; build the release binaries first." }
$facts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $InstallerPath
Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -InstallerPath $InstallerPath -Facts $facts
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\lease-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'
if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-lease-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}

# The machine-scope installation is what a second session must not see.
$installRoot = Get-TigerSetupInstallRoot -Facts $facts -Scope 'machine'
$stateDirectory = "%ProgramData%\TigerSetup\$($facts.id)"
$registrationKey = "HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\$($facts.registrationKey)"

# ---------------------------------------------------------------------------
# Jobs and their evidence
# ---------------------------------------------------------------------------

function New-InstallRequest {
    $log = Join-Path $GuestStageRoot 'lease-install.log'
    @{
        stage = @(@{ source = $installerFile; destination = $stagedInstaller })
        commands = @(@{ name = 'install'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $log); timeoutSeconds = 900 })
        logs = @($log)
        inventory = @($installRoot, $stateDirectory)
        registry = @($registrationKey)
    }
}

function New-ReadRequest {
    @{
        commands = @()
        logs = @()
        inventory = @($installRoot, $stateDirectory)
        registry = @($registrationKey)
    }
}

function Invoke-Job {
    <#
        One TigerWinLab job in the named session, with the lease policies the
        row states - or none, for the lab's defaults. The session context of the
        lab module is switched to the session the job belongs to, because two
        sessions of one row interleave.
    #>
    param(
        [string] $Row, [string] $Suffix, [string] $Session, [hashtable] $Request, [string[]] $PayloadFiles = @(),
        [string] $EntryPolicy, [string] $ExitPolicy, [int] $TimeoutMinutes = $JobTimeoutMinutes
    )
    $arguments = @{
        LabRoot = $labRoot; Baseline = $Baseline; Request = $Request; PayloadFiles = $PayloadFiles; Name = "ts-lease-$Row-$Suffix".ToLowerInvariant()
        ResultPath = (Join-Path $ResultsRoot "runs\$Row-$Suffix.json"); OutputRoot = $labOutputRoot; TimeoutMinutes = $TimeoutMinutes
    }
    if (-not [string]::IsNullOrWhiteSpace($EntryPolicy)) { $arguments.EntryPolicy = $EntryPolicy }
    if (-not [string]::IsNullOrWhiteSpace($ExitPolicy)) { $arguments.ExitPolicy = $ExitPolicy }
    $policyText = if ($EntryPolicy -or $ExitPolicy) { "($EntryPolicy, $ExitPolicy)" } else { '(policies omitted)' }
    Write-Host "  [$Session] job $Suffix $policyText"
    Set-TigerSetupLabSessionContext -SessionId $Session
    $started = [DateTimeOffset]::Now
    $run = Invoke-TigerSetupGuestCommands @arguments
    $run | Add-Member -NotePropertyName elapsedSeconds -NotePropertyValue ([math]::Round(([DateTimeOffset]::Now - $started).TotalSeconds, 1)) -Force
    $status = if ($null -ne $run.result -and $null -ne (Get-Member2 $run.result 'status')) { [string] $run.result.status } else { $run.status }
    Write-Host "    -> $status in $($run.elapsedSeconds)s"
    $run
}

function Add-Check {
    param([System.Collections.Generic.List[object]] $Checks, [string] $Name, [string] $Code, [bool] $Condition, [string] $Message, [string] $Failure)
    $Checks.Add((New-TigerSetupCheck -Name $Name -Code $Code -Status $(if ($Condition) { 'PASS' } else { 'FAIL' }) -Message $(if ($Condition -or [string]::IsNullOrWhiteSpace($Failure)) { $Message } else { $Failure })))
}

function Test-RunUsable {
    param([object] $Run)
    $Run.status -eq 'OK' -and $null -ne $Run.result -and (Get-Member2 $Run.result 'status') -eq 'OK'
}

function Get-Inventory {
    param([object] $Run, [string] $Requested)
    $records = Get-Member2 (Get-Member2 $Run.result 'result') 'inventory'
    @($records | Where-Object { (Get-Member2 $_ 'requested') -eq $Requested }) | Select-Object -First 1
}

function Get-RegistryRecord {
    param([object] $Run, [string] $Requested)
    $records = Get-Member2 (Get-Member2 $Run.result 'result') 'registry'
    @($records | Where-Object { (Get-Member2 $_ 'requested') -eq $Requested }) | Select-Object -First 1
}

function Add-JobChecks {
    param([System.Collections.Generic.List[object]] $Checks, [object] $Run, [string] $Prefix)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix $Prefix -LabRun $Run) { $Checks.Add($check) }
}

function Add-InstalledCheck {
    <# What the guest reports about the installation: present after an install, absent at the baseline. #>
    param([System.Collections.Generic.List[object]] $Checks, [object] $Run, [string] $Prefix, [bool] $Expected)
    $root = Get-Inventory -Run $Run -Requested $installRoot
    $registration = Get-RegistryRecord -Run $Run -Requested $registrationKey
    $rootExists = $null -ne $root -and [bool] (Get-Member2 $root 'exists') -and [int] (Get-Member2 $root 'fileCount') -gt 0
    $registered = $null -ne $registration -and [bool] (Get-Member2 $registration 'exists')
    $word = if ($Expected) { 'present' } else { 'absent' }
    Add-Check $Checks "$Prefix/install root $word" "$Prefix.installRoot.$word" ($rootExists -eq $Expected) "The install root is $(if ($rootExists) { 'present' } else { 'absent' })." "The install root is $(if ($rootExists) { 'present' } else { 'absent' }); expected $word."
    Add-Check $Checks "$Prefix/registration $word" "$Prefix.registration.$word" ($registered -eq $Expected) "The ARP registration is $(if ($registered) { 'present' } else { 'absent' })." "The ARP registration is $(if ($registered) { 'present' } else { 'absent' }); expected $word."
}

function Add-InstallCommandCheck {
    param([System.Collections.Generic.List[object]] $Checks, [object] $Run, [string] $Prefix)
    $command = Get-TigerSetupCommandResult -JobRun $Run -CommandName 'install'
    $exit = if ($null -ne $command) { [int] $command.exitCode } else { -1 }
    Add-Check $Checks "$Prefix/install exit" "$Prefix.install.exit" ($exit -eq 0) "Setup.exe install exited $exit." "Setup.exe install exited $exit; expected 0."
}

function Get-VmState {
    param([string] $Row, [string] $Moment)
    $resource = Get-TigerSetupLabVmState -LabRoot $labRoot -Baseline $Baseline -ResultPath (Join-Path $ResultsRoot "state\$Row-$Moment.json")
    Write-Host "    lab: $($resource.state) / $($resource.powerState) - $($resource.reason)"
    $resource
}

function Add-VmStateCheck {
    param([System.Collections.Generic.List[object]] $Checks, [object] $Resource, [string] $Prefix, [string[]] $States, [string] $PowerState)
    Add-Check $Checks "$Prefix/lab state" "$Prefix.lab.state" ($Resource.state -in $States) "The lab reports the VM '$($Resource.state)'." "The lab reports the VM '$($Resource.state)'; expected $($States -join ' or '): $($Resource.reason)"
    if (-not [string]::IsNullOrWhiteSpace($PowerState)) {
        Add-Check $Checks "$Prefix/power state" "$Prefix.lab.powerState" ($Resource.powerState -eq $PowerState) "The VM is $($Resource.powerState)." "The VM is $($Resource.powerState); expected $PowerState."
    }
}

function Open-Session {
    param([string] $Session, [string] $Row, [string] $Purpose)
    $null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $Session -Description "TigerSetup lease lifecycle: $Purpose" -ResultPath (Join-Path $ResultsRoot "sessions\$Row-open-$Session.json")
    Write-Host "  session $Session opened"
}

function Close-Session {
    param([string] $Session, [string] $Row)
    Set-TigerSetupLabSessionContext -SessionId $Session
    $started = [DateTimeOffset]::Now
    $report = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $Session -ResultPath (Join-Path $ResultsRoot "sessions\$Row-close-$Session.json")
    $report | Add-Member -NotePropertyName elapsedSeconds -NotePropertyValue ([math]::Round(([DateTimeOffset]::Now - $started).TotalSeconds, 1)) -Force
    Write-Host "  session $Session ended in $($report.elapsedSeconds)s (handed over: $(@($report.handedOver) -join ', '))"
    $report
}

function Complete-Row {
    param([string] $Row, [System.Collections.Generic.List[object]] $Checks, [object] $Environment, [hashtable] $Evidence = @{})
    Write-TigerSetupRowResult -Row $Row -Checks $Checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") -Environment $Environment -Evidence $Evidence
}

# ---------------------------------------------------------------------------
# Rows
# ---------------------------------------------------------------------------

function Invoke-DefaultRow {
    <#
        Session A installs under the lab's defaults; the lab returns the VM to
        the baseline when the job ends. Session B sees the baseline.
    #>
    param([string] $Row)
    $checks = [System.Collections.Generic.List[object]]::new()
    $a = "$SessionId-$Row-a"
    $b = "$SessionId-$Row-b"
    Open-Session -Session $a -Row $Row -Purpose 'session A, default lease'
    Open-Session -Session $b -Row $Row -Purpose 'session B, default lease'
    try {
        $install = Invoke-Job -Row $Row -Suffix 'a-install' -Session $a -Request (New-InstallRequest) -PayloadFiles @($InstallerPath)
        Add-JobChecks -Checks $checks -Run $install -Prefix 'a-install'
        if (-not (Test-RunUsable $install)) { return Complete-Row -Row $Row -Checks $checks }
        Add-InstallCommandCheck -Checks $checks -Run $install -Prefix 'a-install'
        Add-InstalledCheck -Checks $checks -Run $install -Prefix 'a-install' -Expected $true
        # The default exit returned the VM before the job's process ended.
        $afterA = Get-VmState -Row $Row -Moment 'after-a'
        Add-VmStateCheck -Checks $checks -Resource $afterA -Prefix 'after-a' -States @('Available') -PowerState 'Off'

        $read = Invoke-Job -Row $Row -Suffix 'b-read' -Session $b -Request (New-ReadRequest)
        Add-JobChecks -Checks $checks -Run $read -Prefix 'b-read'
        if (-not (Test-RunUsable $read)) { return Complete-Row -Row $Row -Checks $checks }
        Add-InstalledCheck -Checks $checks -Run $read -Prefix 'b-read' -Expected $false
        $afterB = Get-VmState -Row $Row -Moment 'after-b'
        Add-VmStateCheck -Checks $checks -Resource $afterB -Prefix 'after-b' -States @('Available') -PowerState 'Off'
        Complete-Row -Row $Row -Checks $checks -Environment (Get-Member2 $read.result 'environment') -Evidence @{ results = @($install.resultPath, $read.resultPath) }
    }
    finally {
        $null = Close-Session -Session $a -Row $Row
        $null = Close-Session -Session $b -Row $Row
    }
}

function Invoke-PreserveRow {
    <#
        Session A preserves its installation between two jobs; session B is
        refused meanwhile; session A's second job continues the state and
        releases the VM with the default exit; session B then sees the baseline.
    #>
    param([string] $Row)
    $checks = [System.Collections.Generic.List[object]]::new()
    $a = "$SessionId-$Row-a"
    $b = "$SessionId-$Row-b"
    Open-Session -Session $a -Row $Row -Purpose 'session A, preserved state'
    Open-Session -Session $b -Row $Row -Purpose 'session B, refused while preserved'
    try {
        $install = Invoke-Job -Row $Row -Suffix 'a-install' -Session $a -Request (New-InstallRequest) -PayloadFiles @($InstallerPath) -EntryPolicy Baseline -ExitPolicy PreserveUntilSessionEndOrNextLease
        Add-JobChecks -Checks $checks -Run $install -Prefix 'a-install'
        if (-not (Test-RunUsable $install)) { return Complete-Row -Row $Row -Checks $checks }
        Add-InstallCommandCheck -Checks $checks -Run $install -Prefix 'a-install'
        Add-InstalledCheck -Checks $checks -Run $install -Prefix 'a-install' -Expected $true
        $preserved = Get-VmState -Row $Row -Moment 'preserved'
        Add-VmStateCheck -Checks $checks -Resource $preserved -Prefix 'preserved' -States @('Preserved') -PowerState 'Running'
        Add-Check $checks 'preserved/owner' 'preserved.lab.owner' ((Get-Member2 (Get-Member2 $preserved 'preserved') 'sessionId') -eq $a) "The state is preserved for session $a." "The state is preserved for '$(Get-Member2 (Get-Member2 $preserved 'preserved') 'sessionId')' instead of $a."

        # Session B is refused while A's state is preserved: the job never runs.
        $refused = Invoke-Job -Row $Row -Suffix 'b-refused' -Session $b -Request (New-ReadRequest) -EntryPolicy DontCare -ExitPolicy DontCare
        $refusedStatus = if ($null -ne $refused.result) { [string] (Get-Member2 $refused.result 'status') } else { $refused.status }
        Add-Check $checks 'b-refused/busy' 'b-refused.busy' ($refused.exitCode -eq 2 -and $refusedStatus -eq 'BUSY') "Session B was refused BUSY: $(Get-Member2 $refused.result 'message')" "Session B was answered '$refusedStatus' (exit $($refused.exitCode)) instead of BUSY."
        $stillPreserved = Get-VmState -Row $Row -Moment 'still-preserved'
        Add-VmStateCheck -Checks $checks -Resource $stillPreserved -Prefix 'still-preserved' -States @('Preserved') -PowerState 'Running'

        # Session A continues its preserved state under a new lease, and releases the VM.
        $continue = Invoke-Job -Row $Row -Suffix 'a-continue' -Session $a -Request (New-ReadRequest) -EntryPolicy DontCare -ExitPolicy DontCare
        Add-JobChecks -Checks $checks -Run $continue -Prefix 'a-continue'
        if (-not (Test-RunUsable $continue)) { return Complete-Row -Row $Row -Checks $checks }
        Add-InstalledCheck -Checks $checks -Run $continue -Prefix 'a-continue' -Expected $true
        $released = Get-VmState -Row $Row -Moment 'released'
        Add-VmStateCheck -Checks $checks -Resource $released -Prefix 'released' -States @('Available') -PowerState 'Off'

        $read = Invoke-Job -Row $Row -Suffix 'b-read' -Session $b -Request (New-ReadRequest)
        Add-JobChecks -Checks $checks -Run $read -Prefix 'b-read'
        if (-not (Test-RunUsable $read)) { return Complete-Row -Row $Row -Checks $checks }
        Add-InstalledCheck -Checks $checks -Run $read -Prefix 'b-read' -Expected $false
        Complete-Row -Row $Row -Checks $checks -Environment (Get-Member2 $read.result 'environment') -Evidence @{ results = @($install.resultPath, $refused.resultPath, $continue.resultPath, $read.resultPath) }
    }
    finally {
        $null = Close-Session -Session $a -Row $Row
        $null = Close-Session -Session $b -Row $Row
    }
}

function Invoke-EndRow {
    <#
        Session A preserves its installation and ends: the end returns after the
        durable handoff, the lab normalizes the VM in its own process, and the
        VM becomes Available at the baseline and Off. Session B then sees the
        baseline.
    #>
    param([string] $Row)
    $checks = [System.Collections.Generic.List[object]]::new()
    $a = "$SessionId-$Row-a"
    $b = "$SessionId-$Row-b"
    Open-Session -Session $a -Row $Row -Purpose 'session A, ended while preserving'
    Open-Session -Session $b -Row $Row -Purpose 'session B, after the handoff'
    $aEnded = $false
    try {
        $install = Invoke-Job -Row $Row -Suffix 'a-install' -Session $a -Request (New-InstallRequest) -PayloadFiles @($InstallerPath) -EntryPolicy Baseline -ExitPolicy PreserveUntilSessionEndOrNextLease
        Add-JobChecks -Checks $checks -Run $install -Prefix 'a-install'
        if (-not (Test-RunUsable $install)) { return Complete-Row -Row $Row -Checks $checks }
        Add-InstallCommandCheck -Checks $checks -Run $install -Prefix 'a-install'
        Add-InstalledCheck -Checks $checks -Run $install -Prefix 'a-install' -Expected $true
        $preserved = Get-VmState -Row $Row -Moment 'preserved'
        Add-VmStateCheck -Checks $checks -Resource $preserved -Prefix 'preserved' -States @('Preserved') -PowerState 'Running'

        # Ending the session hands the VM over and returns; the normalization runs on.
        $report = Close-Session -Session $a -Row $Row
        $aEnded = $true
        Add-Check $checks 'end/ended' 'end.session.ended' ([bool] $report.ended) "Session A ended in $($report.elapsedSeconds)s." "Session A did not end (exit $($report.exitCode))."
        Add-Check $checks 'end/handed over' 'end.session.handedOver' (@($report.handedOver).Count -eq 1) "The session handed its VM to the lab: $(@($report.handedOver) -join ', ')." "The session handed over $(@($report.handedOver).Count) VMs instead of 1."
        Add-Check $checks 'end/maintenance started' 'end.session.maintenanceStarted' ([bool] $report.maintenanceStarted) 'The lab started its maintenance process.' 'The lab did not start its maintenance process; the VM stays owed.'
        # A shutdown, a checkpoint restore and a power state take longer than the handoff: the end
        # returned before the guest could have been shut down and restored.
        Add-Check $checks 'end/returned without waiting' 'end.session.asynchronous' ($report.elapsedSeconds -lt 20) "Ending the session took $($report.elapsedSeconds)s." "Ending the session took $($report.elapsedSeconds)s, which is long enough to have waited for the normalization."
        $handedOver = Get-VmState -Row $Row -Moment 'handed-over'
        Add-VmStateCheck -Checks $checks -Resource $handedOver -Prefix 'handed-over' -States @('Normalizing', 'Available')
        Add-Check $checks 'handed-over/not usable by A' 'handed-over.lab.revoked' ($null -eq (Get-Member2 $handedOver 'preserved') -and ($null -eq (Get-Member2 $handedOver 'lease'))) 'Session A holds nothing on the VM.' 'Session A still holds the VM after its end.'

        # The lab converges without anybody waiting for it; this row waits only to prove it did.
        $waitStarted = [DateTimeOffset]::Now
        $available = Wait-TigerSetupLabVmAvailable -LabRoot $labRoot -Baseline $Baseline -ResultPath (Join-Path $ResultsRoot "state\$Row-wait.json") -TimeoutMinutes $NormalizationTimeoutMinutes
        $waited = [math]::Round(([DateTimeOffset]::Now - $waitStarted).TotalSeconds, 1)
        Write-Host "  lab normalized the VM within ${waited}s of the poll starting"
        Add-VmStateCheck -Checks $checks -Resource $available -Prefix 'normalized' -States @('Available') -PowerState 'Off'
        Add-Check $checks 'normalized/recorded' 'normalized.lab.recorded' (-not [string]::IsNullOrWhiteSpace([string] (Get-Member2 $available 'lastNormalizedAt'))) "The lab recorded the normalization at $(Get-Member2 $available 'lastNormalizedAt')." 'The lab did not record a normalization.'

        $read = Invoke-Job -Row $Row -Suffix 'b-read' -Session $b -Request (New-ReadRequest)
        Add-JobChecks -Checks $checks -Run $read -Prefix 'b-read'
        if (-not (Test-RunUsable $read)) { return Complete-Row -Row $Row -Checks $checks }
        Add-InstalledCheck -Checks $checks -Run $read -Prefix 'b-read' -Expected $false
        Complete-Row -Row $Row -Checks $checks -Environment (Get-Member2 $read.result 'environment') -Evidence @{ results = @($install.resultPath, $read.resultPath); sessionEnd = @($report.resultPath) }
    }
    finally {
        if (-not $aEnded) { $null = Close-Session -Session $a -Row $Row }
        $null = Close-Session -Session $b -Row $Row
    }
}

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

$summary = [System.Collections.Generic.List[object]]::new()
$runStarted = [DateTimeOffset]::Now
foreach ($row in $Rows) {
    Write-Host ''
    Write-Host "### $row ($Baseline)"
    $started = [DateTimeOffset]::Now
    try {
        $result = switch ($row) {
            'default' { Invoke-DefaultRow -Row $row }
            'preserve' { Invoke-PreserveRow -Row $row }
            'end' { Invoke-EndRow -Row $row }
        }
    }
    catch {
        Write-Host "  row failed: $($_.Exception.Message)"
        $result = Write-TigerSetupRowResult -Row $row -Checks @(New-TigerSetupCheck -Name "$row driver" -Code 'row.driver' -Status FAIL -Message $_.Exception.Message) -OutputPath (Join-Path $ResultsRoot "$row.json")
    }
    finally {
        Set-TigerSetupLabSessionContext -SessionId ''
    }
    $summary.Add([pscustomobject]@{ row = $row; status = $result.status; minutes = [math]::Round(([DateTimeOffset]::Now - $started).TotalMinutes, 1) })
}

Write-Host ''
Write-Host ('{0,-10} {1,-6} {2,6}' -f 'row', 'status', 'min')
foreach ($entry in $summary) { Write-Host ('{0,-10} {1,-6} {2,6}' -f $entry.row, $entry.status, $entry.minutes) }
Write-Host "Total: $([math]::Round(([DateTimeOffset]::Now - $runStarted).TotalMinutes, 1)) min. Results: $ResultsRoot"
$final = Get-TigerSetupLabVmState -LabRoot $labRoot -Baseline $Baseline -ResultPath (Join-Path $ResultsRoot 'state\final.json')
Write-Host "Final VM state: $($final.state) / $($final.powerState) - $($final.reason)"
if (@($summary | Where-Object { $_.status -ne 'PASS' }).Count -gt 0) { exit 1 }
exit 0
