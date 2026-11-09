<#
.SYNOPSIS
  Build, sign and package AniCraft for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    anicraft-<version>-windows-<arch>.msi            installer (WiX v5, per-user by default, everyone optional)
    anicraft-<version>-windows-<arch>-portable.zip   anicraft.exe + anicraft-cli.exe + notices
    anicraft-<version>-windows-<arch>-symbols.zip    anicraft.pdb + anicraft_cli.pdb (optional, for profilers)

  The installer registers .anicraft (AniCraft is its default app) and offers AniCraft under
  "Open with" for .fla and .xfl. The binaries link the C runtime statically (+crt-static), so
  neither the MSI nor the portable zip needs the Visual C++ redistributable. Signing is
  delegated to sign.ps1, which warns and leaves files unsigned when no signing secrets are set.

  Needs: Rust (MSVC toolchain + the target), the Windows SDK (rc.exe, signtool.exe),
  and WiX v5: dotnet tool install --global wix --version 5.0.2
             wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2

.EXAMPLE
  pwsh packaging/windows/package.ps1 -Arch x64
  pwsh packaging/windows/package.ps1 -Arch x64 -SkipBuild
  pwsh packaging/windows/package.ps1 -Arch arm64     # cross-compiled; needs the MSVC ARM64 build tools
#>
param(
  [ValidateSet('x64', 'x86', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$AppId = 'ai.storyteller.anicraft'

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
# ANICRAFT_VERSION (set by the shared release workflow from packaging/env.sh) overrides it.
$Version = $env:ANICRAFT_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
# MSI ProductVersion is numeric (major.minor.build); pre-release tags are dropped there.
$MsiVersion = ($Version -split '-')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'x86' { 'i686-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

Write-Output "AniCraft $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # Static CRT: no VC++ redistributable needed. Scoped to the target so host build scripts and
  # proc-macros are unaffected.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  # Fail the build (rather than warn) if build.rs can't embed the icon and version resource.
  $env:ANICRAFT_REQUIRE_WINRES = '1'
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p anicraft -p anicraft-cli --target $Target }
}

$Bin = Join-Path $TargetDir "$Target\release"

# Check both binaries' PE headers before packaging. Machine (COFF header) must match -Arch, so an
# x64 build can never ship labelled arm64. Subsystem (optional header): 2 = Windows GUI, 3 = console.
# The app must be GUI (no console window opens with it); the CLI must stay console so its output
# reaches the terminal.
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'x86' { 0x14C } 'arm64' { 0xAA64 } }
foreach ($check in @(@('anicraft.exe', 2), @('anicraft-cli.exe', 3))) {
  $h = Get-PeHeader (Join-Path $Bin $check[0])
  if ($h.Machine -ne $Machine) { throw "$($check[0]) is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
  if ($h.Subsystem -ne $check[1]) { throw "$($check[0]) has PE subsystem $($h.Subsystem), expected $($check[1])" }
  Write-Output "ok $($check[0]): $Arch, PE subsystem $($h.Subsystem)"
}
$Stage = Join-Path $TargetDir "windows-package\$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $Bin 'anicraft.exe'), (Join-Path $Bin 'anicraft-cli.exe') $Stage

& (Join-Path $PSScriptRoot 'sign.ps1') (Join-Path $Stage 'anicraft.exe') (Join-Path $Stage 'anicraft-cli.exe')

# ---- notices -----------------------------------------------------------------------------------
# Installed next to the app: our licences, the dependencies' notices and the OFL licences of the
# craft-fonts typefaces the build embedded (every official release builds with CRAFT_FONTS_DIR).
$Docs = Join-Path $Stage 'notices'
New-Item -ItemType Directory -Force -Path $Docs | Out-Null
foreach ($f in 'README.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'ATTRIBUTION.md') {
  Copy-Item -LiteralPath (Join-Path $Root $f) -Destination $Docs
}
if ($env:CRAFT_FONTS_DIR) {
  foreach ($ofl in Get-ChildItem -Path (Join-Path $env:CRAFT_FONTS_DIR 'fonts\*\OFL.txt') -File -ErrorAction SilentlyContinue) {
    Copy-Item -LiteralPath $ofl.FullName -Destination (Join-Path $Docs "OFL-$($ofl.Directory.Name).txt")
  }
}

# ---- MSI ---------------------------------------------------------------------------------------
# The setup wizard's art, drawn here from the app icon so the MSI shows AniCraft rather than
# WiX's stock bitmaps: a 493x58 banner across the top of the inner pages (the page title is drawn
# over its left side, so that stays white) and a 493x312 backdrop for the Welcome and Finish pages
# (their text sits on the right; the left 164 px are ours). Both must be 24-bit BMPs. The colours
# are the icon's (assets/app-icon/README.md): Paper and the AniCraft yellow.
function New-InstallerArt([string] $IconPng, [string] $BannerOut, [string] $DialogOut) {
  Add-Type -AssemblyName System.Drawing
  $paper = [System.Drawing.ColorTranslator]::FromHtml('#f2f1ee')
  $accent = [System.Drawing.ColorTranslator]::FromHtml('#dac604')
  $icon = [System.Drawing.Image]::FromFile($IconPng)
  try {
    foreach ($spec in @(@($BannerOut, 493, 58), @($DialogOut, 493, 312))) {
      $bmp = New-Object System.Drawing.Bitmap $spec[1], $spec[2], ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
      $g = [System.Drawing.Graphics]::FromImage($bmp)
      try {
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $g.Clear([System.Drawing.Color]::White)
        if ($spec[2] -eq 58) {
          $g.DrawImage($icon, 493 - 46 - 8, 6, 46, 46)
        } else {
          $panel = New-Object System.Drawing.SolidBrush $paper
          $stripe = New-Object System.Drawing.SolidBrush $accent
          $g.FillRectangle($panel, 0, 0, 164, 312)
          $g.FillRectangle($stripe, 160, 0, 4, 312)
          $g.DrawImage($icon, 18, 64, 124, 124)
          $panel.Dispose(); $stripe.Dispose()
        }
      } finally { $g.Dispose() }
      $bmp.Save($spec[0], [System.Drawing.Imaging.ImageFormat]::Bmp)
      $bmp.Dispose()
    }
  } finally { $icon.Dispose() }
}
$Banner = Join-Path $Stage 'installer-banner.bmp'
$Dialog = Join-Path $Stage 'installer-dialog.bmp'
New-InstallerArt (Join-Path $Root "assets\app-icon\hicolor\256x256\apps\$AppId.png") $Banner $Dialog

$Msi = Join-Path $Dist "anicraft-$Version-windows-$Arch.msi"
& (Join-Path $PSScriptRoot 'check-icons.ps1')
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'anicraft.wxs') -arch $Arch `
    -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
    -culture en-US -loc (Join-Path $PSScriptRoot 'anicraft.en-us.wxl') `
    -d "Version=$MsiVersion" -d "DisplayVersion=$Version" -d "BinDir=$Stage" -d "IconPath=$(Join-Path $Root 'assets\app-icon\anicraft.ico')" `
    -d "BannerBmp=$Banner" -d "DialogBmp=$Dialog" -d "DocsDir=$Docs" `
    -o $Msi
}
Invoke-Native 'MSI shortcut icon validation (ICE50)' {
  wix msi validate $Msi -ice ICE50 -intermediateFolder (Join-Path $Stage 'msi-validation')
}
# Inspect the compiled tables and evaluate the scope and duplicate guards with Windows Installer.
# ICE validation alone cannot catch a well-formed condition with its meaning reversed.
& (Join-Path $PSScriptRoot 'check-install-policy.ps1') -MsiPath $Msi
# wix writes its debug symbols (.wixpdb) next to the MSI; keep them out of the release assets.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
& (Join-Path $PSScriptRoot 'sign.ps1') $Msi

# ---- portable zip ------------------------------------------------------------------------------
# The exes and notices at the archive's root (`<folder>\*`, not the folder itself), so they come
# out side by side wherever it is extracted. Settings still live in %APPDATA%\AniCraft.
$Portable = Join-Path $TargetDir "windows-package\anicraft-$Version-windows-$Arch-portable"
Remove-Item -Recurse -Force $Portable -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Portable | Out-Null
Copy-Item (Join-Path $Stage '*.exe') $Portable
Copy-Item (Join-Path $Docs '*') $Portable
$Zip = Join-Path $Dist "anicraft-$Version-windows-$Arch-portable.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $Portable '*') -DestinationPath $Zip
$Check = Join-Path $TargetDir 'windows-package\portable-zip-check'
Remove-Item -Recurse -Force $Check -ErrorAction SilentlyContinue
Expand-Archive -Path $Zip -DestinationPath $Check -Force
foreach ($f in 'anicraft.exe', 'anicraft-cli.exe', 'NOTICE') {
  if (-not (Test-Path (Join-Path $Check $f))) { throw "the portable zip has no $f at its root: $Zip" }
}
Remove-Item -Recurse -Force $Check -ErrorAction SilentlyContinue

# ---- debug symbols -----------------------------------------------------------------------------
# The MSVC linker writes a PDB beside each exe (anicraft.pdb, anicraft_cli.pdb). They ship as a
# separate, optional download so profilers show AniCraft's function names for the released
# binaries. Signing doesn't change the PDB signature an exe records, so they match the signed exes.
$Outputs = @($Msi, $Zip)
$Pdbs = @('anicraft.pdb', 'anicraft_cli.pdb' | ForEach-Object { Join-Path $Bin $_ })
if (@($Pdbs | Where-Object { -not (Test-Path $_) }).Count -eq 0) {
  $Symbols = Join-Path $TargetDir "windows-package\anicraft-$Version-windows-$Arch-symbols"
  Remove-Item -Recurse -Force $Symbols -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force -Path $Symbols | Out-Null
  Copy-Item $Pdbs $Symbols
  Copy-Item (Join-Path $PSScriptRoot 'symbols.txt') $Symbols
  $SymbolsZip = Join-Path $Dist "anicraft-$Version-windows-$Arch-symbols.zip"
  Remove-Item -Force $SymbolsZip -ErrorAction SilentlyContinue
  Compress-Archive -Path (Join-Path $Symbols '*') -DestinationPath $SymbolsZip
  $Outputs += $SymbolsZip
} elseif ($env:GITHUB_ACTIONS) {
  Write-Output "::warning::no PDBs in $Bin, so there is no symbols zip"
} else {
  Write-Warning "no PDBs in $Bin, so there is no symbols zip"
}

# Smoke-test the CLI when this machine can run it. An ARM64 build made on an x64 runner can't run
# here; .github/workflows/windows-arm64.yml installs and runs it on ARM64 instead.
$HostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($Arch -ne 'arm64' -or $HostArch -eq 'arm64') {
  Invoke-Native 'anicraft-cli --version' { & (Join-Path $Stage 'anicraft-cli.exe') --version }
} else {
  Write-Output "skipping anicraft-cli --version: an $Arch build doesn't run on this $HostArch machine"
}
Get-Item $Outputs | Format-Table Name, Length
