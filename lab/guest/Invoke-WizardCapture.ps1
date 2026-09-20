<#
    .SYNOPSIS
    Drives wizard executables page by page on the lab's interactive desktop and
    captures every page as evidence.

    .DESCRIPTION
    A TigerWinLab guest job entry script. The payload carries the executables
    and a request.json listing the wizards to capture: each names its
    executable, arguments, the window title pattern to wait for, the keys that
    advance a page and how many pages to expect.

    The job is started with -Desktop, so the lab has already established the
    interactive session — the one whose display language, scale and theme the
    run asked for — and handed it over as $TigerWinLabDesktop. This script
    neither imports the lab's module nor carries a copy of its desktop client:
    a payload that staged its own copy of the lab's guest scripts would be
    running last week's lab inside this week's one.

    For each wizard it launches the executable in that session and, per page,
    activates the window, captures it, dumps its UI Automation tree, presses the
    advance keys, and then waits for the wizard to actually leave that page. A
    wizard ends when its process exits or its page limit is reached; a page that
    never changes ends it too, with what it was still showing. The result
    reports the session's measured DPI and scale, every capture, and how each
    process ended. Nothing here judges the pages.
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

$stageRoot = [string] (Get-Member2 $request 'stageRoot' 'C:\TigerSetupLab\ui')
$wizards = @(Get-Member2 $request 'wizards' @())

function Get-WizardPageSignature {
    <#
        Identifies the page a wizard is showing by the controls it shows.

        A wizard keeps one window with one title, so nothing about the window
        says which page is in front, and the text on a page changes while the
        page does not - a progress page rewrites its own label for every file it
        writes. The set of control identifiers is what stays put for exactly as
        long as the page does.
    #>
    param([object] $Session, [long] $Hwnd)

    $tree = Invoke-DesktopCommand -Session $Session -Command 'ui-tree' -Parameters @{ hwnd = $Hwnd; maxDepth = 2; maxNodes = 100 }
    (@($tree.root.children | ForEach-Object { [string] $_.automationId } | Sort-Object) -join ',')
}

function Resolve-WindowOwnerProcessId {
    <#
        The process whose windows the capture photographs. A generated
        TigerSetup Setup.exe is a loader that starts the engine as its child
        and never shows a window of its own, so the wizard is the child's; a
        program that puts up its own window is its own owner. Whichever
        appears first within the timeout wins.
    #>
    param([object] $Session, [int] $ProcessId, [string] $TitlePattern, [int] $TimeoutSeconds = 20)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $children = @(Get-CimInstance -ClassName Win32_Process -Filter "ParentProcessId = $ProcessId" -ErrorAction SilentlyContinue | Sort-Object -Property CreationDate)
        if ($children.Count -gt 0) { return [int] $children[0].ProcessId }
        try {
            $null = Invoke-DesktopCommand -Session $Session -Command 'wait-window' -Parameters @{ processId = $ProcessId; titlePattern = $TitlePattern; timeoutSeconds = 1 }
            return $ProcessId
        }
        catch { }
        if ($null -eq (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) { break }
    }
    return $ProcessId
}

function Invoke-WizardCapture {
    param([object] $Session, [object] $Wizard)

    $captureName = [string] (Get-Member2 $Wizard 'captureName' 'wizard')
    $titlePattern = [string] (Get-Member2 $Wizard 'titlePattern' '.')
    $maxPages = [int] (Get-Member2 $Wizard 'maxPages' 8)
    $advanceKeys = @(Get-Member2 $Wizard 'advanceKeys' @('Return'))
    # Per-page overrides: a map of page number → list of key chords, each chord
    # a list of key names pressed together (e.g. Alt+A then Enter on a licence
    # page: { "2": [["Menu","A"],["Return"]] }).
    $advanceByPage = Get-Member2 $Wizard 'advanceByPage'
    $settleMilliseconds = [int] (Get-Member2 $Wizard 'settleMilliseconds' 800)
    # How long a page may take to answer. A page that is working keeps its
    # forward control visible and disabled, so the keys that answer a page are
    # lost while it is busy: a dependency download is minutes of one page, and a
    # page budget with a settle delay attached would kill the wizard in the
    # middle of it.
    $pageTimeoutSeconds = [int] (Get-Member2 $Wizard 'pageTimeoutSeconds' 300)
    $arguments = @(@(Get-Member2 $Wizard 'arguments' @()) | ForEach-Object { [string] $_ })
    # A rooted executable names a program already in the guest; anything else
    # was carried in the payload and staged.
    $executable = if ([System.IO.Path]::IsPathRooted([string] $Wizard.executable)) { [Environment]::ExpandEnvironmentVariables([string] $Wizard.executable) } else { Join-Path $stageRoot ([string] $Wizard.executable) }

    $pages = [System.Collections.Generic.List[object]]::new()
    $processEnd = $null
    $failure = $null
    $processId = 0
    $startedId = 0
    try {
        $startParameters = @{ filePath = $executable; workingDirectory = $stageRoot }
        if ($arguments.Count -gt 0) { $startParameters.arguments = $arguments }
        $started = Invoke-DesktopCommand -Session $Session -Command 'start-process' -Parameters $startParameters
        $startedId = [int] $started.processId
        $processId = Resolve-WindowOwnerProcessId -Session $Session -ProcessId $startedId -TitlePattern $titlePattern

        for ($page = 1; $page -le $maxPages; $page++) {
            $window = $null
            try {
                $window = Invoke-DesktopCommand -Session $Session -Command 'wait-window' -Parameters @{ processId = $processId; titlePattern = $titlePattern; timeoutSeconds = 20 }
            }
            catch {
                if ($null -eq (Get-Process -Id $processId -ErrorAction SilentlyContinue)) { break }
                throw
            }
            # The wizard's last page is answered like any other, and answering
            # it closes the wizard. The window can therefore be gone between
            # being found and being photographed, which is the end of the
            # wizard rather than a failure to capture it: a second look with a
            # short timeout tells the two apart.
            $name = ('{0}-page-{1}.png' -f $captureName, $page)
            $capture = $null
            $obscuredBy = $null
            $obstruction = $null
            try {
                $null = Invoke-DesktopCommand -Session $Session -Command 'window' -Parameters @{ hwnd = $window.hwnd; action = 'activate'; settleMilliseconds = $settleMilliseconds }
                # A capture is of the screen as it is composited, so anything
                # left open over the wizard is in the picture and nothing about
                # the file says so. Asking who owns the middle pixel turns that
                # from an invisible contamination into a recorded fact.
                try {
                    $centreX = [int] ($window.bounds.x + $window.bounds.width / 2)
                    $centreY = [int] ($window.bounds.y + $window.bounds.height / 2)
                    $hit = Invoke-DesktopCommand -Session $Session -Command 'hit-test' -Parameters @{ x = $centreX; y = $centreY }
                    $owner = Get-Member2 $hit 'topLevelWindow'
                    if ($null -ne $owner -and [int64] $owner.hwnd -ne [int64] $window.hwnd) {
                        $obstruction = $owner
                        $obscuredBy = [string] (Get-Member2 $owner 'title')
                        if ([string]::IsNullOrWhiteSpace($obscuredBy)) { $obscuredBy = [string] (Get-Member2 $owner 'className') }
                        if ([string]::IsNullOrWhiteSpace($obscuredBy)) { $obscuredBy = 'another window' }
                    }
                }
                catch {
                    $obscuredBy = $null
                    $obstruction = $null
                }
                $capture = Save-DesktopCapture -Session $Session -Name $name -Destination (Join-Path $artifactRoot 'screenshots') -Hwnd $window.hwnd -Margin 8 -NoCursor
            }
            catch {
                $stillThere = $false
                try {
                    $null = Invoke-DesktopCommand -Session $Session -Command 'wait-window' -Parameters @{ processId = $processId; titlePattern = $titlePattern; timeoutSeconds = 3 }
                    $stillThere = $true
                }
                catch { }
                if ($stillThere) { throw }
                $processEnd = 'finished'
                break
            }
            $signature = ''
            try { $signature = Get-WizardPageSignature -Session $Session -Hwnd $window.hwnd } catch { }
            $tree = $null
            try {
                $tree = Invoke-DesktopCommand -Session $Session -Command 'ui-tree' -Parameters @{ hwnd = $window.hwnd; maxDepth = 8; maxNodes = 400 }
                [System.IO.File]::WriteAllText(
                    (Join-Path $artifactRoot ('{0}-page-{1}-uia.json' -f $captureName, $page)),
                    ($tree | ConvertTo-Json -Depth 20),
                    [System.Text.UTF8Encoding]::new($false))
            }
            catch {
                $tree = [pscustomobject]@{ error = $_.Exception.Message }
            }
            $pages.Add([pscustomobject][ordered]@{
                    page = $page
                    title = $window.title
                    className = $window.className
                    bounds = $window.bounds
                    capture = $capture.name
                    captureSize = "$($capture.width)x$($capture.height)"
                    obscuredBy = $obscuredBy
                    obstruction = $obstruction
                    uiaNodes = (Get-Member2 $tree 'nodeCount')
                    uiaProvider = (Get-Member2 $tree 'provider')
                    uiaError = (Get-Member2 $tree 'error')
                    controls = $signature
                })
            Write-Host ("{0} page {1}: '{2}' {3}x{4} -> {5}" -f $captureName, $page, $window.title, $capture.width, $capture.height, $capture.name)

            if ($null -ne $obscuredBy) {
                # A capture with something else over the wizard is no evidence about
                # the wizard, and neither is anything captured after it: every page
                # that follows is photographed through the same obstruction. Stop
                # here rather than spending the rest of the row producing pictures
                # already known to be worthless. This page and its descriptor stay,
                # because they are what says what was in the way.
                $failure = "Page $page was captured with '$obscuredBy' over the wizard, so nothing this run captures is evidence about it."
                Write-Host "$captureName stopped at page $page`: $failure"
                break
            }

            $chords = @(, $advanceKeys)
            $override = Get-Member2 $advanceByPage ([string] $page)
            if ($null -ne $override) { $chords = @($override | ForEach-Object { , @($_ | ForEach-Object { [string] $_ }) }) }
            foreach ($chord in $chords) {
                $null = Invoke-DesktopCommand -Session $Session -Command 'keyboard' -Parameters @{ action = 'keys'; keys = @($chord); settleMilliseconds = $settleMilliseconds }
            }
            Start-Sleep -Milliseconds $settleMilliseconds
            if ($null -eq (Get-Process -Id $processId -ErrorAction SilentlyContinue)) { break }
            # The page is answered when the wizard leaves it, not when a timer
            # says so. Waiting here is what tells "still working" from
            # "answered", and a page that never changes is a wrong key or a
            # wedged wizard - which is worth saying rather than photographing
            # the same page until the budget runs out.
            $answered = $false
            $deadline = [DateTimeOffset]::Now.AddSeconds($pageTimeoutSeconds)
            while ([DateTimeOffset]::Now -lt $deadline) {
                if ($null -eq (Get-Process -Id $processId -ErrorAction SilentlyContinue)) { $answered = $true; break }
                $current = $null
                try { $current = Get-WizardPageSignature -Session $Session -Hwnd $window.hwnd }
                catch { $answered = $true; break }
                if ($current -ne $signature) { $answered = $true; break }
                Start-Sleep -Milliseconds 500
            }
            if (-not $answered) {
                throw "The wizard stayed on page $page for $pageTimeoutSeconds second(s) after its keys were pressed; its controls are still '$signature'."
            }
            if ($null -eq (Get-Process -Id $processId -ErrorAction SilentlyContinue)) { break }
        }
    }
    catch {
        $failure = $_.Exception.Message
        Write-Host "$captureName failed: $failure"
    }

    if ($processId -gt 0 -and $null -ne (Get-Process -Id $processId -ErrorAction SilentlyContinue)) {
        try { $null = Invoke-DesktopCommand -Session $Session -Command 'stop-process' -Parameters @{ processId = $processId } } catch { }
        $processEnd = 'killed'
    }
    elseif ($processId -gt 0) {
        $processEnd = 'exited'
    }
    # The loader, when there was one, follows its engine out; give it a
    # moment, then make sure.
    if ($startedId -gt 0 -and $startedId -ne $processId) {
        $wait = [DateTime]::UtcNow.AddSeconds(5)
        while ([DateTime]::UtcNow -lt $wait -and $null -ne (Get-Process -Id $startedId -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
        if ($null -ne (Get-Process -Id $startedId -ErrorAction SilentlyContinue)) {
            try { $null = Invoke-DesktopCommand -Session $Session -Command 'stop-process' -Parameters @{ processId = $startedId } } catch { }
        }
    }

    [pscustomobject][ordered]@{
        captureName = $captureName
        executable = [string] $Wizard.executable
        arguments = $arguments
        pages = @($pages)
        processEnd = $processEnd
        error = $failure
    }
}

$results = [System.Collections.Generic.List[object]]::new()
$info = $null
$sessionError = $null
# The lab supplies the session because the job asked for -Desktop, as a
# variable in the scope this script is invoked from. Its absence is a failure to
# report, never something to work around by starting a session here — and it is
# read with Get-Variable because under Set-StrictMode an unset variable throws
# something far less useful than the message below.
$session = Get-Variable -Name 'TigerWinLabDesktop' -ValueOnly -ErrorAction SilentlyContinue
try {
    if ($null -eq $session) {
        throw 'The job provided no interactive desktop session; it must be started with -Desktop.'
    }
    $info = Invoke-DesktopCommand -Session $session -Command 'info'

    # The job workspace is not readable by the interactive account, so the
    # executables are staged where that account can run them.
    $null = New-Item -ItemType Directory -Path $stageRoot -Force
    foreach ($wizard in $wizards) {
        if ([System.IO.Path]::IsPathRooted([string] $wizard.executable)) { continue }
        Copy-Item -LiteralPath (Join-Path $inputRoot ([string] $wizard.executable)) -Destination (Join-Path $stageRoot ([string] $wizard.executable)) -Force
    }
    $null = & icacls.exe $stageRoot '/grant' 'Users:(OI)(CI)RX' '/T' '/Q' 2>&1

    foreach ($wizard in $wizards) {
        $results.Add((Invoke-WizardCapture -Session $session -Wizard $wizard))
    }
}
catch {
    $sessionError = $_.Exception.Message
    Write-Host "desktop session failed: $sessionError"
}
finally {
    # The session belongs to the lab, which set it up and will tear it down;
    # only its log is copied out as evidence.
    if ($null -ne $session) {
        try { Copy-DesktopAgentLog -Session $session -Destination $artifactRoot } catch { }
    }
}

$result = [pscustomobject][ordered]@{
    session = $(if ($null -ne $info) { [pscustomobject]@{ userName = $info.userName; dpi = $info.dpi; scalePercent = $info.scalePercent; screen = $info.screen; dpiAwareness = $info.dpiAwareness } } else { $null })
    wizards = @($results)
    error = $sessionError
}
$json = $result | ConvertTo-Json -Depth 12
[System.IO.File]::WriteAllText($env:TIGERWINLAB_JOB_RESULT, $json, [System.Text.UTF8Encoding]::new($false))
[System.IO.File]::WriteAllText((Join-Path $artifactRoot 'wizard-capture.json'), $json, [System.Text.UTF8Encoding]::new($false))
if ($null -ne $sessionError -or @($results | Where-Object { $null -ne $_.error }).Count -gt 0) { exit 1 }
exit 0
