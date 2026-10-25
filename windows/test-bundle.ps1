param(
    [Parameter(Mandatory)][ValidateSet('x86_64', 'arm64')][string]$Arch,
    [string]$BundleDir = 'target/bundle/Patinae'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$source = (Resolve-Path $BundleDir).Path
$work = Join-Path ([IO.Path]::GetTempPath()) ('Patinae relocated ' + [guid]::NewGuid())
$relocated = Join-Path $work 'Патина с пробелами'
$null = New-Item $work -ItemType Directory -Force
Copy-Item $source $relocated -Recurse
$manifest = Get-Content "$PSScriptRoot/../Cargo.toml" -Raw
$version = [regex]::Match($manifest, '(?ms)^\[workspace\.package\]\s*\r?\nversion = "([^"]+)"').Groups[1].Value
try {
    # A clean child environment prevents a build-machine Python from rescuing a
    # broken bundle. Direct execution is intentional: the product needs no uv.
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = "$relocated/python/python.exe"
    $start.Arguments = '-I -B "' + "$PSScriptRoot/check-bundle.py" + '" "' + $relocated + '" ' + $Arch + ' ' + $version
    $start.UseShellExecute = $false
    $start.WorkingDirectory = $work
    foreach ($name in @('PYTHONHOME', 'PYTHONPATH', 'VIRTUAL_ENV', 'PYO3_PYTHON')) {
        $start.EnvironmentVariables.Remove($name)
    }
    $start.EnvironmentVariables['PATH'] = "$relocated/python;$env:SystemRoot/System32;$env:SystemRoot"
    $process = [Diagnostics.Process]::Start($start)
    if (!$process.WaitForExit(120000)) { $process.Kill(); throw 'Bundle check timed out' }
    if ($process.ExitCode -ne 0) { throw "Bundle check failed: $($process.ExitCode)" }

    # Exercise the real launcher without needing a graphics adapter. Only the
    # temporary copy is changed. The child must find its DLL and standard library
    # via the launcher's environment, forward Unicode paths and have no console.
    Copy-Item "$relocated/python/python.exe" "$relocated/patinae.exe" -Force
    $marker = Join-Path $work 'Результат запуска.txt'
    $start.FileName = "$relocated/Patinae-launcher.exe"
    $start.Arguments = '-P -B "' + "$PSScriptRoot/check-bundle.py" + '" "' + $relocated + '" ' + $Arch + ' ' + $version + ' "' + $marker + '"'
    $process = [Diagnostics.Process]::Start($start)
    if (!$process.WaitForExit(30000)) { $process.Kill(); throw 'Launcher timed out' }
    if ($process.ExitCode -ne 0) { throw 'Launcher failed' }
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (!(Test-Path $marker) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    if (!(Test-Path $marker)) { throw 'Launcher child check failed' }
    $result = Get-Content $marker -Raw
    if ($result -notmatch '^passed:(\d+)$') { throw 'Invalid launcher check result' }
    $child = Get-Process -Id ([int]$Matches[1]) -ErrorAction SilentlyContinue
    if ($child -and !$child.WaitForExit(30000)) { $child.Kill(); throw 'Launcher child did not exit' }
} finally {
    Remove-Item $work -Recurse -Force
}
