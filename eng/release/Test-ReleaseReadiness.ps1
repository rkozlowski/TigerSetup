<#
    .SYNOPSIS
    Release preparation's last gate: is the release workflow ready to run for
    this tree - the repository's settings, the runner it will get, and this
    machine's access to what release validation retrieves?

    .DESCRIPTION
    Run on dev, after the prepared release is pushed and the runner preflight
    has run on it. It changes nothing on GitHub or in the working tree (it
    fetches origin). Each check names where its
    evidence comes from, because a maintainer's login proving something is not
    the release workflow's token proving it:

      config/repository   the repository is public, so the installer and terms
                          URLs serve anonymously, and its default branch is main
      config/actions      GitHub Actions is enabled and allows the GitHub-owned
                          actions the release workflow uses by version tag
      config/environment  the release environment, where the tag and the draft
                          are made, deploys from main and nothing else - the
                          settings half of the branch safety, independent of the
                          workflow's own ref check
      config/rulesets     no active ruleset targets tags, which could refuse
                          github-actions[bot] the release tag
      config/workflow     origin/main holds the release workflow, so it can be
                          started
      runner/preflight    a 'Runner diagnostics' release-readiness run succeeded
                          on a hosted runner for a commit whose runner-facing
                          files (.github/workflows, eng/release, .cargo,
                          Cargo.lock) are this tree's: the toolchain, tiger-mark
                          built and rendering from the pin, the crates, the
                          release tooling's tests and read access with the job's
                          own token, proved on the runner image itself
      local/gh            this machine's gh login has push access, which
                          release validation needs to see and download a draft

    The config checks read the repository's settings with this machine's gh
    login, so they prove the configuration, not the workflow token's use of
    it. The token's write access - the tag push and the draft - cannot be
    proved without writing; the release action's first write is the tag push,
    after every read it depends on has succeeded, and a rerun of its failed job
    completes it (RELEASING.md, Recovery).

    Exit codes: 0 PASS, 2 BLOCKED (a step has not happened yet: push, run the
    preflight, log in), 1 FAIL (a setting is wrong).

    .EXAMPLE
    pwsh -File eng\release\Test-ReleaseReadiness.ps1
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    # The tree being released; defaults to HEAD.
    [string] $CommitSha = '',
    # The checkout to check; defaults to this repository.
    [string] $RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'TigerSetupRelease.psm1')
$repoRoot = if ($RepositoryRoot) { $RepositoryRoot } else { Split-Path -Parent (Split-Path -Parent $PSScriptRoot) }
$facts = Get-TigerSetupReleaseFacts
$repo = "repos/$($facts.Repository)"
if (-not $CommitSha) { $CommitSha = (Invoke-TigerSetupGit $repoRoot @('rev-parse', 'HEAD')).output }
$CommitSha = $CommitSha.ToLowerInvariant()
$checks = [Collections.Generic.List[object]]::new()
function Add-Check { param($Id, $Status, $Observed, $Remediation = '') $checks.Add((New-TigerSetupReleaseCheck -Id $Id -Status $Status -Observed $Observed -Remediation $Remediation)) }
function Get-Api { param([string] $Path) Invoke-TigerSetupGitHubApi $Path }
# A field GitHub may leave out of a response; $null when it did.
function Get-Field { param([object] $Object, [string] $Name) if ($null -ne $Object -and $null -ne $Object.PSObject.Properties[$Name]) { $Object.$Name } }

# The repository.
$repository = Get-Api $repo
if (-not $repository.ok) {
    Add-Check 'config/repository' BLOCKED "This machine's gh cannot read $($facts.Repository): $($repository.error)" 'Authenticate gh (gh auth login) and rerun.'
    exit (Write-TigerSetupReleaseReport -Title "Release readiness of TigerSetup at $CommitSha" -Checks $checks.ToArray())
}
$data = $repository.data
if ([string] $data.visibility -ceq 'public' -and [string] $data.default_branch -ceq $facts.DefaultBranch -and -not [bool] $data.archived) {
    Add-Check 'config/repository' PASS "$($facts.Repository) is public, its default branch is $($facts.DefaultBranch)."
}
else {
    Add-Check 'config/repository' FAIL "$($facts.Repository) is $($data.visibility), default branch '$($data.default_branch)', archived $($data.archived)." 'The release URLs must be public and the release branch must be main (Architect).'
}

# Actions and the actions it allows.
$actions = Get-Api "$repo/actions/permissions"
if (-not $actions.ok) { Add-Check 'config/actions' BLOCKED "Cannot read the Actions settings: $($actions.error)" }
else {
    # A repository with Actions disabled answers without allowed_actions.
    $allowed = [string] (Get-Field $actions.data 'allowed_actions')
    $pinning = [bool] (Get-Field $actions.data 'sha_pinning_required')
    $githubOwned = $allowed -ceq 'all'
    if ($allowed -ceq 'selected') {
        $selected = Get-Api "$repo/actions/permissions/selected-actions"
        $githubOwned = $selected.ok -and [bool] (Get-Field $selected.data 'github_owned_allowed')
    }
    if (-not [bool] (Get-Field $actions.data 'enabled')) { Add-Check 'config/actions' FAIL 'GitHub Actions is disabled for the repository.' 'Enable Actions (Architect).' }
    elseif (-not $githubOwned) { Add-Check 'config/actions' FAIL "Actions allows '$allowed', which does not include the GitHub-owned actions the release workflow uses." 'Allow GitHub-owned actions (Architect).' }
    elseif ($pinning) { Add-Check 'config/actions' FAIL 'The repository requires actions pinned by SHA; the release workflow uses version tags proven by its runs.' 'Decide the pinning policy (Architect).' }
    else { Add-Check 'config/actions' PASS "Actions is enabled and allows '$allowed' actions; version tags are accepted." }
}

# The release environment: main only.
$environment = Get-Api "$repo/environments/$($facts.ReleaseEnvironment)"
if (-not $environment.ok -and $environment.error -match 'HTTP 404|Not Found') {
    Add-Check 'config/environment' FAIL "The repository has no '$($facts.ReleaseEnvironment)' environment: $($environment.error)" "Create it, restricted to $($facts.DefaultBranch) (Architect)."
}
elseif (-not $environment.ok) {
    Add-Check 'config/environment' BLOCKED "The '$($facts.ReleaseEnvironment)' environment cannot be read: $($environment.error)" 'Rerun with a maintainer''s gh login once GitHub answers.'
}
else {
    $policy = Get-Field $environment.data 'deployment_branch_policy'
    $branches = Get-Api "$repo/environments/$($facts.ReleaseEnvironment)/deployment-branch-policies"
    $named = @(if ($branches.ok) { @(Get-Field $branches.data 'branch_policies') | Where-Object { $_ } | ForEach-Object { "$(Get-Field $_ 'type'):$(Get-Field $_ 'name')" } })
    $rules = @(@(Get-Field $environment.data 'protection_rules') | Where-Object { $_ } | ForEach-Object { [string] (Get-Field $_ 'type') })
    if ($null -ne $policy -and [bool] (Get-Field $policy 'custom_branch_policies') -and -not [bool] (Get-Field $policy 'protected_branches') -and ($named -join '|') -ceq "branch:$($facts.DefaultBranch)") {
        Add-Check 'config/environment' PASS "'$($facts.ReleaseEnvironment)' deploys only from the branch $($facts.DefaultBranch) (protection rules: $($rules -join ', '))."
    }
    else {
        $described = if ($null -eq $policy) { 'any branch' } else { "custom $($policy.custom_branch_policies), protected branches $($policy.protected_branches), policies [$($named -join ', ')]" }
        Add-Check 'config/environment' FAIL "'$($facts.ReleaseEnvironment)' deploys from $described." "Restrict its deployment branches to exactly '$($facts.DefaultBranch)' (Architect; RELEASING.md)."
    }
}

# Rulesets that could refuse the release tag.
$rulesets = Get-Api "$repo/rulesets?includes_parents=true&per_page=100"
if (-not $rulesets.ok) { Add-Check 'config/rulesets' BLOCKED "Cannot read the rulesets: $($rulesets.error)" }
else {
    $tagRules = @(@($rulesets.data) | Where-Object { $_ -and [string] $_.target -ceq 'tag' -and [string] $_.enforcement -ceq 'active' })
    if ($tagRules.Count) {
        Add-Check 'config/rulesets' BLOCKED "Active tag rulesets: $(@($tagRules | ForEach-Object { "'$($_.name)'" }) -join ', ')." 'Confirm each lets github-actions[bot] create v<version>, or exempt it (Architect), then rerun.'
    }
    else { Add-Check 'config/rulesets' PASS "No active ruleset targets tags ($(@($rulesets.data).Count) ruleset(s) in all)." }
}

# The workflow can be started on main.
$branch = $facts.DefaultBranch
$tip = Invoke-TigerSetupGit $repoRoot @('ls-remote', '--heads', 'origin', "refs/heads/$branch")
if (-not $tip.ok) { Add-Check 'config/workflow' BLOCKED "origin cannot be read: $($tip.output)" 'Make origin reachable and rerun.' }
elseif (-not $tip.output) { Add-Check 'config/workflow' FAIL "origin has no $branch; the release workflow cannot be started." }
else {
    $fetched = Invoke-TigerSetupGit $repoRoot @('fetch', '--quiet', 'origin', "+refs/heads/${branch}:refs/remotes/origin/${branch}")
    $onMain = $fetched.ok -and (Invoke-TigerSetupGit $repoRoot @('cat-file', '-e', "refs/remotes/origin/${branch}:$($facts.ReleaseWorkflowPath)")).ok
    Add-Check 'config/workflow' $(if ($onMain) { 'PASS' } else { 'FAIL' }) $(if ($onMain) { "origin/$branch holds $($facts.ReleaseWorkflowPath)." } else { "origin/$branch has no $($facts.ReleaseWorkflowPath); it cannot be started." })
}

# The runner preflight, for this tree's runner-facing files.
$treeOf = {
    param([string] $Commit)
    @($facts.RunnerFacingPaths | ForEach-Object {
            $object = Invoke-TigerSetupGit $repoRoot @('rev-parse', '--verify', '--quiet', "${Commit}:$_")
            "$_=$(if ($object.ok) { $object.output } else { 'absent' })"
        }) -join ';'
}
$wanted = & $treeOf $CommitSha
$runs = Get-Api "$repo/actions/workflows/$(Split-Path -Leaf $facts.RunnerDiagnosticsWorkflowPath)/runs?status=success&per_page=50"
if (-not $runs.ok) { Add-Check 'runner/preflight' BLOCKED "Cannot list the runner diagnostics' runs: $($runs.error)" }
else {
    $null = Invoke-TigerSetupGit $repoRoot @('fetch', '--quiet', 'origin')
    $match = $null
    foreach ($run in @($runs.data.workflow_runs | Where-Object { $_ -and [string] $_.display_title -ceq $facts.RunnerReadinessRunTitle -and [string] $_.conclusion -ceq 'success' })) {
        $present = Invoke-TigerSetupGit $repoRoot @('cat-file', '-e', "$([string] $run.head_sha)^{commit}")
        if ($present.ok -and (& $treeOf ([string] $run.head_sha)) -ceq $wanted) { $match = $run; break }
    }
    if ($null -ne $match) {
        # ConvertFrom-Json has already made GitHub's ISO 8601 time a DateTime;
        # a cast, unlike a parse of its text, does not depend on the culture.
        $age = [DateTimeOffset]::UtcNow - [DateTimeOffset] $match.created_at
        Add-Check 'runner/preflight' PASS "$($match.html_url) succeeded $([Math]::Round($age.TotalDays, 1)) day(s) ago, on $([string] $match.head_branch) at $([string] $match.head_sha), whose runner-facing files are this tree's."
    }
    else {
        Add-Check 'runner/preflight' BLOCKED "No successful '$($facts.RunnerReadinessRunTitle)' run covers this tree's runner-facing files ($($facts.RunnerFacingPaths -join ', '))." `
            "Push the prepared release to dev, start 'Runner diagnostics' (release-readiness) on dev, and rerun this script: gh workflow run $(Split-Path -Leaf $facts.RunnerDiagnosticsWorkflowPath) --repo $($facts.Repository) --ref dev"
    }
}

# This machine's access, which release validation uses.
$push = $null -ne $data.PSObject.Properties['permissions'] -and [bool] $data.permissions.push
Add-Check 'local/gh' $(if ($push) { 'PASS' } else { 'BLOCKED' }) $(if ($push) { "This machine's gh login has push access to $($facts.Repository), so it sees and downloads a draft." } else { "This machine's gh login has no push access to $($facts.Repository); it cannot see a draft." }) 'Log gh in as a maintainer (gh auth login).'

exit (Write-TigerSetupReleaseReport -Title "Release readiness of TigerSetup at $CommitSha" -Checks $checks.ToArray() -Next @(
        'The workflow is ready for this tree. The Architect promotes dev to main and starts the release (RELEASING.md).'
    ))
