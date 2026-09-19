<#
    .SYNOPSIS
    Focused TigerWinLab acceptance for the all-users/elevation path of the
    TigerSetup wizard: the native shield on Next, a genuine UAC transition
    answered on the secure desktop, the elevated child completing the
    machine-scope install and handing its result back to the wizard that asked,
    a safe refusal, and per-user install without a prompt.

    .DESCRIPTION
    Five rows, each its own guest job on one lab session:

      shield-refuse      the unelevated wizard — the shield is absent for "for me
                         only", present for "for all users", survives repaint and
                         hover and is cleared again; pressing Next raises a UAC
                         prompt, the wizard stays responsive while it is up, the
                         prompt is refused on the secure desktop, and nothing is
                         installed.
      complete-uac       the whole real path on the standard user's desktop —
                         unelevated wizard, all users, shield, the genuine
                         credential prompt approved through the lab, the elevated
                         child with the expected authority driven to completion,
                         the parent stepping aside and staying responsive, the
                         machine-scope state, and the child's outcome handed back
                         through the parent's exit code and document.
      complete-uac-admin the same path on an administrator's desktop, where the
                         prompt is the Yes/No consent prompt — the case a
                         developer installing on their own machine meets.
      complete-elevated  the already-elevated wizard (the lab's credential-backed
                         session) — all users is driven to completion with no
                         prompt and no shield, and machine-scope state is verified.
      complete-user      the unelevated wizard — "for me only" is driven to
                         completion with no elevation at any point, and user-scope
                         state is verified.

    The prompt is answered by the lab's host console: the VM's virtual keyboard
    types on the secure desktop exactly as a person at the console would, so
    UAC and its secure desktop stay as Windows ships them, and the lab checks
    that the prompt it answers was raised by this operation and that the
    elevated process which appears is this installer with the hand-back
    argument. Testing the pre-consent and post-elevation halves separately does
    not prove the handoff; complete-uac and complete-uac-admin are one wizard
    end to end for that reason, and they run against the installer under
    release — the self-hosted TigerSetup installer — because that is where the
    handoff failed.

    .EXAMPLE
    pwsh -File lab\Invoke-ElevationRows.ps1 -InstallerPath artifacts\tigersetup\TigerSetup-0.7.0-Setup.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $InstallerPath,
    # Not a [ValidateSet]: `pwsh -File` passes a comma-joined list as one
    # string, which the attribute would reject before it is split. Each value
    # is checked below instead.
    [string[]] $Rows = @('shield-refuse', 'complete-uac', 'complete-uac-admin', 'complete-elevated', 'complete-user'),
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $Language = 'en-US',
    [int] $ScalePercent = 100,
    [string] $BuilderPath,
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    [string] $SessionId,
    [int] $JobTimeoutMinutes = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Rows = @($Rows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$known = @('shield-refuse', 'complete-uac', 'complete-uac-admin', 'complete-elevated', 'complete-user')
$unknown = @($Rows | Where-Object { $_ -notin $known })
if ($unknown.Count -gt 0) { throw "Unknown row(s): $($unknown -join ', '). Known rows: $($known -join ', ')." }

$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\elevation-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'

# The rows measure the engine the installer carries, so refuse a stale one up
# front rather than spending guest time to report on the wrong engine.
if ([string]::IsNullOrWhiteSpace($BuilderPath)) {
    $candidate = Join-Path $PSScriptRoot '..\target\x86_64-pc-windows-msvc\release\tiger-setup.exe'
    if (Test-Path -LiteralPath $candidate -PathType Leaf) { $BuilderPath = (Resolve-Path -LiteralPath $candidate).Path }
}
if (-not [string]::IsNullOrWhiteSpace($BuilderPath) -and (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) {
    $facts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $InstallerPath
    Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -Facts $facts -InstallerPath $InstallerPath
    Write-Host "Installer $([System.IO.Path]::GetFileName($InstallerPath)) carries the current engine."
}
else {
    Write-Warning 'No builder found beside the installer; skipping the engine-currency check. Pass -BuilderPath to enable it.'
}

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-elevation-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}
$null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId `
    -Description "TigerSetup elevation acceptance: $($Rows -join ', ')" `
    -ResultPath (Join-Path $ResultsRoot 'session-open.json')
Write-Host "Lab session $SessionId"

$summary = [System.Collections.Generic.List[object]]::new()
try {
    foreach ($mode in $Rows) {
        Write-Host ""
        Write-Host "### $mode"
        $checks = [System.Collections.Generic.List[object]]::new()
        # Every mode starts from the baseline; the run's session hands the VM back at the end.
        $policy = Get-TigerSetupRowStepPolicy -FromBaseline
        $run = Invoke-TigerSetupElevationDrive -LabRoot $labRoot -Baseline $Baseline -Mode $mode `
            -ExecutablePath $InstallerPath -Language $Language -ScalePercent $ScalePercent `
            @policy -Name "ts-elev-$mode" -ResultPath (Join-Path $ResultsRoot "runs\$mode.json") `
            -OutputRoot $labOutputRoot -TimeoutMinutes $JobTimeoutMinutes
        foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'elevation' -LabRun $run) { $checks.Add($check) }

        $record = if ($null -ne $run.result) { $run.result.PSObject.Properties['result'] } else { $null }
        $result = Write-TigerSetupRowResult -Row $mode -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$mode.json") `
            -Environment $(if ($null -ne $run.result) { $run.result.environment } else { $null }) `
            -Evidence @{ labResult = $run.resultPath }
        $summary.Add([pscustomobject]@{ row = $mode; status = $result.status; pass = $result.counts.pass; warn = $result.counts.warn; fail = $result.counts.fail })
    }
}
finally {
    $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId -ResultPath (Join-Path $ResultsRoot 'session-close.json')
}

Write-Host ""
Write-Host "Summary ($ResultsRoot)"
$summary | Format-Table -AutoSize | Out-String | Write-Host
[System.IO.File]::WriteAllText((Join-Path $ResultsRoot 'summary.json'), ($summary | ConvertTo-Json -Depth 4), [System.Text.UTF8Encoding]::new($false))
if (@($summary | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0) { exit 1 }
exit 0
