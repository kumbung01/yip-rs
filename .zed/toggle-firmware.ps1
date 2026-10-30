$ErrorActionPreference = 'Stop'

$settingsPath = Join-Path $PSScriptRoot 'settings.json'
$settings = [System.IO.File]::ReadAllText($settingsPath)
$pattern = '(?m)^(\s*"linkedProjects"\s*:\s*\[\s*")firmware/(central|peripheral)/Cargo\.toml("\s*\])'
$matches = [regex]::Matches($settings, $pattern)
if ($matches.Count -ne 1) {
    throw 'Expected exactly one active linkedProjects entry for central or peripheral.'
}

$firmware = if ($matches[0].Groups[2].Value -eq 'central') { 'peripheral' } else { 'central' }
$updated = [regex]::Replace($settings, $pattern, {
    param($match)
    $match.Groups[1].Value + "firmware/$firmware/Cargo.toml" + $match.Groups[3].Value
})
[System.IO.File]::WriteAllText($settingsPath, $updated, [System.Text.UTF8Encoding]::new($false))
Write-Host "rust-analyzer linkedProjects: $firmware"
