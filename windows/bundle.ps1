param(
    [Parameter(Mandatory)][string]$PythonDist,
    [Parameter(Mandatory)][string]$Version,
    [string]$BundleDir = 'target/bundle/Patinae',
    [string]$ReleaseDir = 'target/release',
    [string]$Uv = 'uv'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$wheels = @(Get-ChildItem "python/target/wheels/patinae-$Version-*.whl")
if ($wheels.Count -ne 1) { throw "Expected one wheel for $Version; clean python/target/wheels before building." }
$plugins = @('hello_plugin.dll', 'raytracer_plugin.dll', 'python_plugin.dll', 'ai_plugin.dll')
foreach ($file in @('patinae.exe', 'Patinae-launcher.exe')) {
    if (!(Test-Path "$ReleaseDir/$file")) { throw "Missing executable: $file" }
}
foreach ($file in $plugins) {
    if (!(Test-Path "$ReleaseDir/plugins/$file")) { throw "Missing plugin: $file" }
}
if (!(Test-Path "$PythonDist/python.exe")) { throw 'Missing Python runtime' }
if (Test-Path $BundleDir) { Remove-Item $BundleDir -Recurse -Force }
$null = New-Item "$BundleDir/plugins" -ItemType Directory -Force
Copy-Item "$ReleaseDir/patinae.exe", "$ReleaseDir/Patinae-launcher.exe" $BundleDir
foreach ($file in $plugins) { Copy-Item "$ReleaseDir/plugins/$file" "$BundleDir/plugins/" }
Copy-Item 'plugins/python/python_plugin.deps' "$BundleDir/plugins/"
Copy-Item $PythonDist "$BundleDir/python" -Recurse
Copy-Item 'images/patinae.ico', 'LICENSE' $BundleDir
# Install into the copied runtime, never into the uv-managed build interpreter.
# The desktop launcher uses python.exe and site-packages directly, not a venv.
& $Uv pip install --python "$BundleDir/python/python.exe" --no-index --no-deps --link-mode copy $wheels[0].FullName
if ($LASTEXITCODE -ne 0) { throw 'Installing the Patinae wheel failed' }
# Wheel console-script wrappers contain the staging interpreter's absolute path.
# The bundle's supported entry points are Patinae-launcher.exe and patinae.exe.
Remove-Item "$BundleDir/python/Scripts/patinae.exe" -ErrorAction SilentlyContinue
Write-Host "Bundle created: $BundleDir"
