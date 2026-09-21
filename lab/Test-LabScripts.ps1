<#
    .SYNOPSIS
    Static checks over the lab driver's PowerShell, before any of it is run
    against a guest.

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
