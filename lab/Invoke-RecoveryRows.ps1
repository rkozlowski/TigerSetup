<#
    .SYNOPSIS
    Runs the interrupted-install, interrupted-upgrade and rollback rows of
    TigerSetup's transactional validation against TigerWinLab.

    .DESCRIPTION
    Each row is self-contained: it restores the baseline, produces the state it
    needs, interrupts a TigerSetup installer run at a journal/mutation boundary
    through the lab's recovery scenario (process kill, reboot or power-off), lets
    the engine recover, and then reads the machine through the engine's own
    verify --json and inspect --json plus the engine log. What passing means
    is TigerSetup-Validation.md §3: every interruption converges to a verified
    installed or absent state, an interrupted upgrade ends in exactly the old or
    the new version, and a power-off that damaged nothing proved nothing.

    The rows need a package with two versions whose payload is files only; the
    synthetic package under packages/test-app is the one they were written for.
    Installer file names must follow <name>-<version>-Setup.exe.

    .EXAMPLE
    pwsh -File lab\Invoke-RecoveryRows.ps1 -InstallerPath artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe `
        -UpgradeInstallerPath artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe -Rows install-process-kill
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $InstallerPath,
    [string] $UpgradeInstallerPath,
    [string[]] $Rows = @('all'),
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    # The lab session every operation of this run belongs to. One session means
    # the VM boots once for the whole run and is released for the lab's own
    # cleanup when the run ends, instead of being left powered on or shut down
    # under somebody else's work. Naming one joins an existing session.
    [string] $SessionId,
    [string] $DisplayName = 'TigerSetupTestApp',
    [string] $InstallRoot = '%LOCALAPPDATA%\Programs\TigerSetupTestApp',
    [string] $StateDirectory = '%LOCALAPPDATA%\TigerSetup\IT-Tiger.TigerSetupTestApp',
    [int] $MinimumFileCount = 60,
    # How long the run waits at the boundary after announcing it. The
    # interruption is triggered by the announcement rather than by a clock, so
    # this only has to outlast the lab's 100 ms trigger poll and the power-off
    # itself; it is not a window the cut has to be aimed into.
    [int] $HoldSeconds = 30,
    # How long the lab waits for the boundary to be announced before giving up.
    [ValidateRange(1, 600)] [int] $BoundaryTimeoutSeconds = 300,
    [int] $FaultSequence = 30,
    [int] $SmallFileSequence = 12,
    [string] $GuestStageRoot = 'C:\TigerSetupLab',
    # Used only to read what each installer declares, so that a row cannot
    # silently measure an engine older than the one just built.
    [string] $BuilderPath,
    [int] $ScenarioTimeoutMinutes = 30,
    [int] $JobTimeoutMinutes = 15
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

# The lab stages the recovery scenario's installer here and leaves it in place,
# which is where a following job finds the engine that owns the state.
$LabRecoveryStageRoot = 'C:\TigerWinLab\recovery'

$AllRows = @(
    'install-process-kill',
    'install-poweroff-unflushed',
    'install-poweroff-skipflush',
    'install-poweroff-skipflush-small',
    'install-reboot-before-commit',
    'uninstall',
    'upgrade-poweroff-prepared',
    'upgrade-poweroff-renamed',
    'upgrade-poweroff-before-commit',
    'upgrade-poweroff-after-commit',
    'upgrade-rollback-by-old-uninstaller'
)
# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Rows = @($Rows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($Rows -contains 'all') { $Rows = $AllRows }
$unknown = @($Rows | Where-Object { $AllRows -notcontains $_ })
if ($unknown.Count -gt 0) { throw "Unknown row(s): $($unknown -join ', '). Known rows: $($AllRows -join ', ')." }
if (@($Rows | Where-Object { $_ -like 'upgrade-*' }).Count -gt 0 -and [string]::IsNullOrWhiteSpace($UpgradeInstallerPath)) {
    throw 'The upgrade rows need -UpgradeInstallerPath.'
}

function Get-InstallerVersion {
    param([string] $Path)
    if ((Split-Path -Leaf $Path) -match '-(\d+\.\d+\.\d+)-Setup\.exe$') { return $Matches[1] }
    throw "Cannot read a version from installer name '$Path'; expected <name>-<version>-Setup.exe."
}

function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($BuilderPath)) { $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe' }
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$versionA = Get-InstallerVersion $InstallerPath
$installerFileA = Split-Path -Leaf $InstallerPath
$versionB = $null
$installerFileB = $null
if (-not [string]::IsNullOrWhiteSpace($UpgradeInstallerPath)) {
    $UpgradeInstallerPath = (Resolve-Path -LiteralPath $UpgradeInstallerPath).Path
    $versionB = Get-InstallerVersion $UpgradeInstallerPath
    $installerFileB = Split-Path -Leaf $UpgradeInstallerPath
}
# These rows exist to measure the engine's crash behaviour, so an installer
# carrying an older engine than the workspace makes every one of them evidence
# about the wrong product. One second here, against minutes of guest time.
if (Test-Path -LiteralPath $BuilderPath -PathType Leaf) {
    foreach ($installer in @($InstallerPath, $UpgradeInstallerPath | Where-Object { $_ })) {
        Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -InstallerPath $installer `
            -Facts (Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $installer)
    }
}
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'
$commonSetupArguments = @('--scope', 'user')
$stagedA = Join-Path $GuestStageRoot $installerFileA
$stagedB = if ($installerFileB) { Join-Path $GuestStageRoot $installerFileB } else { $null }

Write-Host "TigerWinLab: $labRoot"
Write-Host "Baseline:    $Baseline"
Write-Host "Installer A: $InstallerPath ($versionA)"
if ($versionB) { Write-Host "Installer B: $UpgradeInstallerPath ($versionB)" }
Write-Host "Results:     $ResultsRoot"

# ---------------------------------------------------------------------------
# Helpers that read the engine's evidence
# ---------------------------------------------------------------------------

function Test-LogHasCode {
    param([string[]] $Lines, [string] $Code)
    @($Lines | Where-Object { $_ -match ('\[' + [regex]::Escape($Code) + '\]') }).Count -gt 0
}

function Get-JobLog {
    <#
        The lines of one collected guest log, or an empty array when the job
        collected none — which is what a BUSY or failed run leaves behind.

        Every return is comma-wrapped on purpose: PowerShell unrolls a returned
        collection, so `return @()` returns *nothing* and the caller's variable
        becomes $null. Under Set-StrictMode the next `.Count` on it ends the
        row, which is a row's worth of guest time spent reporting an empty log.
    #>
    param([object] $JobRun, [string] $Path)
    $record = Get-Member2 (Get-Member2 $JobRun.result 'result') 'logs'
    if ($null -eq $record) { return , @() }
    $lines = Get-Member2 $record $Path
    if ($null -eq $lines) { return , @() }
    , @($lines | ForEach-Object { [string] $_ })
}

function Get-JobInventory {
    # Matches on the path as requested (with %VARS% unexpanded): the guest
    # expands it for its own account, which is not the host's.
    param([object] $JobRun, [string] $Path)
    $record = Get-Member2 (Get-Member2 $JobRun.result 'result') 'inventory'
    if ($null -eq $record) { return $null }
    $matches = @($record | Where-Object { (Get-Member2 $_ 'requested') -eq $Path })
    if ($matches.Count -eq 0) { return $null }
    $matches[0]
}

function Add-EngineReadChecks {
    <#
        Turns the verify/inspect command records of a guest job into checks.
        ExpectedVersions: the versions the installation may report; empty
        means the installation must be absent.
    #>
    param(
        [System.Collections.Generic.List[object]] $Checks,
        [string] $Prefix,
        [object] $JobRun,
        [string[]] $ExpectedVersions
    )

    $verify = Get-TigerSetupCommandResult -JobRun $JobRun -CommandName 'verify'
    $inspect = Get-TigerSetupCommandResult -JobRun $JobRun -CommandName 'inspect'
    if ($null -eq $verify -or $null -eq $inspect) {
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/engine read" -Code "$Prefix.engine.read" -Status FAIL -Message 'The verify/inspect job produced no command records.'))
        return $null
    }
    $verifyJson = Get-Member2 $verify 'json'
    $inspectJson = Get-Member2 $inspect 'json'
    $verifyStatus = [string] (Get-Member2 $verifyJson 'status')
    $installation = Get-Member2 $inspectJson 'installation'
    $transaction = Get-Member2 $inspectJson 'transaction'
    $installedVersion = [string] (Get-Member2 $installation 'version')

    $Checks.Add((New-TigerSetupCheck -Name "$Prefix/inspect exit" -Code "$Prefix.inspect.exit" `
                -Status $(if ($inspect.exitCode -eq 0 -and $null -ne $inspectJson) { 'PASS' } else { 'FAIL' }) `
                -Message "inspect --json exited $($inspect.exitCode); JSON $(if ($null -ne $inspectJson) { 'parsed' } else { 'missing' }). $($inspect.stderr)".Trim()))
    $Checks.Add((New-TigerSetupCheck -Name "$Prefix/no open transaction" -Code "$Prefix.transaction.closed" `
                -Status $(if ($null -eq $transaction) { 'PASS' } else { 'FAIL' }) `
                -Message $(if ($null -eq $transaction) { 'No transaction is open after recovery.' } else { "A transaction is still open: $($transaction | ConvertTo-Json -Compress)" })))

    if ($ExpectedVersions.Count -eq 0) {
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/installation absent" -Code "$Prefix.installation.absent" `
                    -Status $(if ($null -eq $installation) { 'PASS' } else { 'FAIL' }) `
                    -Message $(if ($null -eq $installation) { 'No installation is recorded.' } else { "An installation is still recorded: version $installedVersion." })))
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/verify reports absence" -Code "$Prefix.verify.absent" `
                    -Status $(if ($verifyStatus -eq 'not_installed') { 'PASS' } else { 'FAIL' }) `
                    -Message "verify --json status '$verifyStatus' (exit $($verify.exitCode))."))
    }
    else {
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/verify ok" -Code "$Prefix.verify.ok" `
                    -Status $(if ($verify.exitCode -eq 0 -and $verifyStatus -eq 'ok') { 'PASS' } else { 'FAIL' }) `
                    -Message "verify --json status '$verifyStatus' (exit $($verify.exitCode)); findings: $(@(Get-Member2 $verifyJson 'findings') | ConvertTo-Json -Compress -Depth 4)"))
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/installed version" -Code "$Prefix.installation.version" `
                    -Status $(if ($ExpectedVersions -contains $installedVersion) { 'PASS' } else { 'FAIL' }) `
                    -Message "The installation reports version '$installedVersion'; expected one of $($ExpectedVersions -join ', ')."))
    }
    [pscustomobject]@{ verify = $verifyJson; inspect = $inspectJson; installedVersion = $installedVersion }
}

function Add-LogCodeChecks {
    param(
        [System.Collections.Generic.List[object]] $Checks,
        [string] $Prefix,
        [string[]] $Lines,
        [string[]] $Required,
        [string[]] $Forbidden = @(),
        [string] $LogName
    )

    if ($Lines.Count -eq 0) {
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/$LogName present" -Code "$Prefix.log.present" -Status FAIL -Message "The log '$LogName' was not collected or is empty."))
        return
    }
    foreach ($code in $Required) {
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/$LogName records $code" -Code "$Prefix.log.$code" `
                    -Status $(if (Test-LogHasCode $Lines $code) { 'PASS' } else { 'FAIL' }) `
                    -Message "The engine log '$LogName' $(if (Test-LogHasCode $Lines $code) { 'records' } else { 'does not record' }) [$code]."))
    }
    foreach ($code in $Forbidden) {
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/$LogName without $code" -Code "$Prefix.log.no_$code" `
                    -Status $(if (Test-LogHasCode $Lines $code) { 'FAIL' } else { 'PASS' }) `
                    -Message "The engine log '$LogName' $(if (Test-LogHasCode $Lines $code) { 'records' } else { 'does not record' }) [$code]."))
    }
}

# ---------------------------------------------------------------------------
# Steps
# ---------------------------------------------------------------------------

function Invoke-RecoveryStep {
    param(
        [string] $Row,
        [string] $SpecName,
        [string] $InstallerHostPath,
        [string[]] $InstallerArguments,
        [string] $Method,
        [string] $RecoveryCommand = 'installer',
        [string] $RecoveryPath,
        [string[]] $RecoveryArguments,
        [nullable[bool]] $ExpectInstallRootExists,
        [nullable[int]] $ExpectMinimumFileCount,
        [switch] $FromBaseline,
        [string] $BoundarySignalPath
    )

    # The lab caps a specification name at 32 characters; the row name is the
    # identity that matters and the result files carry it in full.
    if ($SpecName.Length -gt 32) { $SpecName = $SpecName.Substring(0, 32).TrimEnd('-', '.') }
    # The engine announces the journal boundary it is holding at; the lab waits
    # for that file and interrupts then. A clock could only aim a cut at the
    # window an unflushed-write row needs; the signal puts it inside.
    $trigger = @{ kind = 'signal'; path = $BoundarySignalPath; timeoutSeconds = $BoundaryTimeoutSeconds }
    $specPath = New-TigerSetupRecoverySpec -Name $SpecName -DisplayName $DisplayName -InstallRoot $InstallRoot `
        -InstallerPath $InstallerHostPath -InstallerArguments $InstallerArguments -Method $Method `
        -Trigger $trigger -StatePaths @($StateDirectory) `
        -RecoveryCommand $RecoveryCommand -RecoveryPath $RecoveryPath -RecoveryArguments $RecoveryArguments `
        -RecoveryTimeoutMinutes 10 -ExpectInstallRootExists $ExpectInstallRootExists -ExpectMinimumFileCount $ExpectMinimumFileCount `
        -OutputPath (Join-Path $ResultsRoot "specs\$Row.json")
    $parameters = @{ SpecPath = $specPath; Baseline = $Baseline } + (Get-TigerSetupRowStepPolicy -FromBaseline:$FromBaseline)
    Write-Host "  recovery scenario: $Method on the announced boundary, fault '$($InstallerArguments[-3])'"
    Invoke-TigerWinLabEntryPoint -LabRoot $labRoot -EntryPoint 'Invoke-TigerWinLabRecoveryScenario.ps1' -Parameters $parameters `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-recovery.json") -OutputRoot $labOutputRoot -TimeoutMinutes $ScenarioTimeoutMinutes
}

function Get-BoundarySignalPath {
    <#
        .SYNOPSIS
        Where the engine announces that it has reached this row's boundary.

        .DESCRIPTION
        Inside TigerSetup's own guest staging root rather than the lab's, and
        named for the row, so that one row's announcement can never be read as
        another's. The engine creates the directory itself.
    #>
    param([Parameter(Mandatory)] [string] $Row)

    Join-Path $GuestStageRoot "boundary-$Row.signal"
}

function Test-LabRunUsable {
    <#
        Whether a lab run produced evidence to read. BUSY is the one that
        matters: a second consumer holds the lease, nothing ran, and every
        reader after this point would be interpreting an absence. The lab's
        contract calls that an outcome to act on, so the row stops here with
        the flattened checks it already has rather than failing later on a
        property that was never going to be there.
    #>
    param([object] $LabRun)
    $null -ne $LabRun -and $LabRun.status -eq 'OK'
}

function Invoke-EngineReadStep {
    param(
        [string] $Row,
        [string] $Suffix = 'read',
        [string] $EngineGuestPath,
        [string[]] $Logs = @()
    )

    $request = @{
        commands = @(
            @{ name = 'verify'; executable = $EngineGuestPath; arguments = @('verify', '--json') + $commonSetupArguments; timeoutSeconds = 300 },
            @{ name = 'inspect'; executable = $EngineGuestPath; arguments = @('inspect', '--json') + $commonSetupArguments; timeoutSeconds = 300 }
        )
        logs = @($Logs)
        inventory = @($InstallRoot, $StateDirectory)
    }
    Write-Host "  engine read: $EngineGuestPath"
    Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $request -Name "ts-$Suffix" `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-$Suffix.json") -OutputRoot $labOutputRoot -TimeoutMinutes $JobTimeoutMinutes
}

function Invoke-PrepareInstalledA {
    <# Restores the baseline, stages installer A outside any workspace and installs it. #>
    param([string] $Row)

    $installLog = Join-Path $GuestStageRoot 'install-a.log'
    $request = @{
        stage = @(@{ source = $installerFileA; destination = $stagedA })
        commands = @(
            @{ name = 'install'; executable = $stagedA; arguments = @('install', '--quiet', '--json', '--log', $installLog) + $commonSetupArguments; timeoutSeconds = 600 },
            @{ name = 'verify'; executable = $stagedA; arguments = @('verify', '--json') + $commonSetupArguments; timeoutSeconds = 300 },
            @{ name = 'inspect'; executable = $stagedA; arguments = @('inspect', '--json') + $commonSetupArguments; timeoutSeconds = 300 }
        )
        logs = @($installLog)
        inventory = @($InstallRoot, $StateDirectory)
    }
    Write-Host "  prepare: from the baseline, stage and install $versionA"
    $policy = Get-TigerSetupRowStepPolicy -FromBaseline
    Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $request -PayloadFiles @($InstallerPath) -Name 'ts-prepare' @policy `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-prepare.json") -OutputRoot $labOutputRoot -TimeoutMinutes $JobTimeoutMinutes
}

function Add-PrepareChecks {
    param([System.Collections.Generic.List[object]] $Checks, [object] $PrepareRun)

    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'prepare' -LabRun $PrepareRun) { $Checks.Add($check) }
    $install = Get-TigerSetupCommandResult -JobRun $PrepareRun -CommandName 'install'
    $outcome = [string] (Get-Member2 (Get-Member2 $install 'json') 'outcome')
    $Checks.Add((New-TigerSetupCheck -Name "prepare/install $versionA" -Code 'prepare.install' `
                -Status $(if ($null -ne $install -and $install.exitCode -eq 0 -and $outcome -eq 'installed') { 'PASS' } else { 'FAIL' }) `
                -Message "Silent install of $versionA exited $(Get-Member2 $install 'exitCode') with outcome '$outcome'."))
    $null = Add-EngineReadChecks -Checks $Checks -Prefix 'prepare' -JobRun $PrepareRun -ExpectedVersions @($versionA)
}

# ---------------------------------------------------------------------------
# Rows
# ---------------------------------------------------------------------------

function Invoke-InstallInterruptionRow {
    param(
        [string] $Row,
        [string] $Fault,
        [string] $Method,
        [string[]] $RequiredRecoveryCodes,
        [string[]] $DamageEvidenceCodes = @()
    )

    $checks = [System.Collections.Generic.List[object]]::new()
    $signal = Get-BoundarySignalPath -Row $Row
    $recovery = Invoke-RecoveryStep -Row $Row -SpecName $Row -InstallerHostPath $InstallerPath `
        -InstallerArguments (@('install', '--quiet') + $commonSetupArguments + @('--fault', $Fault, '--fault-signal', $signal)) -Method $Method `
        -BoundarySignalPath $signal `
        -RecoveryArguments (@('install', '--quiet') + $commonSetupArguments) -ExpectInstallRootExists $true -ExpectMinimumFileCount $MinimumFileCount
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'recovery' -LabRun $recovery) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $recovery)) {
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json")
    }

    $engine = Join-Path $LabRecoveryStageRoot $installerFileA
    $recoveryLog = Join-Path $LabRecoveryStageRoot 'recovery-run.log'
    $interruptedLog = Join-Path $LabRecoveryStageRoot 'interrupted-install.log'
    $read = Invoke-EngineReadStep -Row $Row -EngineGuestPath $engine -Logs @($recoveryLog, $interruptedLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'read' -LabRun $read) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $read)) {
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json")
    }
    $engineState = Add-EngineReadChecks -Checks $checks -Prefix 'read' -JobRun $read -ExpectedVersions @($versionA)

    $recoveryLines = Get-JobLog -JobRun $read -Path $recoveryLog
    $interruptedLines = Get-JobLog -JobRun $read -Path $interruptedLog
    Add-LogCodeChecks -Checks $checks -Prefix 'read' -Lines $recoveryLines -Required (@('recovery_started', 'recovery_completed') + $RequiredRecoveryCodes) -LogName 'recovery-run.log'
    # Two independent records that the run really was at the boundary when it
    # was interrupted, and the weaker one is the log: a power cut takes the
    # unflushed tail of the very log that would have named the fault. What
    # recovery found afterwards is the durable evidence — damage of exactly the
    # kind that boundary produces could not have happened anywhere else.
    $damaged = $DamageEvidenceCodes.Count -gt 0 -and
        @($DamageEvidenceCodes | Where-Object { Test-LogHasCode $recoveryLines $_ }).Count -gt 0
    $interruptedHasFault = Test-LogHasCode $interruptedLines 'fault_injected'
    $checks.Add((New-TigerSetupCheck -Name 'read/interrupted run reached the fault' -Code 'read.fault.reached' `
                -Status $(if ($interruptedHasFault -or $damaged) { 'PASS' } else { 'WARN' }) `
                -Message $(if ($interruptedHasFault) {
                        "The interrupted run's log records [fault_injected]."
                    }
                    elseif ($damaged) {
                        "The interrupted run's log lost its unflushed tail to the power cut, but recovery found the damage only that boundary produces ($($DamageEvidenceCodes -join ' or '))."
                    }
                    else {
                        "The interrupted run's log does not record [fault_injected] (after a power-off the log itself may have lost its unflushed tail)."
                    })))
    if ($DamageEvidenceCodes.Count -gt 0) {
        $checks.Add((New-TigerSetupCheck -Name 'read/interruption damaged something' -Code 'read.damage.observed' `
                    -Status $(if ($damaged) { 'PASS' } else { 'WARN' }) `
                    -Message $(if ($damaged) { "Recovery found and repaired damage ($($DamageEvidenceCodes -join ' or '))." } else { "Recovery found nothing to repair: the interruption landed outside the window, so this run proved nothing about $($DamageEvidenceCodes -join '/')." })))
    }

    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") `
        -Environment (Get-Member2 $recovery.result 'environment') `
        -Evidence @{ recoveryLog = $recoveryLines; interruptedLog = $interruptedLines; engine = $engineState; results = @($recovery.resultPath, $read.resultPath) }
}

function Invoke-UninstallRow {
    param([string] $Row)

    $checks = [System.Collections.Generic.List[object]]::new()
    $prepare = Invoke-PrepareInstalledA -Row $Row
    Add-PrepareChecks -Checks $checks -PrepareRun $prepare

    $log1 = Join-Path $GuestStageRoot 'uninstall-1.log'
    $log2 = Join-Path $GuestStageRoot 'uninstall-2.log'
    $request = @{
        commands = @(
            @{ name = 'uninstall'; executable = $stagedA; arguments = @('uninstall', '--quiet', '--json', '--log', $log1) + $commonSetupArguments; timeoutSeconds = 600 },
            @{ name = 'verify'; executable = $stagedA; arguments = @('verify', '--json') + $commonSetupArguments; timeoutSeconds = 300 },
            @{ name = 'inspect'; executable = $stagedA; arguments = @('inspect', '--json') + $commonSetupArguments; timeoutSeconds = 300 },
            @{ name = 'uninstall-again'; executable = $stagedA; arguments = @('uninstall', '--quiet', '--json', '--log', $log2) + $commonSetupArguments; timeoutSeconds = 600 }
        )
        logs = @($log1, $log2)
        inventory = @($InstallRoot, $StateDirectory)
    }
    Write-Host "  uninstall twice"
    $run = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $request -Name 'ts-uninstall' `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-uninstall.json") -OutputRoot $labOutputRoot -TimeoutMinutes $JobTimeoutMinutes
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'uninstall' -LabRun $run) { $checks.Add($check) }

    $first = Get-TigerSetupCommandResult -JobRun $run -CommandName 'uninstall'
    $second = Get-TigerSetupCommandResult -JobRun $run -CommandName 'uninstall-again'
    $firstOutcome = [string] (Get-Member2 (Get-Member2 $first 'json') 'outcome')
    $secondOutcome = [string] (Get-Member2 (Get-Member2 $second 'json') 'outcome')
    $checks.Add((New-TigerSetupCheck -Name 'uninstall/first run removes' -Code 'uninstall.first' `
                -Status $(if ($null -ne $first -and $first.exitCode -eq 0 -and $firstOutcome -eq 'uninstalled') { 'PASS' } else { 'FAIL' }) `
                -Message "First uninstall exited $(Get-Member2 $first 'exitCode') with outcome '$firstOutcome'."))
    $checks.Add((New-TigerSetupCheck -Name 'uninstall/second run reports not installed' -Code 'uninstall.second' `
                -Status $(if ($null -ne $second -and $second.exitCode -eq 0 -and $secondOutcome -eq 'not_installed') { 'PASS' } else { 'FAIL' }) `
                -Message "Second uninstall exited $(Get-Member2 $second 'exitCode') with outcome '$secondOutcome'."))
    $root = Get-JobInventory -JobRun $run -Path $InstallRoot
    $rootExists = $null -ne $root -and [bool] $root.exists
    $checks.Add((New-TigerSetupCheck -Name 'uninstall/install root gone' -Code 'uninstall.root.absent' `
                -Status $(if (-not $rootExists) { 'PASS' } else { 'FAIL' }) `
                -Message $(if ($rootExists) { "The install root still holds $($root.fileCount) file(s)." } else { 'The install root is gone.' })))
    $null = Add-EngineReadChecks -Checks $checks -Prefix 'uninstall' -JobRun $run -ExpectedVersions @()

    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") `
        -Environment (Get-Member2 $run.result 'environment') `
        -Evidence @{ uninstallLog = (Get-JobLog -JobRun $run -Path $log1); secondLog = (Get-JobLog -JobRun $run -Path $log2); results = @($prepare.resultPath, $run.resultPath) }
}

function Invoke-UpgradeInterruptionRow {
    param(
        [string] $Row,
        [string] $Fault,
        [string] $Method,
        # What the recovery run's log must record. An interruption after the
        # commit leaves no open transaction, so that row expects the cleanup
        # sweep rather than a recovery.
        [string[]] $RequiredRecoveryCodes = @('recovery_started', 'recovery_completed')
    )

    $checks = [System.Collections.Generic.List[object]]::new()
    $prepare = Invoke-PrepareInstalledA -Row $Row
    Add-PrepareChecks -Checks $checks -PrepareRun $prepare
    if (@($checks | Where-Object status -eq 'FAIL').Count -gt 0) {
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") -Environment (Get-Member2 $prepare.result 'environment')
    }

    $signal = Get-BoundarySignalPath -Row $Row
    $recovery = Invoke-RecoveryStep -Row $Row -SpecName $Row -InstallerHostPath $UpgradeInstallerPath `
        -InstallerArguments (@('install', '--quiet') + $commonSetupArguments + @('--fault', $Fault, '--fault-signal', $signal)) -Method $Method `
        -BoundarySignalPath $signal `
        -RecoveryArguments (@('install', '--quiet') + $commonSetupArguments) -ExpectInstallRootExists $true
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'recovery' -LabRun $recovery) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $recovery)) {
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json")
    }

    $engine = Join-Path $LabRecoveryStageRoot $installerFileB
    $recoveryLog = Join-Path $LabRecoveryStageRoot 'recovery-run.log'
    $interruptedLog = Join-Path $LabRecoveryStageRoot 'interrupted-install.log'
    $read = Invoke-EngineReadStep -Row $Row -EngineGuestPath $engine -Logs @($recoveryLog, $interruptedLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'read' -LabRun $read) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $read)) {
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json")
    }
    $engineState = Add-EngineReadChecks -Checks $checks -Prefix 'read' -JobRun $read -ExpectedVersions @($versionA, $versionB)
    $recoveryLines = Get-JobLog -JobRun $read -Path $recoveryLog
    Add-LogCodeChecks -Checks $checks -Prefix 'read' -Lines $recoveryLines -Required $RequiredRecoveryCodes -LogName 'recovery-run.log'

    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") `
        -Environment (Get-Member2 $recovery.result 'environment') `
        -Evidence @{ recoveryLog = $recoveryLines; interruptedLog = (Get-JobLog -JobRun $read -Path $interruptedLog); engine = $engineState; results = @($prepare.resultPath, $recovery.resultPath, $read.resultPath) }
}

function Invoke-UpgradeRollbackRow {
    param([string] $Row)

    $checks = [System.Collections.Generic.List[object]]::new()
    $prepare = Invoke-PrepareInstalledA -Row $Row
    Add-PrepareChecks -Checks $checks -PrepareRun $prepare
    if (@($checks | Where-Object status -eq 'FAIL').Count -gt 0) {
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") -Environment (Get-Member2 $prepare.result 'environment')
    }

    # The upgrade is held open before its commit and killed; the OLD version's
    # installer then uninstalls, which must roll the upgrade back to a complete
    # A before removing A.
    $rollbackLog = Join-Path $GuestStageRoot 'rollback-uninstall.log'
    $signal = Get-BoundarySignalPath -Row $Row
    $recovery = Invoke-RecoveryStep -Row $Row -SpecName $Row -InstallerHostPath $UpgradeInstallerPath `
        -InstallerArguments (@('install', '--quiet') + $commonSetupArguments + @('--fault', "before_commit:hold:$HoldSeconds", '--fault-signal', $signal)) -Method 'process' `
        -BoundarySignalPath $signal `
        -RecoveryCommand 'path' -RecoveryPath $stagedA -RecoveryArguments (@('uninstall', '--quiet', '--log', $rollbackLog) + $commonSetupArguments) `
        -ExpectInstallRootExists $false
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'recovery' -LabRun $recovery) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $recovery)) {
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json")
    }

    $read = Invoke-EngineReadStep -Row $Row -EngineGuestPath $stagedA -Logs @($rollbackLog, (Join-Path $LabRecoveryStageRoot 'interrupted-install.log'))
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'read' -LabRun $read) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $read)) {
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json")
    }
    $engineState = Add-EngineReadChecks -Checks $checks -Prefix 'read' -JobRun $read -ExpectedVersions @()
    $rollbackLines = Get-JobLog -JobRun $read -Path $rollbackLog
    Add-LogCodeChecks -Checks $checks -Prefix 'read' -Lines $rollbackLines -Required @('recovery_started', 'transaction_rolled_back', 'recovery_completed') -LogName 'rollback-uninstall.log'
    $root = Get-JobInventory -JobRun $read -Path $InstallRoot
    $rootExists = $null -ne $root -and [bool] $root.exists
    $checks.Add((New-TigerSetupCheck -Name 'read/install root gone' -Code 'read.root.absent' `
                -Status $(if (-not $rootExists) { 'PASS' } else { 'FAIL' }) `
                -Message $(if ($rootExists) { "The install root still holds $($root.fileCount) file(s)." } else { 'The install root is gone.' })))

    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") `
        -Environment (Get-Member2 $recovery.result 'environment') `
        -Evidence @{ rollbackLog = $rollbackLines; engine = $engineState; results = @($prepare.resultPath, $recovery.resultPath, $read.resultPath) }
}

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-recovery-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}
$null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId `
    -Description "TigerSetup recovery rows: $($Rows -join ', ')" `
    -ResultPath (Join-Path $ResultsRoot 'session-open.json')
Write-Host "Lab session $SessionId"

$summary = [System.Collections.Generic.List[object]]::new()
try {
foreach ($row in $Rows) {
    Write-Host ""
    Write-Host "### $row"
    $started = [DateTimeOffset]::Now
    try {
        $result = switch ($row) {
            'install-process-kill' {
                Invoke-InstallInterruptionRow -Row $row -Fault "after_prepare@${FaultSequence}:hold:$HoldSeconds" -Method 'process' -RequiredRecoveryCodes @('operation_reapplied')
            }
            'install-poweroff-unflushed' {
                Invoke-InstallInterruptionRow -Row $row -Fault "after_write_before_flush@${FaultSequence}:hold:$HoldSeconds" -Method 'powerOff' -RequiredRecoveryCodes @('operation_reapplied') -DamageEvidenceCodes @('file_missing', 'file_content_mismatch', 'temp_file_swept')
            }
            'install-poweroff-skipflush' {
                # Whether recovery repairs anything here depends on whether the
                # cut lands before Windows wrote the unflushed data on its own;
                # the row records that as WARN, never as FAIL.
                Invoke-InstallInterruptionRow -Row $row -Fault "after_rename@${FaultSequence}:hold:${HoldSeconds}:skip_flush" -Method 'powerOff' -RequiredRecoveryCodes @() -DamageEvidenceCodes @('file_content_mismatch')
            }
            'install-poweroff-skipflush-small' {
                Invoke-InstallInterruptionRow -Row $row -Fault "after_rename@${SmallFileSequence}:hold:${HoldSeconds}:skip_flush" -Method 'powerOff' -RequiredRecoveryCodes @() -DamageEvidenceCodes @('file_content_mismatch')
            }
            'install-reboot-before-commit' {
                Invoke-InstallInterruptionRow -Row $row -Fault "before_commit:hold:$HoldSeconds" -Method 'reboot' -RequiredRecoveryCodes @('transaction_committed')
            }
            'uninstall' { Invoke-UninstallRow -Row $row }
            'upgrade-poweroff-prepared' { Invoke-UpgradeInterruptionRow -Row $row -Fault "after_prepare@${FaultSequence}:hold:$HoldSeconds" -Method 'powerOff' }
            'upgrade-poweroff-renamed' { Invoke-UpgradeInterruptionRow -Row $row -Fault "after_rename@${FaultSequence}:hold:$HoldSeconds" -Method 'powerOff' }
            'upgrade-poweroff-before-commit' { Invoke-UpgradeInterruptionRow -Row $row -Fault "before_commit:hold:$HoldSeconds" -Method 'powerOff' }
            'upgrade-poweroff-after-commit' { Invoke-UpgradeInterruptionRow -Row $row -Fault "after_commit_before_cleanup:hold:$HoldSeconds" -Method 'powerOff' -RequiredRecoveryCodes @('staging_swept', 'already_installed') }
            'upgrade-rollback-by-old-uninstaller' { Invoke-UpgradeRollbackRow -Row $row }
        }
        $summary.Add([pscustomobject]@{ row = $row; status = $result.status; pass = $result.counts.pass; warn = $result.counts.warn; fail = $result.counts.fail; minutes = [math]::Round(([DateTimeOffset]::Now - $started).TotalMinutes, 1) })
    }
    catch {
        Write-Host "   ERROR $($_.Exception.Message)"
        # Re-running a recovery row costs minutes of guest time, so the failing
        # position and the stack are recorded with the message rather than left
        # to a second run to discover — the same rule the matrix rows follow.
        $where = "$($_.InvocationInfo.ScriptName):$($_.InvocationInfo.ScriptLineNumber)"
        Write-Host "         at $where — $($_.InvocationInfo.Line.Trim())"
        $summary.Add([pscustomobject]@{
                row = $row; status = 'ERROR'; pass = 0; warn = 0; fail = 1
                minutes = [math]::Round(([DateTimeOffset]::Now - $started).TotalMinutes, 1)
                error = $_.Exception.Message; at = $where; statement = $_.InvocationInfo.Line.Trim()
                stack = @($_.ScriptStackTrace -split "`r?`n")
            })
    }
}

}
finally {
    # The session is the run's, so it ends with the run - on the failing paths
    # too, because the lab never expires one and a leaked session holds a VM
    # until somebody ends it by hand.
    $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId -ResultPath (Join-Path $ResultsRoot 'session-close.json')
}

Write-Host ""
Write-Host "Summary ($ResultsRoot)"
$summary | Format-Table -AutoSize | Out-String | Write-Host
[System.IO.File]::WriteAllText((Join-Path $ResultsRoot 'summary.json'), ($summary | ConvertTo-Json -Depth 4), [System.Text.UTF8Encoding]::new($false))
if (@($summary | Where-Object { $_.status -in @('FAIL', 'ERROR') }).Count -gt 0) { exit 1 }
exit 0
