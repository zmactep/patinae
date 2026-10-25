# Windows distribution

Patinae ships a full installer and a portable ZIP for each of x86_64 and ARM64.
Both contain the same application, native launcher, hello/raytracer/python/ai
plugins, Python 3.13, Patinae wheel (including widget assets), icon and licenses.
IPC is not built for Windows. Setup and the application binaries are unsigned.

## Build

Use a native Windows MSVC Rust toolchain matching the desired architecture,
GNU Make, uv, the web build prerequisites from the root README, and
[Inno Setup 7.1.0](https://jrsoftware.org/). CI pins the compiler and
checks its publisher signature. Build from the repository root:

```powershell
uv python install 3.13
# On ARM64: uv python install cpython-3.13-windows-aarch64-none
uv tool install maturin
make bundle-windows
$env:ISCC = 'C:\Program Files\Inno Setup 7\ISCC.exe'
make installer-windows WINDOWS_ARCH=x86_64
# On ARM64: make installer-windows WINDOWS_ARCH=arm64
```

`installer-windows` packages the existing `target/bundle/Patinae` without
rebuilding it. The architecture argument is required and checked against PE
headers. Version comes from the workspace manifest; existing release-metadata
checks keep it aligned with the wheel and Makefile. Output is
`target/installers/patinae-setup-windows-<arch>.exe`.

Only one matching-version wheel should be present in `python/target/wheels`.
Clean that output directory when switching architecture on the same checkout.

The copied Python runtime has the wheel installed directly into its own
`Lib/site-packages`. There is no venv or dependency on the builder's interpreter.
The wheel's staging-path console wrapper is omitted. Use `Patinae-launcher.exe`
for the desktop application or `patinae.exe` for direct CLI use. The launcher
configures the child process's Python paths, suppresses its console and forwards
Unicode arguments with native process quoting. It preserves the caller's working
directory and does not modify the system PATH or user Python configuration.

## Installation and upgrades

Setup installs to `Program Files\Patinae` with elevation, creates a common Start
menu shortcut, and offers an optional common desktop shortcut. It registers
optional Open with entries without writing extension defaults or UserChoice.
The fixed AppId must remain unchanged across releases. Run the new installer to
upgrade; Setup retains the installation directory and selected tasks. It asks
before closing applications with open files and does not force termination.

Ordinary launches do not require elevation. Do not add packages to the shared
Python runtime or put user plugins in Program Files: it is installer-owned.
Keep user plugins and settings in the existing user profile directory. Removing
Patinae removes installer-owned files, shortcuts and associations, while
preserving profile data and documents. There is no automatic updater or MSI.

Silent deployment from an elevated shell:

```powershell
.\patinae-setup-windows-x86_64.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCLOSEAPPLICATIONS
# Explicit optional tasks: /TASKS="desktopicon,fileassociations"
```

## Verification

Release CI runs on native x64 and ARM64 Windows runners. It tests launcher
argument/environment construction, copies the full bundle to a new Unicode path
with spaces and a clean child environment, imports the wheel, and loads all four
plugin DLLs. It also launches the copied interpreter as a stand-in for the GUI,
checking actual argument forwarding, runtime discovery and console suppression.
This intentionally invokes the distributed Python directly, so uv
or a build interpreter cannot mask missing runtime dependencies.

```powershell
./windows/test-bundle.ps1 -Arch x86_64
# Destructive lifecycle test: use ONLY an elevated disposable Windows VM/runner.
./windows/test-installer.ps1 -Arch x86_64
```

The lifecycle test refuses to run over an existing installation. It checks
opt-in defaults, shortcuts, associations, installs a synthetic version 0.0.0,
upgrades to the current version, reinstalls it, then uninstalls. It verifies
preservation of extension defaults, foreign Open with entries and profile data.
Installer logs are retained in `target/installer-test-logs` and uploaded by CI.
Any test failure blocks the existing release dependency chain.

Before publishing, perform these desktop acceptance checks on clean Windows
machines for both architectures (CI DLL loading does not test GUI registration):

- Install as administrator, then launch as a different standard user with no
  Python installed; verify GUI startup, all four plugins, and Python commands.
- Open `.prs`, `.pdb`, `.cif`, and `.sdf` using Explorer's Open with menu, including
  filenames with spaces and Cyrillic characters; ensure no console window opens.
- Upgrade while Patinae has unsaved work: verify the close prompt, cancel the
  upgrade, save work and retry. Check that the application is never force-killed.
- Uninstall and confirm documents, settings and user plugins remain accessible.

GitHub publication and release dispatch are separate actions, not part of a
local build.
