# Builds the web editor into web\ (ADR 0036, docs\web.md): index.html, anicraft.js,
# anicraft_bg.wasm and the page icon. Everything in web\ is generated (git-ignored); serve the
# folder with any static web server. The Windows twin of scripts/build-web.sh.
#
# Needs the wasm32-unknown-unknown target and the wasm-bindgen CLI at the version Cargo.lock
# pins for the wasm-bindgen crate (both MIT OR Apache-2.0):
#
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <the version this script asks for> --locked
#
#   scripts\build-web.ps1          # optimized (the `web` profile)
#   scripts\build-web.ps1 -Dev     # quicker to build, larger and slower
[CmdletBinding()]
param([switch]$Dev)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$CargoProfile = if ($Dev) { 'dev' } else { 'web' }

# The CLI and the crate must be the same version, or the generated glue doesn't match.
$lock = Get-Content -Raw (Join-Path $Root 'Cargo.lock')
$m = [regex]::Match($lock, '(?m)^name = "wasm-bindgen"\r?\nversion = "([^"]+)"')
if (-not $m.Success) { throw 'Cargo.lock names no wasm-bindgen version' }
$Want = $m.Groups[1].Value
if (-not (Get-Command wasm-bindgen -ErrorAction SilentlyContinue)) {
    throw "wasm-bindgen not found: cargo install wasm-bindgen-cli --version $Want --locked"
}
$Have = ((& wasm-bindgen --version) -split ' ')[1]
if ($Have -ne $Want) {
    throw "wasm-bindgen $Have found, Cargo.lock needs ${Want}: cargo install wasm-bindgen-cli --version $Want --locked"
}
if (Get-Command rustup -ErrorAction SilentlyContinue) {
    if (-not ((& rustup target list --installed) -contains 'wasm32-unknown-unknown')) {
        throw 'the wasm32-unknown-unknown target is missing: rustup target add wasm32-unknown-unknown'
    }
}

& cargo build -p anicraft --bin anicraft --target wasm32-unknown-unknown --profile $CargoProfile
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
$Dir = if ($Dev) { 'debug' } else { $CargoProfile }
$Wasm = Join-Path $TargetDir "wasm32-unknown-unknown\$Dir\anicraft.wasm"
$Out = Join-Path $Root 'web'
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
New-Item -ItemType Directory -Force $Out | Out-Null
& wasm-bindgen --target web --no-typescript --out-dir $Out --out-name anicraft $Wasm
if ($LASTEXITCODE -ne 0) { throw 'wasm-bindgen failed' }
Copy-Item (Join-Path $Root 'packaging\web\index.html') (Join-Path $Out 'index.html')
Copy-Item (Join-Path $Root 'assets\app-icon\hicolor\128x128\apps\ai.storyteller.anicraft.png') (Join-Path $Out 'anicraft.png')
# The user guide beside the editor: Help › User Guide opens guide/index.html (ADR 0069).
cargo run -q -p anicraft-cli -- guide --out (Join-Path $Out 'guide')
if ($LASTEXITCODE -ne 0) { throw 'writing the user guide failed' }

$Bytes = (Get-Item (Join-Path $Out 'anicraft_bg.wasm')).Length
Write-Host ("web\ is ready: anicraft_bg.wasm is {0} KB." -f [math]::Floor($Bytes / 1024))
Write-Host 'Serve it locally, for example: python -m http.server --directory web 8080  (then open http://localhost:8080)'
