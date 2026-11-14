# Takes the user guide's screenshots (docs/guide/images, ADR 0069): the desktop app at its fixed
# window size (1600 x 940) with the sample animation, one `--open` state per picture, scaled down
# to 1100 px across. Run it after a change that alters what a picture shows, look at every PNG,
# and commit them. The pictures are original work (ATTRIBUTION.md).
#
#   scripts/guide-shots.ps1            # all of them
#   scripts/guide-shots.ps1 library    # only the ones whose name contains "library"
param([string]$Only = "")
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$Out = "docs/guide/images"
$Width = 1100

cargo build -p anicraft --bin anicraft
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$App = Join-Path $TargetDir "debug/anicraft.exe"
if (-not (Test-Path $App)) { $App = Join-Path $TargetDir "debug/anicraft" }
if (-not (Test-Path $App)) { Write-Error "$App was not built" }

# picture name, then the --open state ("-" for the window as it starts)
$Shots = @(
  @("workspace", "-"),
  @("start-screen", "start"),
  @("library", "library"),
  @("onion-skin", "timelinepolish"),
  @("motion-editor", "motion"),
  @("test-movie", "testmovieinside"),
  @("publish-settings", "publish"),
  @("preferences", "prefs")
)
New-Item -ItemType Directory -Force $Out | Out-Null
foreach ($s in $Shots) {
  $name, $state = $s
  if ($Only -and -not $name.Contains($Only)) { continue }
  $argv = @("--screenshot", "$Out/$name.png", "--shot-width", "$Width", "--lang", "en")
  if ($state -ne "-") { $argv += @("--open", $state) }
  & $App @argv
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
Write-Output "Pictures in ${Out}: look at each one before committing."
