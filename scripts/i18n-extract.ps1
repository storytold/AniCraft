# Lists every AniCraft interface string for translators (ADR 0040, docs/localisation.md), as a
# craft-i18n catalog template. Same as scripts/i18n-extract.sh:
#
#   scripts/i18n-extract.ps1 [out.tsv]                     # default: target/i18n/anicraft.tsv
#   $env:CRAFT_I18N_DIR = 'C:\src\craft-i18n'; scripts/i18n-extract.ps1    # + coverage per locale
#   $env:ANICRAFT_I18N_LOCALE = 'de'; scripts/i18n-extract.ps1 de.tsv     # only what German lacks
param([string]$Out = '')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if ($Out -eq '') { $Out = Join-Path $root 'target/i18n/anicraft.tsv' }
elseif (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path (Get-Location) $Out }
Push-Location $root
try {
    $env:ANICRAFT_I18N_OUT = $Out
    cargo test --quiet --bin anicraft i18n::tests::extract -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "extraction failed ($LASTEXITCODE)" }
} finally {
    Remove-Item Env:ANICRAFT_I18N_OUT -ErrorAction SilentlyContinue
    Pop-Location
}
