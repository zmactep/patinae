param(
    [Parameter(Mandatory)][ValidateSet('x86_64', 'arm64')][string]$Arch,
    [string]$InstallerDir = 'target/installers',
    [string]$BundleDir = 'target/bundle/Patinae'
)
# Run only on a disposable elevated Windows runner. Refuse an existing install.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$appKey = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{DCB97EB3-F531-4AE6-BA3D-C0F95BEE7501}_is1'
if (Test-Path $appKey) { throw 'Refusing to test over an existing Patinae installation' }
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Installer tests require an elevated disposable Windows runner' }
$installer = (Resolve-Path "$InstallerDir/patinae-setup-windows-$Arch.exe").Path
$work = Join-Path ([IO.Path]::GetTempPath()) ('Patinae installer ' + [guid]::NewGuid())
$installDir = Join-Path $work 'Программа с пробелами'
$null = New-Item $work -ItemType Directory -Force
$extensions = @('prs', 'pdb', 'cif', 'sdf')
# Preserve all pre-existing extension keys; use a uniquely named foreign entry.
$foreign = 'PatinaeInstallerTest.' + [guid]::NewGuid().ToString('N')
$originalDefaults = @{}
$createdKeys = @()
$profileSentinel = Join-Path $env:USERPROFILE ('.patinae/installer-test-' + [guid]::NewGuid())

function Run-Setup([string]$Path, [string[]]$Arguments) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Path
    $start.Arguments = $Arguments -join ' '
    $start.UseShellExecute = $false
    $process = [Diagnostics.Process]::Start($start)
    if (!$process.WaitForExit(300000)) { $process.Kill(); throw "Installer timed out: $Path" }
    if ($process.ExitCode -ne 0) { throw "Installer exited with $($process.ExitCode): $Path" }
}
function Assert-Installed([string]$ExpectedVersion) {
    if (!(Test-Path "$installDir/Patinae-launcher.exe")) { throw 'Missing launcher' }
    if ((Get-ItemProperty $appKey).DisplayVersion -ne $ExpectedVersion) { throw 'Wrong installed version' }
    if (!(Test-Path "$env:ProgramData/Microsoft/Windows/Start Menu/Programs/Patinae.lnk")) { throw 'Missing Start menu shortcut' }
    if (!(Test-Path "$env:PUBLIC/Desktop/Patinae.lnk")) { throw 'Missing desktop shortcut' }
    foreach ($extension in $extensions) {
        $key = "HKLM:\Software\Classes\.$extension"
        if ((Get-Item $key).GetValue('') -ne $originalDefaults[$extension]) { throw "Changed default for .$extension" }
        $openWith = Get-Item "$key/OpenWithProgids"
        if ($openWith.GetValueNames() -notcontains "Patinae.$extension") { throw "Missing OpenWith registration: $extension" }
        if ($openWith.GetValue($foreign) -ne 'preserve') { throw 'Foreign registration was changed' }
        $command = (Get-Item "HKLM:\Software\Classes\Patinae.$extension/shell/open/command").GetValue('')
        if ($command -ne ('"' + $installDir + '\Patinae-launcher.exe" "%1"')) { throw 'Incorrect association quoting' }
    }
    if ((Get-Content $profileSentinel -Raw) -notmatch 'preserve') { throw 'User data was changed' }
}

try {
    $null = New-Item (Split-Path $profileSentinel) -ItemType Directory -Force
    Set-Content $profileSentinel 'preserve'
    foreach ($extension in $extensions) {
        $key = "HKLM:\Software\Classes\.$extension"
        if (!(Test-Path $key)) { $null = New-Item $key -Force; $createdKeys += $key }
        $originalDefaults[$extension] = (Get-Item $key).GetValue('')
        $null = New-Item "$key/OpenWithProgids" -Force
        $null = New-ItemProperty "$key/OpenWithProgids" -Name $foreign -Value 'preserve' -PropertyType String
    }
    # First verify default optional tasks are off, then uninstall cleanly.
    $baseArgs = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOCLOSEAPPLICATIONS', "/DIR=`"$installDir`"")
    Run-Setup $installer ($baseArgs + @('/TASKS=""', "/LOG=`"$work/default-install.log`""))
    if (Test-Path "$env:PUBLIC/Desktop/Patinae.lnk") { throw 'Desktop shortcut should be opt-in' }
    foreach ($extension in $extensions) {
        if ((Get-Item "HKLM:\Software\Classes\.$extension/OpenWithProgids").GetValueNames() -contains "Patinae.$extension") { throw 'File association should be opt-in' }
    }
    Run-Setup "$installDir/unins000.exe" @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')

    & "$PSScriptRoot/build-installer.ps1" -Arch $Arch -BundleDir $BundleDir -OutputDir "$work/previous" -AppVersion '0.0.0'
    Run-Setup "$work/previous/patinae-setup-windows-$Arch.exe" ($baseArgs + @('/TASKS="desktopicon,fileassociations"'))
    Assert-Installed '0.0.0'
    # Upgrade and reinstall, with tasks restored from the previous installation.
    $manifest = Get-Content "$PSScriptRoot/../Cargo.toml" -Raw
    $version = [regex]::Match($manifest, '(?ms)^\[workspace\.package\]\s*\r?\nversion = "([^"]+)"').Groups[1].Value
    foreach ($phase in @('upgrade', 'reinstall')) {
        Run-Setup $installer ($baseArgs + @("/LOG=`"$work/$phase.log`""))
        Assert-Installed $version
    }
    & "$PSScriptRoot/test-bundle.ps1" -Arch $Arch -BundleDir $installDir
    Run-Setup "$installDir/unins000.exe" @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
    foreach ($file in @('patinae.exe', 'Patinae-launcher.exe', 'python/python.exe')) {
        if (Test-Path "$installDir/$file") { throw "Uninstall left $file" }
    }
    if (Test-Path $appKey) { throw 'Uninstall left its registry entry' }
    foreach ($shortcut in @("$env:PUBLIC/Desktop/Patinae.lnk", "$env:ProgramData/Microsoft/Windows/Start Menu/Programs/Patinae.lnk")) {
        if (Test-Path $shortcut) { throw 'Uninstall left a shortcut' }
    }
    foreach ($extension in $extensions) {
        $key = "HKLM:\Software\Classes\.$extension"
        if ((Get-Item $key).GetValue('') -ne $originalDefaults[$extension]) { throw 'Uninstall changed a default' }
        $openWith = Get-Item "$key/OpenWithProgids"
        if ($openWith.GetValueNames() -contains "Patinae.$extension") { throw 'Uninstall left an association' }
        if ($openWith.GetValue($foreign) -ne 'preserve') { throw 'Uninstall removed a foreign registration' }
        if (Test-Path "HKLM:\Software\Classes\Patinae.$extension") { throw 'Uninstall left a ProgID' }
    }
    if (!(Test-Path $profileSentinel)) { throw 'Uninstall removed user data' }
    Write-Host 'Install, opt-in tasks, upgrade, reinstall and uninstall checks passed.'
} finally {
    # Keep test logs as CI artifacts, even when validation fails.
    $null = New-Item 'target/installer-test-logs' -ItemType Directory -Force
    Get-ChildItem $work -Filter '*.log' | Copy-Item -Destination 'target/installer-test-logs/'
    if (Test-Path "$installDir/unins000.exe") {
        Run-Setup "$installDir/unins000.exe" @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
    }
    foreach ($extension in $extensions) {
        Remove-ItemProperty "HKLM:\Software\Classes\.$extension/OpenWithProgids" -Name $foreign -ErrorAction SilentlyContinue
    }
    foreach ($key in $createdKeys) { Remove-Item $key -Recurse -Force }
    Remove-Item $profileSentinel -ErrorAction SilentlyContinue
    Remove-Item $work -Recurse -Force
}
