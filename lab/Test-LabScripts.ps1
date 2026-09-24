<#
    .SYNOPSIS
    Static checks over the lab driver's PowerShell, before any of it is run
    against a guest, and one behavioural check of a verdict the rows reach on
    the host.

    .DESCRIPTION
    A lab row costs minutes of virtual-machine time, and a row that dies on a
    variable which is not there has spent all of it to tell you so. Everything
    under lab/ runs with Set-StrictMode -Version Latest, where reading an unset
    variable is a terminating error rather than an empty string, so the mistake
    this catches is exactly the one that is expensive to catch any other way.

    Four checks:

    - **Syntax.** Every script and the module parse.
    - **Undeclared reads.** Inside each function, every variable that is read is
      one of: its own parameters, something assigned earlier in it, a loop or
      catch variable, a script-level parameter or variable, a function declared
      in the same file, or a PowerShell automatic. Anything else is reported
      with its line.
    - **A local that is really a parameter.** Variable names are
      case-insensitive, so `$sessionId = $null` beside a `[string] $SessionId`
      parameter is not a new variable: it silently empties the one the caller
      supplied, and the value is missing everywhere it was going to be used.
      Any assignment whose name matches a parameter of its own scope in a
      different spelling is reported.
    - **A job with no lease policy.** A lab job started without one runs from
      the baseline and hands the VM back when it ends — right for a lone job,
      silently wrong for a step of a chained row, which then measures a clean
      VM. Any function that starts a job (Invoke-TigerSetupGuestCommands,
      Invoke-TigerSetupWizardCapture, Invoke-TigerWinLabEntryPoint) without
      Get-TigerSetupRowStepPolicy or the lab's EntryPolicy/ExitPolicy anywhere
      in its body is reported.

    This is deliberately a linter and not a type checker: it is looking for the
    name that was renamed, the parameter that was removed, the line that was
    pasted into the wrong function, and the step that was added to a row
    without deciding what it starts from.

    One more check runs the module rather than reading it: **the shipped-help
    source check.** The self-installer rows require the shipped Markdown help
    to be CRLF-only and byte for byte the source commit's file as Git checks
    it out. Synthetic cases prove that CRLF-only equal content passes and that
    LF-only, mixed, lone-CR and changed content fail, and a throwaway
    repository proves that the expected bytes come from the commit — not from
    the working tree — and that a file committed mixed is not repaired on the
    way.

    .EXAMPLE
    pwsh -File lab\Test-LabScripts.ps1
#>
[CmdletBinding()]
param(
    # Files to check; defaults to every script and module under lab/.
    [string[]] $Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $Path -or $Path.Count -eq 0) {
    $Path = @(
        Get-ChildItem -LiteralPath $PSScriptRoot -File -Filter '*.ps1' |
            Where-Object { $_.Name -ne 'Test-LabScripts.ps1' } |
            ForEach-Object { $_.FullName }
        Get-ChildItem -LiteralPath $PSScriptRoot -File -Filter '*.psm1' | ForEach-Object { $_.FullName }
        Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'guest') -File -Filter '*.ps1' -ErrorAction SilentlyContinue |
            ForEach-Object { $_.FullName }
    )
}

# Variables PowerShell itself provides. `Matches` and `LASTEXITCODE` are here
# because they are set by an operator or a native command rather than by an
# assignment the parser can see.
$automatic = @(
    '_', 'args', 'error', 'false', 'foreach', 'home', 'host', 'input', 'lastexitcode',
    'matches', 'myinvocation', 'null', 'pid', 'profile', 'psboundparameters',
    'pscmdlet', 'pscommandpath', 'psculture', 'psitem', 'psscriptroot', 'psstyle',
    'psversiontable', 'pwd', 'stacktrace', 'switch', 'this', 'true',
    'executioncontext', 'env', 'global', 'script', 'using', 'ofs',
    'erroractionpreference', 'informationpreference', 'progresspreference',
    'verbosepreference', 'warningpreference', 'debugpreference', 'confirmpreference',
    'whatifpreference', 'outputencoding', 'nestedpromptlevel'
)

function Get-AssignedNames {
    <# Every variable a scriptblock assigns, binds or iterates over. #>
    param([Parameter(Mandatory)] [System.Management.Automation.Language.Ast] $Ast)

    $names = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($assignment in $Ast.FindAll({ $args[0] -is [System.Management.Automation.Language.AssignmentStatementAst] }, $true)) {
        foreach ($variable in $assignment.Left.FindAll({ $args[0] -is [System.Management.Automation.Language.VariableExpressionAst] }, $true)) {
            $null = $names.Add($variable.VariablePath.UserPath)
        }
    }
    foreach ($loop in $Ast.FindAll({ $args[0] -is [System.Management.Automation.Language.ForEachStatementAst] }, $true)) {
        $null = $names.Add($loop.Variable.VariablePath.UserPath)
    }
    foreach ($catch in $Ast.FindAll({ $args[0] -is [System.Management.Automation.Language.CatchClauseAst] }, $true)) {
        if ($catch.Body) { $null = $names.Add('_') }
    }
    # A scriptblock's own param() binds inside it, and a nested function's
    # parameters bind inside that function. Both are declarations as far as the
    # enclosing body is concerned, so neither is an undeclared read.
    foreach ($block in $Ast.FindAll({ $args[0] -is [System.Management.Automation.Language.ScriptBlockExpressionAst] }, $true)) {
        if ($block.ScriptBlock.ParamBlock) {
            foreach ($parameter in $block.ScriptBlock.ParamBlock.Parameters) { $null = $names.Add($parameter.Name.VariablePath.UserPath) }
        }
    }
    foreach ($nested in $Ast.FindAll({ $args[0] -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)) {
        foreach ($parameter in $nested.Parameters) { $null = $names.Add($parameter.Name.VariablePath.UserPath) }
        if ($nested.Body.ParamBlock) {
            foreach ($parameter in $nested.Body.ParamBlock.Parameters) { $null = $names.Add($parameter.Name.VariablePath.UserPath) }
        }
    }
    foreach ($command in $Ast.FindAll({ $args[0] -is [System.Management.Automation.Language.CommandAst] }, $true)) {
        # -ErrorVariable / -OutVariable name a variable without assigning it.
        for ($i = 0; $i -lt $command.CommandElements.Count - 1; $i++) {
            $element = $command.CommandElements[$i]
            if ($element -is [System.Management.Automation.Language.CommandParameterAst] -and $element.ParameterName -match '^(Error|Out|Warning|Information)Variable$') {
                $value = $command.CommandElements[$i + 1]
                if ($value -is [System.Management.Automation.Language.StringConstantExpressionAst]) { $null = $names.Add($value.Value) }
            }
        }
    }
    $names
}

$findings = [System.Collections.Generic.List[object]]::new()

foreach ($file in $Path) {
    $resolved = (Resolve-Path -LiteralPath $file).Path
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($resolved, [ref] $null, [ref] $errors)
    if ($errors) {
        foreach ($parseError in $errors) {
            $findings.Add([pscustomobject]@{
                    file = $resolved; line = $parseError.Extent.StartLineNumber
                    kind = 'syntax'; message = $parseError.Message
                })
        }
        continue
    }

    $functions = @($ast.FindAll({ $args[0] -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true))

    # A parameter and a differently-spelled assignment to the same name are one
    # variable. Reported per scope, because a function's own parameter shadows
    # the script's and only its own is at risk.
    $scopes = @(, [pscustomobject]@{ name = 'the script'; parameters = $ast.ParamBlock; body = $ast }) +
    @($functions | ForEach-Object {
            [pscustomobject]@{
                name = $_.Name
                parameters = $(if ($_.Body.ParamBlock) { $_.Body.ParamBlock } else { $null })
                body = $_.Body
            }
        })
    foreach ($scope in $scopes) {
        $declared = @{}
        if ($null -ne $scope.parameters) {
            foreach ($parameter in $scope.parameters.Parameters) { $declared[$parameter.Name.VariablePath.UserPath] = $true }
        }
        if ($declared.Count -eq 0) { continue }
        foreach ($assignment in $scope.body.FindAll({ $args[0] -is [System.Management.Automation.Language.AssignmentStatementAst] }, $true)) {
            foreach ($variable in $assignment.Left.FindAll({ $args[0] -is [System.Management.Automation.Language.VariableExpressionAst] }, $true)) {
                $assigned = $variable.VariablePath.UserPath
                $match = @($declared.Keys | Where-Object { $_ -eq $assigned -and -not $_.Equals($assigned, [StringComparison]::Ordinal) })
                if ($match.Count -eq 0) { continue }
                $findings.Add([pscustomobject]@{
                        file = $resolved; line = $variable.Extent.StartLineNumber
                        kind = 'shadowed-parameter'
                        message = "$($scope.name) assigns `$$assigned, which is the parameter `$$($match[0]) spelled differently, so the value it was given is lost."
                    })
            }
        }
    }

    # Names visible to every function in the file: script parameters, anything
    # the script body assigns at top level, and the functions themselves.
    $fileScope = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    if ($ast.ParamBlock) {
        foreach ($parameter in $ast.ParamBlock.Parameters) { $null = $fileScope.Add($parameter.Name.VariablePath.UserPath) }
    }
    foreach ($name in (Get-AssignedNames -Ast $ast)) { $null = $fileScope.Add($name) }
    foreach ($function in $functions) { $null = $fileScope.Add($function.Name) }

    foreach ($function in $functions) {
        $known = [System.Collections.Generic.HashSet[string]]::new($fileScope, [StringComparer]::OrdinalIgnoreCase)
        if ($function.Body.ParamBlock) {
            foreach ($parameter in $function.Body.ParamBlock.Parameters) { $null = $known.Add($parameter.Name.VariablePath.UserPath) }
        }
        foreach ($parameter in $function.Parameters) { $null = $known.Add($parameter.Name.VariablePath.UserPath) }
        foreach ($name in (Get-AssignedNames -Ast $function.Body)) { $null = $known.Add($name) }

        # One finding per name per line: the same variable read twice on one
        # line is one mistake to fix, not two.
        $reported = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($variable in $function.Body.FindAll({ $args[0] -is [System.Management.Automation.Language.VariableExpressionAst] }, $true)) {
            $name = $variable.VariablePath.UserPath
            if ($variable.VariablePath.IsDriveQualified) { continue }
            if ($automatic -contains $name.ToLowerInvariant()) { continue }
            if ($known.Contains($name)) { continue }
            if (-not $reported.Add("$($variable.Extent.StartLineNumber):$name")) { continue }
            $findings.Add([pscustomobject]@{
                    file = $resolved; line = $variable.Extent.StartLineNumber
                    kind = 'undeclared'
                    message = "$($function.Name) reads `$$name, which nothing in scope declares or assigns."
                })
        }
    }

    # A lab job started without a lease policy runs from the baseline and
    # hands the VM back when it ends: the right default for a lone job, and
    # silently wrong for a step of a chained row, which then measures a clean
    # VM (LESSONS_LEARNED.md). A function that starts a job must say what the
    # job starts from, somewhere in its body: Get-TigerSetupRowStepPolicy,
    # a splat of it, or the lab's own EntryPolicy/ExitPolicy parameters. The
    # job helpers themselves take the policies as parameters and pass them on.
    $starters = @('Invoke-TigerSetupGuestCommands', 'Invoke-TigerSetupWizardCapture', 'Invoke-TigerWinLabEntryPoint')
    foreach ($function in $functions) {
        if ($function.Name -in $starters) { continue }
        $starts = @($function.Body.FindAll({ $args[0] -is [System.Management.Automation.Language.CommandAst] }, $true) |
                Where-Object { $_.GetCommandName() -in $starters })
        if ($starts.Count -eq 0) { continue }
        if ($function.Body.Extent.Text -match 'Get-TigerSetupRowStepPolicy|EntryPolicy|ExitPolicy') { continue }
        $findings.Add([pscustomobject]@{
                file = $resolved; line = $starts[0].Extent.StartLineNumber
                kind = 'no-lease-policy'
                message = "$($function.Name) starts a lab job with $($starts[0].GetCommandName()) and never says what it starts from: a step of a chained row needs Get-TigerSetupRowStepPolicy, or the lab's EntryPolicy and ExitPolicy."
            })
    }
}

# The shipped-help source check, exercised rather than read: a Windows text
# file ships CRLF-only and exactly as Git checks its commit out, and nothing
# between the two is normalized (LESSONS_LEARNED.md). The repository is a
# throwaway one with its own core.autocrlf=true — the policy the release
# checkout applies — so the result does not depend on this machine's Git.
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force
function Add-BehaviourFinding {
    param([string] $Message)
    $findings.Add([pscustomobject]@{ file = (Join-Path $PSScriptRoot 'TigerSetupLab.psm1'); line = 0; kind = 'help-source-check'; message = $Message })
}
function Get-Verdict {
    <# The two check statuses for a shipped text against an expected one, as 'crlf/source'. #>
    param([AllowNull()] [object] $Shipped, [string] $Expected)
    $shippedBytes = $(if ($null -ne $Shipped) { , [System.Text.Encoding]::UTF8.GetBytes($Shipped) } else { $null })
    $checks = @(Get-TigerSetupCommittedTextChecks -Step 'test' -Name 'the file' -Code 'test' -Shipped $shippedBytes `
            -Expected ([System.Text.Encoding]::UTF8.GetBytes($Expected)) -ExpectedLabel 'commit:file')
    (@($checks | Where-Object { $_.code -eq 'test.crlf' })[0].status) + '/' + (@($checks | Where-Object { $_.code -eq 'test.source' })[0].status)
}
$crlfText = "# Help`r`n`r`nOne line.`r`n"
foreach ($case in @(
        @{ name = 'CRLF-only, as committed'; shipped = $crlfText; expected = $crlfText; verdict = 'PASS/PASS' },
        @{ name = 'LF-only'; shipped = "# Help`n`nOne line.`n"; expected = $crlfText; verdict = 'FAIL/FAIL' },
        @{ name = 'mixed CRLF and LF'; shipped = "# Help`r`n`nOne line.`r`n"; expected = $crlfText; verdict = 'FAIL/FAIL' },
        @{ name = 'a lone CR'; shipped = "# Help`r`n`rOne line.`r`n"; expected = $crlfText; verdict = 'FAIL/FAIL' },
        @{ name = 'CRLF-only with other content'; shipped = "# Help`r`n`r`nAnother line.`r`n"; expected = $crlfText; verdict = 'PASS/FAIL' },
        @{ name = 'CRLF-only with trailing whitespace'; shipped = "# Help `r`n`r`nOne line.`r`n"; expected = $crlfText; verdict = 'PASS/FAIL' },
        @{ name = 'LF-only, equal to an LF expected source'; shipped = "# Help`n"; expected = "# Help`n"; verdict = 'FAIL/PASS' },
        @{ name = 'not shipped'; shipped = $null; expected = $crlfText; verdict = 'FAIL/FAIL' })) {
    $verdict = Get-Verdict -Shipped $case.shipped -Expected $case.expected
    if ($verdict -ne $case.verdict) { Add-BehaviourFinding "$($case.name): crlf/source is $verdict, not $($case.verdict)." }
}

$scratch = Join-Path ([System.IO.Path]::GetTempPath()) ('TigerSetupEol-' + [Guid]::NewGuid().ToString('N'))
try {
    $null = New-Item -ItemType Directory -Path $scratch
    $git = { param([string[]] $Arguments) $output = & git -C $scratch @Arguments 2>&1 | Out-String; if ($LASTEXITCODE -ne 0) { throw "git $($Arguments -join ' '): $output" }; $output.Trim() }
    $file = Join-Path $scratch 'help.md'
    $null = & $git @('init', '--quiet')
    $null = & $git @('config', 'core.autocrlf', 'true')
    $null = & $git @('config', 'user.name', 'Test-LabScripts')
    $null = & $git @('config', 'user.email', 'test-labscripts@invalid')
    $null = & $git @('config', 'commit.gpgsign', 'false')
    $null = & $git @('config', 'core.hooksPath', 'no-hooks')
    # Written LF-only, as an agent writes it; committed; then the working tree
    # is rewritten to other content, and the commit's checkout must not care.
    [System.IO.File]::WriteAllText($file, "# Help`nFirst.`n")
    $null = & $git @('add', 'help.md')
    $null = & $git @('commit', '--quiet', '-m', 'first')
    $first = & $git @('rev-parse', 'HEAD')
    [System.IO.File]::WriteAllText($file, "# Help`nSecond.`n")
    $null = & $git @('commit', '--quiet', '-am', 'second')
    [System.IO.File]::WriteAllText($file, "# Help`nUncommitted.`n")
    $atFirst = Get-TigerSetupCommittedFile -RepositoryRoot $scratch -Commit $first -Path 'help.md'
    if ([System.Text.Encoding]::UTF8.GetString($atFirst) -cne "# Help`r`nFirst.`r`n") {
        Add-BehaviourFinding "the first commit as Git checks it out is '$([System.Text.Encoding]::UTF8.GetString($atFirst))', not its content in CRLF."
    }
    $verdict = Get-Verdict -Shipped "# Help`r`nFirst.`r`n" -Expected ([System.Text.Encoding]::UTF8.GetString($atFirst))
    if ($verdict -ne 'PASS/PASS') { Add-BehaviourFinding "a CRLF copy of the first commit's file against that commit: $verdict, not PASS/PASS." }
    $verdict = Get-Verdict -Shipped "# Help`r`nUncommitted.`r`n" -Expected ([System.Text.Encoding]::UTF8.GetString($atFirst))
    if ($verdict -ne 'PASS/FAIL') { Add-BehaviourFinding "the working tree's content against the first commit: $verdict, not PASS/FAIL." }
    # A file committed mixed stays mixed on checkout: Git does not repair it,
    # and the check must not either.
    $null = & $git @('config', 'core.autocrlf', 'false')
    [System.IO.File]::WriteAllText($file, "# Help`r`nMixed.`n")
    $null = & $git @('commit', '--quiet', '-am', 'mixed')
    $null = & $git @('config', 'core.autocrlf', 'true')
    $mixed = Get-TigerSetupCommittedFile -RepositoryRoot $scratch -Commit (& $git @('rev-parse', 'HEAD')) -Path 'help.md'
    $endings = Get-TigerSetupLineEndings -Bytes $mixed
    if ($endings.kind -ne 'mixed') { Add-BehaviourFinding "a file committed mixed checks out as '$($endings.kind)', not 'mixed'." }
}
finally { Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue }

if ($findings.Count -eq 0) {
    Write-Host "lab scripts: $($Path.Count) file(s) checked, no findings."
    exit 0
}

foreach ($finding in $findings) {
    Write-Host ("{0}({1}): {2}: {3}" -f (Split-Path -Leaf $finding.file), $finding.line, $finding.kind, $finding.message)
}
Write-Host ""
Write-Host "lab scripts: $($findings.Count) finding(s) in $($Path.Count) file(s)."
exit 1
