<#
    .SYNOPSIS
    Proves the TigerSetup wizard's all-users/elevation behaviour on the lab's
    interactive desktop: the native shield on Next, a genuine UAC transition
    answered on the secure desktop, the elevated child doing the machine-scope
    install and handing its result back to the wizard that asked, a safe
    refusal, and per-user install without a prompt.

    .DESCRIPTION
    A TigerWinLab guest job entry script. The payload carries the installer and
    a request.json naming the mode to run and the wizard's window class and
    control identifiers. The job is started with -Desktop, -HostConsole and,
    where the elevated child has to be driven, -ElevatedDesktop; the lab has
    already established the interactive session and handed it over as
    $TigerWinLabDesktop / $TigerWinLabElevatedDesktop, and answers the elevation
    prompt from the host when asked. This script neither imports the lab's
    module nor copies its client.

    Modes:
      shield-refuse     the unelevated wizard: the shield is absent for "for me
                        only", present for "for all users", survives repaint and
                        hover, and is cleared again; pressing Next raises a UAC
                        prompt, the wizard stays responsive while it is up, the
                        prompt is refused on the secure desktop, and nothing is
                        installed.
      complete-uac      the whole real path: the unelevated wizard, "for all
                        users", the shield on Next, the genuine UAC prompt
                        approved through the lab, the elevated child with the
                        expected authority driven to completion, the parent
                        stepping aside and staying responsive, the machine-scope
                        state, and the child's outcome handed back through the
                        parent's exit code and document.
      complete-elevated the already-elevated wizard (through -ElevatedDesktop):
                        all users is driven to completion with no prompt and no
                        shield, and the machine-scope state is verified.
      complete-user     the unelevated wizard: "for me only" is driven to
                        completion, with no elevation prompt at any point, and
                        the user-scope state is verified.

    The wizard is driven through the desktop agent by the control identifiers it
    publishes as its UI Automation ids (ui::window's ID_*), which are stable and
    language-independent, so nothing here depends on the wording of a button.
    The elevated child is a high-integrity window, so it is driven by the lab's
    elevated agent: the standard user's agent may find it but, by UIPI, not
    click into it.

    The result is the standard phase/check contract written to
    $env:TIGERWINLAB_JOB_RESULT.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$inputRoot = $env:TIGERWINLAB_JOB_INPUT
$artifactRoot = $env:TIGERWINLAB_JOB_ARTIFACTS
$request = Get-Content -LiteralPath (Join-Path $inputRoot 'request.json') -Raw | ConvertFrom-Json

function Get-Member2 {
    param([object] $Object, [string] $Name, [object] $Default = $null)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $Default }
    $Object.$Name
}

$mode = [string] (Get-Member2 $request 'mode' 'shield-refuse')
$executable = [string] (Get-Member2 $request 'executable')
$windowClass = [string] (Get-Member2 $request 'windowClass' 'TigerSetupWizard')
$questionClass = [string] (Get-Member2 $request 'questionClass' 'TigerSetupQuestion')
$stageRoot = [string] (Get-Member2 $request 'stageRoot' 'C:\TigerSetupLab\elevation')
$shieldThreshold = [int] (Get-Member2 $request 'shieldPixelThreshold' 24)
# Control identifiers the wizard publishes as its UI Automation ids.
$ID_NEXT = '101'
$ID_SCOPE_USER = '111'
$ID_SCOPE_MACHINE = '112'
$ID_LICENSE_ACCEPT = '122'
$ID_FINISH_BODY = '170'
$ID_FINISH_LAUNCH = '171'
$ID_PROGRESS_BAR = '161'

$checks = [System.Collections.Generic.List[object]]::new()
function Add-Check {
    param([string] $Name, [string] $Code, [ValidateSet('PASS', 'WARN', 'FAIL')] [string] $Status, [string] $Message)
    $checks.Add([pscustomobject][ordered]@{ name = $Name; code = $Code; status = $Status; message = $Message })
    Write-Host ("  [{0}] {1}: {2}" -f $Status, $Name, $Message)
}

function Find-Control {
    param([object] $Session, [long] $Hwnd, [string] $AutomationId)
    $found = Invoke-DesktopCommand -Session $Session -Command 'ui-find' -Parameters @{ selector = @{ hwnd = $Hwnd; automationId = $AutomationId } }
    if ($found.found) { return $found.element }
    return $null
}

function Wait-Control {
    param([object] $Session, [long] $Hwnd, [string] $AutomationId, [int] $TimeoutSeconds = 20)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $control = Find-Control -Session $Session -Hwnd $Hwnd -AutomationId $AutomationId
        if ($null -ne $control -and $null -ne $control.bounds -and -not $control.offscreen) { return $control }
        Start-Sleep -Milliseconds 300
    }
    return $null
}

function Invoke-ControlClick {
    param([object] $Session, [long] $Hwnd, [string] $AutomationId, [int] $SettleMilliseconds = 600)
    $control = Wait-Control -Session $Session -Hwnd $Hwnd -AutomationId $AutomationId
    if ($null -eq $control) { throw "Control '$AutomationId' did not appear on window $Hwnd." }
    $null = Invoke-DesktopCommand -Session $Session -Command 'window' -Parameters @{ hwnd = $Hwnd; action = 'activate'; settleMilliseconds = 250 }
    $null = Invoke-DesktopCommand -Session $Session -Command 'mouse' -Parameters @{ action = 'click'; x = [int] $control.bounds.centerX; y = [int] $control.bounds.centerY; settleMilliseconds = $SettleMilliseconds }
    $control
}

function Move-PointerAway {
    param([object] $Session, [object] $Window)
    # Park the pointer on the window's header, well away from the Next button,
    # so no hover highlight contaminates a Next-button capture.
    $x = [int] ($Window.bounds.x + [math]::Min(40, [int]($Window.bounds.width / 4)))
    $y = [int] ($Window.bounds.y + 20)
    $null = Invoke-DesktopCommand -Session $Session -Command 'mouse' -Parameters @{ action = 'move'; x = $x; y = $y; settleMilliseconds = 300 }
}

function Get-NextRegionCapture {
    param([object] $Session, [object] $Window, [long] $Hwnd, [string] $Name)
    Move-PointerAway -Session $Session -Window $Window
    $next = Wait-Control -Session $Session -Hwnd $Hwnd -AutomationId $ID_NEXT
    if ($null -eq $next) { throw 'The Next button was not found for a shield capture.' }
    $capture = Save-DesktopCapture -Session $Session -Name $Name -Destination (Join-Path $artifactRoot 'shots') -NoCursor
    [pscustomobject]@{
        capture = $capture
        # The Next button's rectangle, expressed relative to the capture's own
        # origin so it names the same pixels whatever the screen offset.
        region = [pscustomobject]@{
            x = [int] ($next.bounds.x - $capture.x)
            y = [int] ($next.bounds.y - $capture.y)
            width = [int] $next.bounds.width
            height = [int] $next.bounds.height
        }
    }
}

function Get-RegionDifference {
    param([object] $Session, [object] $First, [object] $Second)
    # Compare over the union region of the two Next rectangles, so a one-pixel
    # layout shift between captures cannot hide the shield.
    $x = [Math]::Min($First.region.x, $Second.region.x)
    $y = [Math]::Min($First.region.y, $Second.region.y)
    $right = [Math]::Max($First.region.x + $First.region.width, $Second.region.x + $Second.region.width)
    $bottom = [Math]::Max($First.region.y + $First.region.height, $Second.region.y + $Second.region.height)
    $result = Invoke-DesktopCommand -Session $Session -Command 'compare-captures' -Parameters @{
        first = $First.capture.name
        second = $Second.capture.name
        x = $x; y = $y; width = ($right - $x); height = ($bottom - $y)
    }
    [int] $result.differentPixels
}

function Test-MachineState {
    param([string] $Scope)
    # Ask the installer itself what the machine holds, as the job account (an
    # administrator), through the language-independent JSON. This is product
    # evidence, not a screen reading.
    $exePath = Join-Path $stageRoot ([System.IO.Path]::GetFileName($executable))
    $raw = & $exePath inspect --scope $Scope --json 2>&1 | Out-String
    try { return $raw | ConvertFrom-Json } catch { return $null }
}

function Get-VisiblePage {
    param([object] $Session, [long] $Hwnd)
    # Only the current page's controls are visible; the rest are hidden. Probe
    # the later pages first so a transitional frame resolves to the newer page.
    foreach ($page in @(
            @{ id = $ID_FINISH_BODY; name = 'finish' },
            @{ id = $ID_PROGRESS_BAR; name = 'progress' },
            @{ id = '151'; name = 'ready' },
            @{ id = '200'; name = 'options' },
            @{ id = '133'; name = 'destination' },
            @{ id = $ID_LICENSE_ACCEPT; name = 'license' },
            @{ id = $ID_SCOPE_MACHINE; name = 'scope' })) {
        $control = Find-Control -Session $Session -Hwnd $Hwnd -AutomationId $page.id
        if ($null -ne $control -and $null -ne $control.bounds -and -not $control.offscreen) { return $page.name }
    }
    return 'unknown'
}

function Test-ProcessAlive {
    param([int] $ProcessId)
    $null -ne (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)
}

function Invoke-WizardPages {
    <#
        Drives one wizard window from wherever it is to its completion page,
        choosing $Scope when the scope page is shown, and reports what was
        observed on the way: whether the finish page was reached, whether the
        secure desktop was ever seen, and whether the window kept pumping
        messages while the transaction applied.
    #>
    param([object] $Session, [int] $ProcessId, [long] $Hwnd, [ValidateSet('user', 'machine')] [string] $Scope, [int] $PageTimeoutSeconds = 60)

    $reached = $false
    $sawSecureDesktop = $false
    $installResponsive = $true
    for ($step = 1; $step -le 16; $step++) {
        if (-not (Test-ProcessAlive -ProcessId $ProcessId)) { break }
        $page = Get-VisiblePage -Session $Session -Hwnd $Hwnd
        if ($page -eq 'finish') { $reached = $true; break }
        if ((Invoke-DesktopCommand -Session $Session -Command 'input-desktop').secureDesktop) { $sawSecureDesktop = $true }
        if ($page -eq 'progress') {
            $responsive = Invoke-DesktopCommand -Session $Session -Command 'window-responsive' -Parameters @{ hwnd = $Hwnd; samples = 8; probeTimeoutMilliseconds = 1000; intervalMilliseconds = 1000 }
            if (-not $responsive.responsive) { $installResponsive = $false }
            $deadline = [DateTime]::UtcNow.AddSeconds(180)
            while ([DateTime]::UtcNow -lt $deadline) {
                if (-not (Test-ProcessAlive -ProcessId $ProcessId)) { break }
                if ((Get-VisiblePage -Session $Session -Hwnd $Hwnd) -eq 'finish') { $reached = $true; break }
                Start-Sleep -Milliseconds 500
            }
            if ($reached) { break }
            continue
        }
        if ($page -eq 'scope') {
            $null = Invoke-ControlClick -Session $Session -Hwnd $Hwnd -AutomationId $(if ($Scope -eq 'machine') { $ID_SCOPE_MACHINE } else { $ID_SCOPE_USER })
        }
        elseif ($page -eq 'license') {
            $null = Invoke-ControlClick -Session $Session -Hwnd $Hwnd -AutomationId $ID_LICENSE_ACCEPT
        }
        $before = $page
        $null = Invoke-ControlClick -Session $Session -Hwnd $Hwnd -AutomationId $ID_NEXT -SettleMilliseconds 500
        $wait = [DateTime]::UtcNow.AddSeconds($PageTimeoutSeconds)
        while ([DateTime]::UtcNow -lt $wait) {
            if (-not (Test-ProcessAlive -ProcessId $ProcessId)) { break }
            if ((Get-VisiblePage -Session $Session -Hwnd $Hwnd) -ne $before) { break }
            Start-Sleep -Milliseconds 400
        }
    }
    [pscustomobject]@{ reached = $reached; sawSecureDesktop = $sawSecureDesktop; installResponsive = $installResponsive }
}

function Test-ScopeState {
    <#
        Product-owned verification of the resulting state. Machine scope is read
        by the job account (an administrator); user scope belongs to the
        signed-in account, so it is read in that session.
    #>
    param([object] $Session, [string] $ExePath, [ValidateSet('user', 'machine')] [string] $Scope, [string] $Prefix)

    $state = $null
    $verify = $null
    if ($Scope -eq 'machine') {
        $state = Test-MachineState -Scope 'machine'
        $verifyRaw = & $ExePath verify --scope machine --json 2>&1 | Out-String
        try { $verify = $verifyRaw | ConvertFrom-Json } catch { $verify = $null }
    }
    else {
        $inspectRun = Invoke-DesktopCommand -Session $Session -Command 'run' -Parameters @{ filePath = $ExePath; arguments = @('inspect', '--scope', 'user', '--json') }
        try { $state = (@($inspectRun.stdout) -join "`n") | ConvertFrom-Json } catch { $state = $null }
        $verifyRun = Invoke-DesktopCommand -Session $Session -Command 'run' -Parameters @{ filePath = $ExePath; arguments = @('verify', '--scope', 'user', '--json') }
        try { $verify = (@($verifyRun.stdout) -join "`n") | ConvertFrom-Json } catch { $verify = $null }
    }
    $installation = Get-Member2 $state 'installation'
    $installed = $null -ne $installation
    $scopeOk = $installed -and ([string] (Get-Member2 $installation 'scope') -eq $Scope)
    Add-Check "$Scope install state" "$Prefix.installed" $(if ($scopeOk) { 'PASS' } else { 'FAIL' }) `
        "inspect reports an installation in $Scope scope at '$(if ($installed) { Get-Member2 $installation 'install_root' } else { '<none>' })'."
    $verifyStatus = [string] (Get-Member2 $verify 'status')
    Add-Check "$Scope verify" "$Prefix.verify" $(if ($verifyStatus -eq 'installed' -or $verifyStatus -eq 'ok') { 'PASS' } elseif ([string]::IsNullOrWhiteSpace($verifyStatus)) { 'WARN' } else { 'FAIL' }) `
        "verify reports '$verifyStatus' for the $Scope-scope installation."
}

function Invoke-WizardToFinish {
    param([object] $Session, [string] $ExePath, [ValidateSet('user', 'machine')] [string] $Scope, [string] $Prefix)

    $started = Invoke-DesktopCommand -Session $Session -Command 'start-process' -Parameters @{ filePath = $ExePath; arguments = @('install'); workingDirectory = $stageRoot }
    $processId = [int] $started.processId
    $window = Invoke-DesktopCommand -Session $Session -Command 'wait-window' -Parameters @{ processId = $processId; classPattern = $windowClass; timeoutSeconds = 30 }
    $hwnd = [long] $window.hwnd

    $driven = Invoke-WizardPages -Session $Session -ProcessId $processId -Hwnd $hwnd -Scope $Scope
    Add-Check "$Scope wizard reached finish" "$Prefix.finish" $(if ($driven.reached) { 'PASS' } else { 'FAIL' }) `
        "The $Scope-scope wizard was driven to its completion page."
    if ($Scope -eq 'machine') {
        Add-Check 'no shield or prompt when elevated' "$Prefix.noprompt" $(if (-not $driven.sawSecureDesktop) { 'PASS' } else { 'FAIL' }) `
            "An already-elevated machine-scope install completed without raising a UAC prompt (secure desktop seen: $($driven.sawSecureDesktop))."
    }
    else {
        Add-Check 'no elevation for user scope' "$Prefix.noprompt" $(if (-not $driven.sawSecureDesktop) { 'PASS' } else { 'FAIL' }) `
            "The per-user install completed without ever raising a UAC prompt (secure desktop seen: $($driven.sawSecureDesktop))."
    }
    Add-Check "$Scope wizard responsive while installing" "$Prefix.responsive" $(if ($driven.installResponsive) { 'PASS' } else { 'FAIL' }) `
        "The wizard kept pumping messages while the transaction was applying."

    Test-ScopeState -Session $Session -ExePath $ExePath -Scope $Scope -Prefix $Prefix

    if (Test-ProcessAlive -ProcessId $processId) {
        try { $null = Invoke-DesktopCommand -Session $Session -Command 'stop-process' -Parameters @{ processId = $processId } } catch { }
    }
}

function Get-ParentWindowVisibility {
    param([object] $Session, [int] $ProcessId)
    $windows = @(Invoke-DesktopCommand -Session $Session -Command 'list-windows' -Parameters @{ processId = $ProcessId; includeInvisible = $true })
    $wizards = @($windows | Where-Object { [string] (Get-Member2 $_ 'className') -eq $windowClass })
    [pscustomobject]@{
        count = $wizards.Count
        visible = @($wizards | Where-Object { [bool] (Get-Member2 $_ 'visible' $true) }).Count
    }
}

function Invoke-RealElevationPath {
    <#
        The whole path the product has to support, measured end to end on one
        wizard: unelevated launch, all users, the genuine prompt approved on the
        secure desktop through the lab, the elevated child doing the install,
        and the child's outcome coming back through the parent that asked.
    #>
    param([object] $Standard, [object] $Elevated, [string] $ExePath)

    # The parent is started through the lab's tracked wrapper with its output
    # captured, so its exit code and the document it prints - the child's,
    # handed back - are evidence rather than inference.
    $null = Initialize-InteractiveRun -StageRoot (Join-Path $stageRoot 'stage') -RunRoot (Join-Path $stageRoot 'runs') -WrapperScript (Join-Path $env:TIGERWINLAB_DESKTOP_SUPPORT 'Start-TrackedProcess.ps1')
    $run = Start-TrackedRun -Session $Standard -Name 'parent' -Path $ExePath -Arguments @('install', '--json') -CaptureOutput -TimeoutSeconds 600
    $parentId = 0
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        $record = Get-TrackedRun -Run $run
        if ($null -ne $record -and [int] (Get-Member2 $record 'processId' 0) -gt 0) { $parentId = [int] $record.processId; break }
        Start-Sleep -Milliseconds 300
    }
    if ($parentId -le 0) { throw 'The parent wizard was not started by the tracked wrapper.' }
    $window = Invoke-DesktopCommand -Session $Standard -Command 'wait-window' -Parameters @{ processId = $parentId; classPattern = $windowClass; timeoutSeconds = 30 }
    $hwnd = [long] $window.hwnd
    $parentToken = Get-ProcessElevation -ProcessId $parentId
    Add-Check 'parent wizard unelevated' 'uac.parent_unelevated' $(if ($parentToken.elevated -eq $false) { 'PASS' } else { 'FAIL' }) `
        "The wizard started unelevated (pid $parentId, elevated: $($parentToken.elevated), integrity: $($parentToken.integrityLevel))."

    # 1. "for all users" puts the native shield on Next.
    $scopeMachine = Wait-Control -Session $Standard -Hwnd $hwnd -AutomationId $ID_SCOPE_MACHINE
    if ($null -eq $scopeMachine) { throw 'The scope page did not appear.' }
    $meOnly = Get-NextRegionCapture -Session $Standard -Window $window -Hwnd $hwnd -Name 'uac-next-me-only.png'
    $null = Invoke-ControlClick -Session $Standard -Hwnd $hwnd -AutomationId $ID_SCOPE_MACHINE
    $allUsers = Get-NextRegionCapture -Session $Standard -Window $window -Hwnd $hwnd -Name 'uac-next-all-users.png'
    $appear = Get-RegionDifference -Session $Standard -First $meOnly -Second $allUsers
    Add-Check 'shield on Next for all users' 'uac.shield' $(if ($appear -ge $shieldThreshold) { 'PASS' } else { 'FAIL' }) `
        "The Next button changed by $appear pixel(s) when 'for all users' was selected (threshold $shieldThreshold); the native shield is expected there."

    # 2. Next raises the genuine prompt; the lab answers it on the secure desktop.
    $since = [DateTime]::Now
    $null = Invoke-ControlClick -Session $Standard -Hwnd $hwnd -AutomationId $ID_NEXT -SettleMilliseconds 300
    $responsive = Invoke-DesktopCommand -Session $Standard -Command 'window-responsive' -Parameters @{ hwnd = $hwnd; samples = 4; probeTimeoutMilliseconds = 1000; intervalMilliseconds = 400 }
    $approval = Approve-ElevationPrompt -Since $since -Session $Standard -TimeoutSeconds 30 -CaptureName 'uac-prompt.png'
    [System.IO.File]::WriteAllText((Join-Path $artifactRoot 'uac-approval.json'), ($approval | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
    $prompted = $null -ne $approval.prompt
    Add-Check 'genuine UAC prompt raised' 'uac.prompted' $(if ($prompted -and $approval.secureDesktop -eq $true) { 'PASS' } elseif ($prompted) { 'WARN' } else { 'FAIL' }) `
        "Pressing Next with 'for all users' raised consent.exe $(if ($prompted) { $approval.prompt.processId } else { '<none>' }) and switched the input to the secure desktop ($($approval.secureDesktop))."
    Add-Check 'wizard responsive while the prompt is up' 'uac.responsive' $(if ($responsive.responsive) { 'PASS' } else { 'FAIL' }) `
        "The wizard kept pumping messages across the elevation transition (hung samples: $(@($responsive.samples | Where-Object { $_.hung }).Count) of $(@($responsive.samples).Count))."
    $answer = Get-Member2 $approval 'answer'
    Add-Check 'prompt approved on the secure desktop' 'uac.approved' $(if ($approval.performed) { 'PASS' } else { 'FAIL' }) `
        $(if ($approval.performed) { "The lab answered the prompt from the host ($(Get-Member2 $answer 'method') as $(Get-Member2 $answer 'account' '<the signed-in administrator>')); consent.exe left and the input desktop returned." } else { "The prompt was not answered: $($approval.reason)" })
    if (-not $approval.performed) {
        if (Test-ProcessAlive -ProcessId $parentId) { try { $null = Invoke-DesktopCommand -Session $Standard -Command 'stop-process' -Parameters @{ processId = $parentId } } catch { } }
        return
    }

    # 3. The elevated child is this installer, elevated, carrying the hand-back
    #    argument - and nothing else appeared elevated.
    $children = @($approval.elevatedProcesses | Where-Object {
            [string] (Get-Member2 $_ 'executablePath') -eq $ExePath -and [string] (Get-Member2 $_ 'commandLine') -like '*--elevated-result*'
        })
    $child = if ($children.Count -ge 1) { $children[0] } else { $null }
    Add-Check 'elevated child has the expected authority' 'uac.child' $(if ($null -ne $child -and $child.integrityLevel -eq 'high' -and $children.Count -eq 1) { 'PASS' } else { 'FAIL' }) `
        $(if ($null -ne $child) { "The elevated child is $($child.executablePath) (pid $($child.processId), $($child.owner), integrity $($child.integrityLevel)) started with '$($child.commandLine)'; $(@($approval.elevatedProcesses).Count) process(es) appeared elevated." } else { "No elevated child of the installer appeared; elevated after the prompt: $(@($approval.elevatedProcesses | ForEach-Object { $_.name }) -join ', ')." })
    if ($null -eq $child) {
        if (Test-ProcessAlive -ProcessId $parentId) { try { $null = Invoke-DesktopCommand -Session $Standard -Command 'stop-process' -Parameters @{ processId = $parentId } } catch { } }
        return
    }
    $childId = [int] $child.processId

    # 4. The child shows the wizard from its next page on, and the parent steps
    #    aside for it while staying alive and responsive.
    $childWindow = Invoke-DesktopCommand -Session $Elevated -Command 'wait-window' -Parameters @{ processId = $childId; classPattern = $windowClass; timeoutSeconds = 30 } -PassThruError
    Add-Check 'elevated child shows its wizard' 'uac.child_window' $(if ($childWindow.ok) { 'PASS' } else { 'FAIL' }) `
        $(if ($childWindow.ok) { "The elevated child put up a visible wizard window (hwnd $($childWindow.result.hwnd))." } else { "The elevated child showed no visible wizard within 30 s: $($childWindow.error)" })
    $parentVisibility = Get-ParentWindowVisibility -Session $Standard -ProcessId $parentId
    $parentResponsive = Invoke-DesktopCommand -Session $Standard -Command 'window-responsive' -Parameters @{ hwnd = $hwnd; samples = 3; probeTimeoutMilliseconds = 1000; intervalMilliseconds = 300 }
    Add-Check 'parent steps aside and stays responsive' 'uac.parent_aside' $(if ((Test-ProcessAlive -ProcessId $parentId) -and $parentResponsive.responsive -and $parentVisibility.visible -eq 0) { 'PASS' } elseif ((Test-ProcessAlive -ProcessId $parentId) -and $parentResponsive.responsive) { 'WARN' } else { 'FAIL' }) `
        "While the child runs, the parent is alive ($(Test-ProcessAlive -ProcessId $parentId)), responsive ($($parentResponsive.responsive)) and shows $($parentVisibility.visible) visible wizard window(s) of $($parentVisibility.count)."
    if (-not $childWindow.ok) {
        try { Stop-Process -Id $childId -Force -ErrorAction SilentlyContinue } catch { }
        return
    }
    $childHwnd = [long] $childWindow.result.hwnd
    $driven = Invoke-WizardPages -Session $Elevated -ProcessId $childId -Hwnd $childHwnd -Scope 'machine'
    Add-Check 'elevated child reached finish' 'uac.finish' $(if ($driven.reached) { 'PASS' } else { 'FAIL' }) `
        "The elevated child's wizard was driven to its completion page (secure desktop seen again: $($driven.sawSecureDesktop))."
    Add-Check 'elevated child responsive while installing' 'uac.child_responsive' $(if ($driven.installResponsive) { 'PASS' } else { 'FAIL' }) `
        'The elevated child kept pumping messages while the transaction was applying.'
    if ($driven.reached) {
        # Leave the launch offer alone and close the wizard; the child exits
        # and hands its outcome to the parent.
        $launch = Find-Control -Session $Elevated -Hwnd $childHwnd -AutomationId $ID_FINISH_LAUNCH
        if ($null -ne $launch -and [string] (Get-Member2 $launch 'toggleState') -eq 'On') {
            $null = Invoke-ControlClick -Session $Elevated -Hwnd $childHwnd -AutomationId $ID_FINISH_LAUNCH
        }
        $null = Invoke-ControlClick -Session $Elevated -Hwnd $childHwnd -AutomationId $ID_NEXT -SettleMilliseconds 500
    }

    # 5. The child exits; the parent reports the child's outcome as its own and
    #    exits with the child's code.
    $record = Wait-TrackedRun -Run $run -TimeoutSeconds 90
    $childGone = $false
    $gone = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $gone) { if (-not (Test-ProcessAlive -ProcessId $childId)) { $childGone = $true; break }; Start-Sleep -Milliseconds 300 }
    $exitCode = Get-Member2 $record 'exitCode'
    $document = $null
    try { $document = (@(Get-Member2 $record 'output' @()) -join "`n") | ConvertFrom-Json } catch { $document = $null }
    $outcome = [string] (Get-Member2 $document 'outcome')
    $reportedScope = [string] (Get-Member2 (Get-Member2 $document 'installation') 'scope')
    Add-Check 'child exits and parent completes' 'uac.parent_exit' $(if ($childGone -and $null -ne $exitCode -and [int] $exitCode -eq 0 -and -not (Test-ProcessAlive -ProcessId $parentId)) { 'PASS' } else { 'FAIL' }) `
        "The elevated child exited ($childGone) and the parent exited with code $(if ($null -ne $exitCode) { $exitCode } else { '<none>' }) (timed out: $(Get-Member2 $record 'timedOut' $false); error: $(Get-Member2 $record 'error' '<none>'))."
    Add-Check 'result handed back to the parent' 'uac.handback' $(if ($outcome -eq 'installed' -and $reportedScope -eq 'machine') { 'PASS' } else { 'FAIL' }) `
        "The parent printed the child's outcome document: outcome '$outcome', scope '$reportedScope' ($(@(Get-Member2 $record 'output' @()).Count) line(s) of output)."
    [System.IO.File]::WriteAllText((Join-Path $artifactRoot 'uac-parent-run.json'), ($record | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))

    # 6. The machine holds the installation.
    Test-ScopeState -Session $Standard -ExePath $ExePath -Scope 'machine' -Prefix 'uac'

    foreach ($processId in @($parentId, $childId)) {
        if (Test-ProcessAlive -ProcessId $processId) { try { Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue } catch { } }
    }
}

# --------------------------------------------------------------------------
# Session
# --------------------------------------------------------------------------
$standard = Get-Variable -Name 'TigerWinLabDesktop' -ValueOnly -ErrorAction SilentlyContinue
$elevated = Get-Variable -Name 'TigerWinLabElevatedDesktop' -ValueOnly -ErrorAction SilentlyContinue
$sessionError = $null
$info = $null

try {
    if ($null -eq $standard) { throw 'The job provided no interactive desktop session; it must be started with -Desktop.' }
    $info = Invoke-DesktopCommand -Session $standard -Command 'info'

    # Stage the installer where the interactive account can run it; the job
    # workspace is not readable by that account.
    $null = New-Item -ItemType Directory -Path $stageRoot -Force
    Copy-Item -LiteralPath (Join-Path $inputRoot ([System.IO.Path]::GetFileName($executable))) -Destination (Join-Path $stageRoot ([System.IO.Path]::GetFileName($executable))) -Force
    $null = & icacls.exe $stageRoot '/grant' 'Users:(OI)(CI)RX' '/T' '/Q' 2>&1
    $exeInStage = Join-Path $stageRoot ([System.IO.Path]::GetFileName($executable))

    switch ($mode) {

        'shield-refuse' {
            $session = $standard
            $started = Invoke-DesktopCommand -Session $session -Command 'start-process' -Parameters @{ filePath = $exeInStage; arguments = @('install'); workingDirectory = $stageRoot }
            $processId = [int] $started.processId
            $window = Invoke-DesktopCommand -Session $session -Command 'wait-window' -Parameters @{ processId = $processId; classPattern = $windowClass; timeoutSeconds = 30 }
            $hwnd = [long] $window.hwnd

            $scopeUser = Wait-Control -Session $session -Hwnd $hwnd -AutomationId $ID_SCOPE_USER
            $scopeMachine = Wait-Control -Session $session -Hwnd $hwnd -AutomationId $ID_SCOPE_MACHINE
            Add-Check 'scope page shown' 'scope.page' $(if ($null -ne $scopeUser -and $null -ne $scopeMachine) { 'PASS' } else { 'FAIL' }) `
                "The scope page offers both 'for me only' and 'for all users'."

            # 1. "for me only" (the default): no shield on Next.
            $meOnly = Get-NextRegionCapture -Session $session -Window $window -Hwnd $hwnd -Name 'next-me-only.png'

            # 2. "for all users": the shield appears on Next.
            $null = Invoke-ControlClick -Session $session -Hwnd $hwnd -AutomationId $ID_SCOPE_MACHINE
            $allUsers = Get-NextRegionCapture -Session $session -Window $window -Hwnd $hwnd -Name 'next-all-users.png'
            $appear = Get-RegionDifference -Session $session -First $meOnly -Second $allUsers
            Add-Check 'shield appears for all users' 'shield.appears' $(if ($appear -ge $shieldThreshold) { 'PASS' } else { 'FAIL' }) `
                "The Next button changed by $appear pixel(s) when 'for all users' was selected (threshold $shieldThreshold); the native shield is expected there."

            # 2b. It survives hover and repaint.
            $next = Wait-Control -Session $session -Hwnd $hwnd -AutomationId $ID_NEXT
            $null = Invoke-DesktopCommand -Session $session -Command 'mouse' -Parameters @{ action = 'move'; x = [int] $next.bounds.centerX; y = [int] $next.bounds.centerY; settleMilliseconds = 300 }
            $null = Invoke-DesktopCommand -Session $session -Command 'window' -Parameters @{ hwnd = $hwnd; action = 'activate'; settleMilliseconds = 300 }
            $afterRepaint = Get-NextRegionCapture -Session $session -Window $window -Hwnd $hwnd -Name 'next-all-users-repaint.png'
            $persists = Get-RegionDifference -Session $session -First $meOnly -Second $afterRepaint
            $drift = Get-RegionDifference -Session $session -First $allUsers -Second $afterRepaint
            Add-Check 'shield survives hover and repaint' 'shield.persists' $(if ($persists -ge $shieldThreshold -and $drift -lt $shieldThreshold) { 'PASS' } else { 'FAIL' }) `
                "After a hover and repaint the shield still differs from 'for me only' by $persists pixel(s) and is stable against its first capture ($drift pixel(s))."

            # 2c. Selecting "for me only" again clears it.
            $null = Invoke-ControlClick -Session $session -Hwnd $hwnd -AutomationId $ID_SCOPE_USER
            $meOnlyAgain = Get-NextRegionCapture -Session $session -Window $window -Hwnd $hwnd -Name 'next-me-only-again.png'
            $cleared = Get-RegionDifference -Session $session -First $meOnly -Second $meOnlyAgain
            $removed = Get-RegionDifference -Session $session -First $allUsers -Second $meOnlyAgain
            Add-Check 'shield cleared for me only' 'shield.cleared' $(if ($cleared -lt $shieldThreshold -and $removed -ge $shieldThreshold) { 'PASS' } else { 'FAIL' }) `
                "Re-selecting 'for me only' returned Next to its no-shield state (differs from the first no-shield capture by $cleared pixel(s); differs from the shielded one by $removed)."

            # 3. Press Next with "for all users": elevation is requested and the
            #    wizard stays responsive while the prompt is up.
            $null = Invoke-ControlClick -Session $session -Hwnd $hwnd -AutomationId $ID_SCOPE_MACHINE
            $since = [DateTime]::Now
            $null = Invoke-ControlClick -Session $session -Hwnd $hwnd -AutomationId $ID_NEXT -SettleMilliseconds 400
            $prompt = Wait-ElevationPrompt -Since $since -Session $session -TimeoutSeconds 20
            Add-Check 'elevation requested on Next' 'elevation.requested' $(if ($prompt.prompted) { 'PASS' } else { 'FAIL' }) `
                "Pressing Next with 'for all users' raised a UAC prompt (consent.exe: $(if ($prompt.prompted) { $prompt.prompt.processId } else { '<none>' }); secure desktop: $($prompt.secureDesktop))."

            $responsive = Invoke-DesktopCommand -Session $session -Command 'window-responsive' -Parameters @{ hwnd = $hwnd; samples = 6; probeTimeoutMilliseconds = 1000; intervalMilliseconds = 500 }
            Add-Check 'wizard responsive during elevation' 'elevation.responsive' $(if ($responsive.responsive) { 'PASS' } else { 'FAIL' }) `
                "The wizard kept pumping messages across the whole elevation transition ($(@($responsive.samples).Count) samples over ~3s; hung: $(@($responsive.samples | Where-Object { $_.hung }).Count))."

            # 4. Refuse the prompt on the secure desktop, as a person's No.
            $refusal = Deny-ElevationPrompt -Since $since -Session $session -TimeoutSeconds 20 -CaptureName 'uac-prompt-refused.png'
            [System.IO.File]::WriteAllText((Join-Path $artifactRoot 'uac-refusal.json'), ($refusal | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
            Add-Check 'UAC prompt refused' 'elevation.refused' $(if ($refusal.performed) { 'PASS' } else { 'FAIL' }) `
                $(if ($refusal.performed) { 'The prompt was refused on the secure desktop through the lab; consent.exe left and the input desktop returned, so ShellExecuteEx reports the cancellation.' } else { "The prompt was not refused: $($refusal.reason)" })

            # The wizard shows a modal error on refusal; dismiss it so the page
            # is usable again.
            $dialog = Invoke-DesktopCommand -Session $session -Command 'wait-window' -Parameters @{ processId = $processId; classPattern = $questionClass; timeoutSeconds = 20 } -PassThruError
            $dismissed = $false
            if ($dialog.ok -and $null -ne $dialog.result) {
                $ok = Get-DialogControl -Session $session -Hwnd ([long] $dialog.result.hwnd) | Where-Object { $_.enabled -and $_.controlType -eq 'button' } | Select-Object -First 1
                if ($null -ne $ok) {
                    $null = Invoke-DesktopCommand -Session $session -Command 'mouse' -Parameters @{ action = 'click'; x = [int] $ok.bounds.centerX; y = [int] $ok.bounds.centerY; settleMilliseconds = 500 }
                    $dismissed = $true
                }
            }

            # 5. The wizard is usable and responsive again, and nothing was
            #    installed in machine scope.
            $afterResponsive = Invoke-DesktopCommand -Session $session -Command 'window-responsive' -Parameters @{ hwnd = $hwnd; samples = 3; probeTimeoutMilliseconds = 1000; intervalMilliseconds = 300 }
            $backOnScope = Wait-Control -Session $session -Hwnd $hwnd -AutomationId $ID_SCOPE_MACHINE -TimeoutSeconds 10
            $alive = Test-ProcessAlive -ProcessId $processId
            Add-Check 'wizard usable after refusal' 'refusal.usable' $(if ($afterResponsive.responsive -and $alive -and $null -ne $backOnScope) { 'PASS' } else { 'FAIL' }) `
                "After refusal the wizard is still running ($alive), responsive ($($afterResponsive.responsive)), and back on the scope page ($($null -ne $backOnScope)); error dialog dismissed: $dismissed."

            $state = Test-MachineState -Scope 'machine'
            $installed = $null -ne $state -and $null -ne (Get-Member2 $state 'installation')
            Add-Check 'no machine install after refusal' 'refusal.no_install' $(if (-not $installed) { 'PASS' } else { 'FAIL' }) `
                "A refused elevation left no machine-scope installation (installed: $installed)."

            # Close the wizard.
            $null = Invoke-DesktopCommand -Session $session -Command 'window' -Parameters @{ hwnd = $hwnd; action = 'close'; settleMilliseconds = 500 } -PassThruError
            Start-Sleep -Seconds 2
            if (Test-ProcessAlive -ProcessId $processId) {
                try { $null = Invoke-DesktopCommand -Session $session -Command 'stop-process' -Parameters @{ processId = $processId } } catch { }
            }
        }

        'complete-uac' {
            if ($null -eq $elevated) { throw 'complete-uac needs the elevated desktop session to drive the elevated child; it must be started with -ElevatedDesktop.' }
            Invoke-RealElevationPath -Standard $standard -Elevated $elevated -ExePath $exeInStage
        }

        'complete-elevated' {
            if ($null -eq $elevated) { throw 'complete-elevated needs an elevated desktop session; it must be started with -ElevatedDesktop.' }
            Invoke-WizardToFinish -Session $elevated -ExePath $exeInStage -Scope 'machine' -Prefix 'machine'
        }

        'complete-user' {
            Invoke-WizardToFinish -Session $standard -ExePath $exeInStage -Scope 'user' -Prefix 'user'
        }

        default { throw "Unknown mode '$mode'." }
    }
}
catch {
    $sessionError = ('{0} (line {1}: {2})' -f $_.Exception.Message, $_.InvocationInfo.ScriptLineNumber, $_.InvocationInfo.Line.Trim())
    Write-Host "acceptance failed: $sessionError"
    Add-Check 'acceptance run' 'run.error' 'FAIL' $sessionError
}
finally {
    if ($null -ne $standard) { try { Copy-DesktopAgentLog -Session $standard -Destination $artifactRoot } catch { } }
    if ($null -ne $elevated) { try { Copy-DesktopAgentLog -Session $elevated -Destination $artifactRoot } catch { } }
}

$phaseStatus = if (@($checks | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0) { 'FAIL' } elseif (@($checks | Where-Object { $_.status -eq 'WARN' }).Count -gt 0) { 'WARN' } else { 'PASS' }
$result = [pscustomobject][ordered]@{
    mode = $mode
    session = $(if ($null -ne $info) { [pscustomobject]@{ userName = $info.userName; dpi = $info.dpi; scalePercent = $info.scalePercent } } else { $null })
    error = $sessionError
    phases = @([pscustomobject]@{ name = $mode; status = $phaseStatus; checks = @($checks) })
}
$json = $result | ConvertTo-Json -Depth 12
[System.IO.File]::WriteAllText($env:TIGERWINLAB_JOB_RESULT, $json, [System.Text.UTF8Encoding]::new($false))
[System.IO.File]::WriteAllText((Join-Path $artifactRoot 'elevation-acceptance.json'), $json, [System.Text.UTF8Encoding]::new($false))
if ($null -ne $sessionError -or $phaseStatus -eq 'FAIL') { exit 1 }
exit 0
