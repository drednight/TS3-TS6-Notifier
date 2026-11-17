# Stage a distributable folder + zip for Windows.
#   pwsh native/tools/package-win.ps1 [-OutDir dist-win]
# Expects native/target/release/tsn-native.exe (cargo build --release).
# The plugin DLL is included when dist/ts3plugin_tsn_win64.dll exists (prebuilt).

param([string]$OutDir = "dist-win")

$ErrorActionPreference = "Stop"
$native = Split-Path -Parent $PSScriptRoot        # native/
$repo   = Split-Path -Parent $native              # repo root
$stage  = Join-Path $OutDir "tsn-native"

$exe = Join-Path $native "target/release/tsn-native.exe"
if (-not (Test-Path $exe)) {
    Write-Error "tsn-native.exe not found — run: cargo build --release --manifest-path native/Cargo.toml"
}

Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Copy-Item $exe $stage/
Copy-Item -Recurse (Join-Path $repo "public") (Join-Path $stage "public")
Copy-Item (Join-Path $native "README.md") $stage/

# plugin DLL (prebuilt — see plugin/README.md)
$dll = Join-Path $repo "dist/ts3plugin_tsn_win64.dll"
if (Test-Path $dll) {
    New-Item -ItemType Directory -Force -Path (Join-Path $stage "dist") | Out-Null
    Copy-Item $dll (Join-Path $stage "dist/")
}

# data folder with the live config when present (otherwise the app writes defaults)
New-Item -ItemType Directory -Force -Path (Join-Path $stage "data") | Out-Null
$cfg = Join-Path $repo "data/config.json"
if (Test-Path $cfg) {
    Copy-Item $cfg (Join-Path $stage "data/")
}

$zip = Join-Path $OutDir "tsn-native-win64.zip"
Compress-Archive -Force -Path $stage -DestinationPath $zip
Write-Host "staged: $stage"
Write-Host "zip:    $zip"
