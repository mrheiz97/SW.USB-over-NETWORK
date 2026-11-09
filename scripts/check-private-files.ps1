$ErrorActionPreference = 'Stop'
$tracked = @(git ls-files | Where-Object { $_ -match '(^|/)AGENTS\.md$' })
if ($tracked.Count -gt 0) {
    Write-Error "Private AGENTS.md file is tracked. Remove it from the Git index."
    exit 1
}
