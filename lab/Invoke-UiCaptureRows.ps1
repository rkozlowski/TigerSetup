<#
    .SYNOPSIS
    Captures wizard executables on the lab's interactive desktop across the
    language × scale × theme combinations TigerSetup's UI validation covers.

    .DESCRIPTION
    One guest job per combination; every executable named is captured in that
    job, page by page, with a UI Automation tree per page. The lab selects the
    signed-in account for the language, the session scale and the Windows
    light or dark theme, and reports what it measured, so a capture that was
    not taken at the requested scale or theme fails its row rather than passing
    quietly. Screenshots are copied out of the lab artifacts into
    <ResultsRoot>\<combination>\<captureName>-page-N.png.

    .EXAMPLE
    pwsh -File lab\Invoke-UiCaptureRows.ps1 -ExecutablePath artifacts\TigerMarkView\TigerMarkView-0.8.2-Setup.exe `
        -TitlePattern TigerMarkView -Combinations en-US:100:light, en-US:200:dark, pl-PL:150:dark
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string[]] $ExecutablePath,
    [string[]] $Arguments = @(),
    [string] $TitlePattern = '.',
    [int] $MaxPages = 8,
    [string[]] $AdvanceKeys = @('Return'),
    # Per-page key sequences: "<page>=<chord>;<chord>" with keys of a chord joined
    # by '+', several pages separated by spaces, e.g. "2=Menu+A;Return" for a
    # licence page that needs Alt+A before Enter.
    [string] $AdvanceByPage = '',
    # An argument added per combination with {lang} replaced by its language,
    # e.g. '--lang {lang}'; empty leaves the executable to detect the language.
    [string] $LanguageArgumentTemplate = '',
    # <language>:<scale>[:<theme>]. Naming no theme leaves the guest on
    # whatever it was, which is light on a clean baseline; naming one makes the
    # run assert it. Windows 11 UI acceptance covers both themes and at least
    # one non-default scale.
    [string[]] $Combinations = @(
        'en-US:100:light', 'en-US:100:dark',
        'en-US:125:light', 'en-US:150:dark', 'en-US:200:dark',
        'pl-PL:100:light', 'pl-PL:150:dark'),
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    # The lab session every operation of this run belongs to. One session means
    # the VM boots once for the whole run and is released for the lab's own
    # cleanup when the run ends, instead of being left powered on or shut down
    # under somebody else's work. Naming one joins an existing session.
    [string] $SessionId,
    # Each combination starts from the baseline. The capture presses the
    # advance keys on every page it captures, including the last, so a run that
    # reaches the wizard's ready page starts a real installation — and the next
    # combination would then be photographing an upgrade wizard instead of the
    # install one it asked for.
    [switch] $SkipReset,
    [int] $JobTimeoutMinutes = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$ExecutablePath = @($ExecutablePath | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$Combinations = @($Combinations | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\ui-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'

$advanceChords = @{}
foreach ($entry in ($AdvanceByPage -split '\s+' | Where-Object { $_ })) {
    $separator = $entry.IndexOf('=')
    if ($separator -lt 1) { throw "AdvanceByPage entry '$entry' must be <page>=<chord>;<chord>." }
    $page = $entry.Substring(0, $separator)
    $sequence = $entry.Substring($separator + 1)
    $advanceChords[$page] = @($sequence -split ';' | Where-Object { $_ } | ForEach-Object { , @($_ -split '\+') })
}

$wizards = foreach ($path in $ExecutablePath) {
    $resolved = (Resolve-Path -LiteralPath $path).Path
    @{
        ExecutablePath = $resolved
        Arguments = @($Arguments)
        CaptureName = ([System.IO.Path]::GetFileNameWithoutExtension($resolved) -replace '[^A-Za-z0-9._-]', '-')
        TitlePattern = $TitlePattern
        MaxPages = $MaxPages
        AdvanceKeys = @($AdvanceKeys)
        AdvanceByPage = $advanceChords
    }
}

function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-ui-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}
$null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId `
    -Description "TigerSetup UI captures: $($Combinations -join ', ')" `
    -ResultPath (Join-Path $ResultsRoot 'session-open.json')
Write-Host "Lab session $SessionId"

$summary = [System.Collections.Generic.List[object]]::new()
try {
foreach ($combination in $Combinations) {
    $parts = $combination.Split(':')
    if ($parts.Count -lt 2) { throw "Combination '$combination' must be <language>:<scale>[:<theme>]." }
    $language = $parts[0]
    $scale = [int] $parts[1]
    $theme = if ($parts.Count -ge 3) { $parts[2] } else { '' }
    if ($theme -notin @('', 'light', 'dark')) { throw "Combination '$combination' names theme '$theme'; expected light or dark." }
    $row = if ($theme) { "$language-$scale-$theme" } else { "$language-$scale" }
    Write-Host ""
    Write-Host "### $row"
    $checks = [System.Collections.Generic.List[object]]::new()
    $rowWizards = foreach ($wizard in $wizards) {
        $copy = $wizard.Clone()
        if (-not [string]::IsNullOrWhiteSpace($LanguageArgumentTemplate)) {
            $copy.Arguments = @($wizard.Arguments) + @(($LanguageArgumentTemplate.Replace('{lang}', $language)) -split '\s+' | Where-Object { $_ })
        }
        $copy
    }
    $run = Invoke-TigerSetupWizardCapture -LabRoot $labRoot -Baseline $Baseline -Wizards $rowWizards -Language $language -ScalePercent $scale -Theme $theme `
        -Reset:(-not $SkipReset) `
        -Name "ts-ui-$row" -ResultPath (Join-Path $ResultsRoot "runs\$row.json") -OutputRoot $labOutputRoot -TimeoutMinutes $JobTimeoutMinutes
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'capture' -LabRun $run) { $checks.Add($check) }

    # What the lab found on its desktop before this row's wizard started. The lab
    # clears its own transient shell state there and refuses the row when it
    # cannot, so this is evidence that the row was given a desktop worth
    # photographing rather than a check of TigerSetup's own behaviour.
    $jobId = [string] (Get-Member2 $run.result 'jobId')
    $jobArtifacts = if ($jobId) { Join-Path $labOutputRoot $jobId } else { $null }
    $workspacePath = if ($jobArtifacts) { Join-Path $jobArtifacts 'desktop-workspace.json' } else { $null }
    $workspace = $null
    if ($workspacePath -and (Test-Path -LiteralPath $workspacePath -PathType Leaf)) {
        try { $workspace = Get-Content -LiteralPath $workspacePath -Encoding UTF8 -Raw | ConvertFrom-Json } catch { $workspace = $null }
    }
    # A record from an older lab carries neither list, and Get-Member2 answers
    # $null for both; an empty list is what that means here.
    $clearSteps = @(Get-Member2 $workspace 'steps' | Where-Object { $null -ne $_ })
    $foundOnDesktop = @(Get-Member2 $workspace 'found' | Where-Object { $null -ne $_ })
    $checks.Add((New-TigerSetupCheck -Name 'capture/desktop precondition' -Code 'capture.desktop.precondition' `
                -Status $(if ($null -ne $workspace -and (Get-Member2 $workspace 'clear')) { 'PASS' } elseif ($null -eq $workspace) { 'WARN' } else { 'FAIL' }) `
                -Message $(if ($null -eq $workspace) {
                        'The lab reported no desktop workspace record for this row.'
                    }
                    elseif (Get-Member2 $workspace 'clear') {
                        "The lab handed over an unobstructed desktop$(if ($foundOnDesktop.Count -gt 0) { ' after clearing ' + (($foundOnDesktop | ForEach-Object { "$($_.className)/$($_.processName)" }) -join '; ') + ' with ' + (($clearSteps | ForEach-Object { $_.method }) -join ', ') } else { '' })."
                    }
                    else {
                        "The lab could not clear the desktop before this row: $(((Get-Member2 $workspace 'obstructions' | Where-Object { $null -ne $_ }) | ForEach-Object { "$($_.className)/$($_.processName)" }) -join '; ')."
                    })))

    $record = Get-Member2 $run.result 'result'
    $session = Get-Member2 $record 'session'
    $measuredScale = [int] (Get-Member2 $session 'scalePercent')
    $checks.Add((New-TigerSetupCheck -Name 'capture/session scale' -Code 'capture.session.scale' `
                -Status $(if ($measuredScale -eq $scale) { 'PASS' } else { 'FAIL' }) `
                -Message "The desktop agent measured $measuredScale % ($(Get-Member2 $session 'dpi') DPI) as $(Get-Member2 $session 'userName'); $scale % was requested."))

    # The theme is the lab's own measurement of the interactive session, not
    # what the run asked for, so a capture taken in the wrong theme cannot pass
    # as evidence for it.
    if ($theme) {
        $interactive = Get-Member2 (Get-Member2 $run.result 'environment') 'interactive'
        # The lab reports the interactive theme as a record — the app and system
        # preferences it set, and what it observed of the session's actual
        # background colour. The app preference is the one an application reads,
        # so it is the one a wizard following the theme has to match.
        $reported = Get-Member2 $interactive 'theme'
        $measuredTheme = [string] (Get-Member2 $reported 'apps')
        if ([string]::IsNullOrWhiteSpace($measuredTheme)) { $measuredTheme = [string] (Get-Member2 $reported 'observation') }
        if ([string]::IsNullOrWhiteSpace($measuredTheme) -and $reported -is [string]) { $measuredTheme = [string] $reported }
        $checks.Add((New-TigerSetupCheck -Name 'capture/session theme' -Code 'capture.session.theme' `
                    -Status $(if ($measuredTheme -eq $theme) { 'PASS' } elseif ([string]::IsNullOrWhiteSpace($measuredTheme)) { 'WARN' } else { 'FAIL' }) `
                    -Message $(if ([string]::IsNullOrWhiteSpace($measuredTheme)) { "The lab reported no interactive theme; $theme was requested." } else { "The session's theme is '$measuredTheme'; $theme was requested." })))
    }

    $destination = Join-Path $ResultsRoot $row
    $null = New-Item -ItemType Directory -Path $destination -Force
    # Every combination names its captures the same way, so copy only from this
    # row's own job directory, never from the whole artifact tree.
    $copied = 0
    foreach ($wizard in @(Get-Member2 $record 'wizards')) {
        $pages = @(Get-Member2 $wizard 'pages')
        $checks.Add((New-TigerSetupCheck -Name "capture/$($wizard.captureName) pages" -Code "capture.$($wizard.captureName).pages" `
                    -Status $(if ($pages.Count -gt 0 -and $null -eq (Get-Member2 $wizard 'error')) { 'PASS' } else { 'FAIL' }) `
                    -Message "$($pages.Count) page(s) captured; process $(Get-Member2 $wizard 'processEnd'). $(Get-Member2 $wizard 'error')".Trim()))
        # A capture is of the screen as composited, so anything left open over
        # the wizard is in the picture while every other check still passes.
        # These captures exist to be looked at, so an obscured one is no
        # evidence for the pages it covers and the row says so.
        $obscured = @($pages | Where-Object { -not [string]::IsNullOrWhiteSpace([string] (Get-Member2 $_ 'obscuredBy')) })
        $checks.Add((New-TigerSetupCheck -Name "capture/$($wizard.captureName) unobstructed" -Code "capture.$($wizard.captureName).unobstructed" `
                    -Status $(if ($obscured.Count -eq 0) { 'PASS' } else { 'FAIL' }) `
                    -Message $(if ($obscured.Count -eq 0) {
                            "The wizard owned the middle of every one of the $($pages.Count) capture(s)."
                        }
                        else {
                            "$($obscured.Count) of $($pages.Count) capture(s) have something else over the wizard ($((($obscured | ForEach-Object { 'page ' + $_.page + ': ' + $_.obscuredBy }) -join '; '))), so they are not evidence about it."
                        })))
        if ($null -eq $jobArtifacts -or -not (Test-Path -LiteralPath $jobArtifacts)) { continue }
        foreach ($page in $pages) {
            $source = Join-Path (Join-Path $jobArtifacts 'screenshots') $page.capture
            if (Test-Path -LiteralPath $source -PathType Leaf) { Copy-Item -LiteralPath $source -Destination (Join-Path $destination $page.capture) -Force; $copied++ }
        }
        foreach ($tree in Get-ChildItem -LiteralPath $jobArtifacts -Filter "$($wizard.captureName)-page-*-uia.json" -File -ErrorAction SilentlyContinue) {
            Copy-Item -LiteralPath $tree.FullName -Destination (Join-Path $destination $tree.Name) -Force
        }
    }
    $checks.Add((New-TigerSetupCheck -Name 'capture/screenshots copied' -Code 'capture.screenshots.copied' `
                -Status $(if ($copied -gt 0) { 'PASS' } else { 'FAIL' }) -Message "$copied screenshot(s) copied from job '$jobId'."))
    Write-Host "  $copied screenshot(s) copied to $destination"

    $result = Write-TigerSetupRowResult -Row $row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$row.json") `
        -Environment (Get-Member2 $run.result 'environment') -Evidence @{ capture = $record; screenshots = $destination }
    $summary.Add([pscustomobject]@{ row = $row; status = $result.status; pass = $result.counts.pass; warn = $result.counts.warn; fail = $result.counts.fail; screenshots = $copied })
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
if (@($summary | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0) { exit 1 }
exit 0
