<#
    .SYNOPSIS
    Puts named runtimes in place on a TigerWinLab guest before a matrix row runs.

    .DESCRIPTION
    A TigerWinLab guest job entry script. The request names, per dependency, a
    download URL, the unattended arguments and the exit codes that mean
    success, plus the detector the row will later rely on (a directory or a
    registry value), so the job reports whether the runtime was present before
    and is present afterwards. This is the lab's "prepared" dependency state
    (`TigerSetup-Validation.md` §5.2): the runtime installed by its vendor's
    own installer, the way a machine that already has it would look.

    Nothing here is TigerSetup: the product's own acquisition path is what the
    rows measure, and a row that wants it measured starts from a clean guest.
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

function Test-DependencyPresent {
    param([object] $Detect)
    if ($null -eq $Detect) { return $null }
    $kind = [string] (Get-Member2 $Detect 'kind')
    switch ($kind) {
        'directory' {
            $path = [Environment]::ExpandEnvironmentVariables([string] $Detect.path)
            $pattern = [string] (Get-Member2 $Detect 'pattern')
            if (-not (Test-Path -LiteralPath $path)) { return [pscustomobject]@{ present = $false; version = $null } }
            $dirs = @(Get-ChildItem -LiteralPath $path -Directory -ErrorAction SilentlyContinue | Where-Object { [string]::IsNullOrWhiteSpace($pattern) -or $_.Name -like $pattern })
            if ($dirs.Count -eq 0) { return [pscustomobject]@{ present = $false; version = $null } }
            return [pscustomobject]@{ present = $true; version = ($dirs | Sort-Object Name | Select-Object -Last 1).Name }
        }
        'registry' {
            foreach ($key in @($Detect.keys)) {
                $provider = ([string] $key) -replace '^HKLM\\', 'HKLM:\' -replace '^HKCU\\', 'HKCU:\'
                if (Test-Path -LiteralPath $provider) {
                    $value = [string] (Get-ItemProperty -LiteralPath $provider -ErrorAction SilentlyContinue).($Detect.value)
                    if (-not [string]::IsNullOrWhiteSpace($value) -and $value -ne '0.0.0.0') {
                        return [pscustomobject]@{ present = $true; version = $value }
                    }
                }
            }
            return [pscustomobject]@{ present = $false; version = $null }
        }
        default { return $null }
    }
}

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$results = [System.Collections.Generic.List[object]]::new()
$failed = $false
foreach ($dependency in @(Get-Member2 $request 'dependencies')) {
    if ($null -eq $dependency) { continue }
    $id = [string] $dependency.id
    $before = Test-DependencyPresent (Get-Member2 $dependency 'detect')
    $record = [ordered]@{ id = $id; before = $before; downloaded = $null; exitCode = $null; durationSeconds = $null; after = $null; error = $null }
    if ($null -ne $before -and [bool] $before.present -and -not [bool] (Get-Member2 $dependency 'force')) {
        $record.after = $before
        $results.Add([pscustomobject] $record)
        Write-Host "[$id] already present ($($before.version)); nothing to do."
        continue
    }
    try {
        $target = Join-Path $env:TIGERWINLAB_JOB_WORKSPACE ([string] $dependency.file)
        $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        Invoke-WebRequest -Uri ([string] $dependency.url) -OutFile $target -UseBasicParsing
        $record.downloaded = (Get-Item -LiteralPath $target).Length
        $arguments = @(@(Get-Member2 $dependency 'arguments') | ForEach-Object { [string] $_ })
        $process = Start-Process -FilePath $target -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
        $stopwatch.Stop()
        $record.exitCode = $process.ExitCode
        $record.durationSeconds = [math]::Round($stopwatch.Elapsed.TotalSeconds, 1)
        $success = @(Get-Member2 $dependency 'successExitCodes')
        if ($success.Count -eq 0) { $success = @(0, 3010) }
        if ($success -notcontains $process.ExitCode) {
            $record.error = "The installer exited with $($process.ExitCode); expected $($success -join ' or ')."
            $failed = $true
        }
    }
    catch {
        $record.error = $_.Exception.Message
        $failed = $true
    }
    $record.after = Test-DependencyPresent (Get-Member2 $dependency 'detect')
    if ($null -ne $record.after -and -not [bool] $record.after.present -and $null -eq $record.error) {
        $record.error = 'The installer succeeded but the detector still reports the runtime absent.'
        $failed = $true
    }
    $results.Add([pscustomobject] $record)
    Write-Host "[$id] exit $($record.exitCode); present afterwards: $(if ($null -ne $record.after) { $record.after.present } else { 'unknown' })"
}

$result = [pscustomobject][ordered]@{
    collectedAt = [DateTimeOffset]::Now.ToString('o')
    dependencies = @($results)
}
$json = $result | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText($env:TIGERWINLAB_JOB_RESULT, $json, [System.Text.UTF8Encoding]::new($false))
[System.IO.File]::WriteAllText((Join-Path $artifactRoot 'prepare-dependencies.json'), $json, [System.Text.UTF8Encoding]::new($false))
if ($failed) { exit 1 }
exit 0
