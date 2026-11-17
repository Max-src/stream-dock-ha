# Builds the optimised program and wraps it in a setup wizard made with Inno
# Setup (installer\stream-dock-ha.iss), saved in the dist folder.
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
# The compiler leaves source paths in the program, for error messages. These
# shorten them so the setup file doesn't carry this PC's user folder or name.
# When several match, the last one wins. They're joined with the character
# Cargo expects there, as the paths can have spaces.
$env:CARGO_ENCODED_RUSTFLAGS = @(
  "--remap-path-prefix=$env:USERPROFILE=home",
  "--remap-path-prefix=$env:USERPROFILE\.cargo\registry\src=cargo",
  "--remap-path-prefix=$env:USERPROFILE\.rustup=rustup",
  "--remap-path-prefix=$PSScriptRoot=app"
) -join [char]0x1f
# Its own folder, so a copy of the app running from target\release doesn't block it.
# Without the "shots" feature, so the hooks for pictures of the window stay out.
cargo build --release --no-default-features --target-dir target\setup-build
if ($LASTEXITCODE -ne 0) { throw 'The build failed' }
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

$iscc = @(
  "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
  "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
  "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 is needed to make the setup file: https://jrsoftware.org/isdl.php' }

New-Item -ItemType Directory -Force dist | Out-Null
& $iscc /Q "/DAppVersion=$version" installer\stream-dock-ha.iss
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }
$target = "dist\Stream Dock HA Setup $version.exe"
Write-Host "Setup file: $((Resolve-Path $target).Path)"
