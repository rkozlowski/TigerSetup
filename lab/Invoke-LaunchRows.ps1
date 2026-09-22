<#
    .SYNOPSIS
    Focused TigerWinLab acceptance for launch after install
    (TigerSetup-Design.md 11.7): the program the completion page offers is
    started as the signed-in user and never elevated, with exactly the declared
    arguments, in the declared directory, and in the foreground — through
    every path a wizard can be elevated by — and a cleared offer or a desktop
    with no non-elevated context starts nothing.

    .DESCRIPTION
    Six rows, each its own guest job from the baseline on one lab session,
    against the TigerSetupTestLaunch package (packages/test-launch), whose
    program reports how it was started:

      launch-user        the unelevated wizard, "for me only": Finish starts the
                         program with the wizard's own token.
      launch-unchecked   the same, with the offer cleared by the person: nothing
                         is started, and the run's log says it was declined.
      launch-uac         the standard user's desktop, "for all users", the
                         genuine credential prompt answered with the lab
                         administrator's password: the elevated child runs as
                         another account, so it has the desktop's shell start
                         the program — which runs as the standard user,
                         unelevated.
      launch-uac-admin   the same on an administrator's desktop, where the
                         prompt is Yes/No and the child is the same account
                         elevated: the program still runs with the filtered,
                         unelevated token.
      launch-elevated    an installer started elevated (the lab's
                         credential-backed session over the standard user's
                         desktop): the program runs as the standard user,
                         unelevated.
      launch-no-shell    the same elevated installer with the desktop's shell
                         ended before Finish: there is no non-elevated context,
                         so nothing is started, the person is told, and the
                         installation stands — never a fallback to the
                         wizard's own elevated token.

    Every started program is checked for: unelevated token (not elevated, no
    enabled Administrators group, integrity below high), the desktop's own
    account, exactly the declared arguments after `--` (with %VERSION%
    expanded), the declared working directory under the real install root,
    its parent (the wizard itself, or explorer.exe for an elevated wizard),
    its window in the foreground as the program itself observed it, and the
    run's log line. The expectations are read from the installer through the
    builder's own inspection, never typed here.

    .EXAMPLE
    pwsh -File lab\Invoke-LaunchRows.ps1 -InstallerPath artifacts\test-launch\TigerSetupTestLaunch-1.0.0-Setup.exe
    pwsh -File lab\Invoke-LaunchRows.ps1 -InstallerPath artifacts\test-launch\TigerSetupTestLaunch-1.0.0-Setup.exe -Rows launch-uac,launch-elevated
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $InstallerPath,
    # Not a [ValidateSet]: `pwsh -File` passes a comma-joined list as one
    # string, which the attribute would reject before it is split. Each value
    # is checked below instead.
    [string[]] $Rows = @('launch-user', 'launch-unchecked', 'launch-uac', 'launch-uac-admin', 'launch-elevated', 'launch-no-shell'),
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $Language = 'en-US',
    [string] $BuilderPath,
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    [string] $SessionId,
    [int] $JobTimeoutMinutes = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

# Each row: the elevation mode that reaches the completion page, and what the
# launch offer gets there.
$rowTable = [ordered]@{
    'launch-user' = @{ mode = 'complete-user'; action = 'launch' }
    'launch-unchecked' = @{ mode = 'complete-user'; action = 'decline' }
    'launch-uac' = @{ mode = 'complete-uac'; action = 'launch' }
    'launch-uac-admin' = @{ mode = 'complete-uac-admin'; action = 'launch' }
    'launch-elevated' = @{ mode = 'complete-elevated'; action = 'launch' }
    'launch-no-shell' = @{ mode = 'complete-elevated'; action = 'no-shell' }
}

# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Rows = @($Rows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$unknown = @($Rows | Where-Object { -not $rowTable.Contains($_) })
if ($unknown.Count -gt 0) { throw "Unknown row(s): $($unknown -join ', '). Known rows: $($rowTable.Keys -join ', ')." }

$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\launch-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'

# The expectations come from the package itself, and the rows measure the
# engine the installer carries, so the builder is required here.
if ([string]::IsNullOrWhiteSpace($BuilderPath)) {
    $BuilderPath = Join-Path $PSScriptRoot '..\target\x86_64-pc-windows-msvc\release\tiger-setup.exe'
}
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) { throw "No builder at '$BuilderPath'; build the release binaries or pass -BuilderPath." }
$BuilderPath = (Resolve-Path -LiteralPath $BuilderPath).Path
$facts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $InstallerPath
Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -Facts $facts -InstallerPath $InstallerPath
Write-Host "Installer $([System.IO.Path]::GetFileName($InstallerPath)) carries the current engine."

$declared = $facts.raw.PSObject.Properties['launch']
if ($null -eq $declared -or $null -eq $declared.Value) { throw "'$InstallerPath' declares no [launch]; build packages\test-launch." }
$declared = $declared.Value
$arguments = @($declared.arguments | ForEach-Object { [string] $_ })
$separator = [Array]::IndexOf($arguments, '--')
if ($separator -lt 0) { throw 'The package''s launch arguments carry no `--`, so there is nothing to compare exactly.' }
$reportIndex = [Array]::IndexOf($arguments, '--report')
if ($reportIndex -lt 0) { throw 'The package''s launch arguments name no --report.' }
# The guest expands nothing: what the program must receive is spelled out
# here, from the declaration, the way the engine expands it.
$launch = @{
    productId = $facts.id
    report = $arguments[$reportIndex + 1].Replace('%PROGRAMDATA%', 'C:\ProgramData')
    arguments = @($arguments | Select-Object -Skip ($separator + 1) | ForEach-Object { $_.Replace('%VERSION%', $facts.version) })
    workingDirectory = [string] $declared.working_directory
}
Write-Host "Launch offer: $($declared.executable), $($launch.arguments.Count) argument(s) after --, in '$($launch.workingDirectory)'."

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-launch-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}
$null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId `
    -Description "TigerSetup launch-after-install acceptance: $($Rows -join ', ')" `
    -ResultPath (Join-Path $ResultsRoot 'session-open.json')
Write-Host "Lab session $SessionId"

$summary = [System.Collections.Generic.List[object]]::new()
try {
    foreach ($row in $Rows) {
        Write-Host ""
        Write-Host "### $row"
        $spec = $rowTable[$row]
        $checks = [System.Collections.Generic.List[object]]::new()
        # Every row starts from the baseline; the run's session hands the VM back at the end.
        $policy = Get-TigerSetupRowStepPolicy -FromBaseline
        $run = Invoke-TigerSetupElevationDrive -LabRoot $labRoot -Baseline $Baseline -Mode $spec.mode `
            -ExecutablePath $InstallerPath -Language $Language -LaunchAction $spec.action -Launch $launch `
            @policy -Name "ts-$row" -ResultPath (Join-Path $ResultsRoot "runs\$row.json") `
            -OutputRoot $labOutputRoot -TimeoutMinutes $JobTimeoutMinutes
        foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'launch' -LabRun $run) { $checks.Add($check) }

        $result = Write-TigerSetupRowResult -Row $row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$row.json") `
            -Environment $(if ($null -ne $run.result) { $run.result.environment } else { $null }) `
            -Evidence @{ labResult = $run.resultPath }
        $summary.Add([pscustomobject]@{ row = $row; status = $result.status; pass = $result.counts.pass; warn = $result.counts.warn; fail = $result.counts.fail })
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
