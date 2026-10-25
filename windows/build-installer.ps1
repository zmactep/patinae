param(
    [Parameter(Mandatory)][ValidateSet('x86_64', 'arm64')][string]$Arch,
    [string]$BundleDir = 'target/bundle/Patinae',
    [string]$OutputDir = 'target/installers',
    # Version overrides are used only to build the upgrade-test fixture.
    [string]$AppVersion,
    [string]$Iscc = $env:ISCC
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$AppVersion) {
    $manifest = Get-Content "$PSScriptRoot/../Cargo.toml" -Raw
    $match = [regex]::Match($manifest, '(?ms)^\[workspace\.package\]\s*\r?\nversion = "([^"]+)"')
    if (!$match.Success) { throw 'Cannot read workspace version' }
    $AppVersion = $match.Groups[1].Value
}
if ($AppVersion -notmatch '^\d+\.\d+\.\d+$') { throw "Invalid installer version: $AppVersion" }
if (!$Iscc) { $Iscc = 'ISCC.exe' }
$bundle = (Resolve-Path $BundleDir).Path
foreach ($file in @('patinae.exe', 'Patinae-launcher.exe', 'patinae.ico', 'LICENSE', 'python/python.exe')) {
    if (!(Test-Path "$bundle/$file")) { throw "Incomplete bundle: missing $file" }
}
# Prevent a mislabeled release: validate the PE machine of both entry points.
$expectedMachine = if ($Arch -eq 'arm64') { 0xaa64 } else { 0x8664 }
foreach ($file in @('patinae.exe', 'Patinae-launcher.exe', 'python/python.exe')) {
    $bytes = [IO.File]::ReadAllBytes("$bundle/$file")
    $offset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ([BitConverter]::ToUInt16($bytes, $offset + 4) -ne $expectedMachine) {
        throw "$file does not match installer architecture $Arch"
    }
}
$null = New-Item $OutputDir -ItemType Directory -Force
$output = (Resolve-Path $OutputDir).Path
& $Iscc "/DAppVersion=$AppVersion" "/DAppArch=$Arch" "/DBundleDir=$bundle" "/DOutputDir=$output" "$PSScriptRoot/patinae.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE" }
if (!(Test-Path "$output/patinae-setup-windows-$Arch.exe")) { throw 'Installer output is missing' }
