# Puts the latest work on GitHub: pushes the commits, then publishes the setup
# file as a release, so people can download it from the repository's Releases
# page. Running it again for the same version replaces the setup file there.
#
#   powershell -ExecutionPolicy Bypass -File .\publish-release.ps1
Set-Location $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

function Stop-With($message) {
  Write-Host $message -ForegroundColor Red
  exit 1
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$setup = "dist\Stream Dock HA Setup $version.exe"
$tag = "v$version"

# A setup file older than the code would publish an old version.
$newest = Get-ChildItem src, assets, Cargo.toml -Recurse -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not (Test-Path $setup) -or (Get-Item $setup).LastWriteTime -lt $newest.LastWriteTime) {
  Write-Host 'Making a new setup file first...'
  & .\make-setup.ps1
  if (-not $?) { Stop-With 'The setup file could not be made. If Stream Dock HA is running from this folder, quit it and try again.' }
}

# GitHub would turn spaces into dots, and a name without the version keeps the
# README's download link working for every release.
$upload = 'dist\Stream-Dock-HA-Setup.exe'
Copy-Item -LiteralPath $setup $upload -Force

Write-Host 'Pushing the commits...'
git push origin main
if ($LASTEXITCODE -ne 0) { Stop-With 'Pushing failed, so nothing was published.' }

$ErrorActionPreference = 'SilentlyContinue'
gh release view $tag *> $null
$exists = $LASTEXITCODE -eq 0
$ErrorActionPreference = 'Continue'

if ($exists) {
  Write-Host "Replacing the setup file in release $tag..."
  gh release upload $tag $upload --clobber
} else {
  Write-Host "Publishing release $tag..."
  gh release create $tag $upload --title "Stream Dock HA $version" --notes-file docs\release-notes.md
}
if ($LASTEXITCODE -ne 0) { Stop-With 'Publishing the release failed.' }

Write-Host 'Done: https://github.com/Max-src/stream-dock-ha/releases/latest' -ForegroundColor Green
