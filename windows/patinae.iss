; Build through build-installer.ps1 so version and architecture are explicit.
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef AppArch
  #error AppArch is required
#endif
#ifndef BundleDir
  #error BundleDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#if AppArch == "x86_64"
  #define AllowedArchitecture "x64os"
#elif AppArch == "arm64"
  #define AllowedArchitecture "arm64"
#else
  #error Unsupported architecture
#endif

[Setup]
; Permanent identity shared by all releases. Do not derive it from the version.
AppId={{DCB97EB3-F531-4AE6-BA3D-C0F95BEE7501}
AppName=Patinae
AppVersion={#AppVersion}
AppPublisher=Pavel Yakovlev
AppPublisherURL=https://github.com/zmactep/patinae
AppSupportURL=https://github.com/zmactep/patinae/issues
DefaultDirName={autopf}\Patinae
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed={#AllowedArchitecture}
ArchitecturesInstallIn64BitMode={#AllowedArchitecture}
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=patinae-setup-windows-{#AppArch}
SetupIconFile={#BundleDir}\patinae.ico
UninstallDisplayIcon={app}\patinae.ico
LicenseFile={#BundleDir}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UsePreviousAppDir=yes
UsePreviousTasks=yes
CloseApplications=yes
CloseApplicationsFilter=*.exe,*.dll,*.pyd
RestartApplications=no
ChangesAssociations=yes
Uninstallable=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
english.DesktopShortcut=Create a desktop shortcut
russian.DesktopShortcut=Создать ярлык на рабочем столе
english.FileAssociations=Add Patinae to Open with for .prs, .pdb, .cif and .sdf files
russian.FileAssociations=Добавить Patinae в «Открыть с помощью» для .prs, .pdb, .cif и .sdf

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopShortcut}"; Flags: unchecked
Name: "fileassociations"; Description: "{cm:FileAssociations}"; Flags: unchecked

[Files]
Source: "{#BundleDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{commonprograms}\Patinae"; Filename: "{app}\Patinae-launcher.exe"; IconFilename: "{app}\patinae.ico"; WorkingDir: "{app}"
Name: "{commondesktop}\Patinae"; Filename: "{app}\Patinae-launcher.exe"; IconFilename: "{app}\patinae.ico"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
; Only our ProgIDs and OpenWith values are owned. Never write extension defaults
; or Explorer's per-user UserChoice keys.
#sub RegisterExtension
Root: HKLM; Subkey: "Software\Classes\Patinae.{#Extension}"; ValueType: string; ValueData: "Patinae .{#Extension} file"; Flags: uninsdeletekey; Tasks: fileassociations
Root: HKLM; Subkey: "Software\Classes\Patinae.{#Extension}\DefaultIcon"; ValueType: string; ValueData: """{app}\patinae.ico"""; Tasks: fileassociations
Root: HKLM; Subkey: "Software\Classes\Patinae.{#Extension}\shell\open\command"; ValueType: string; ValueData: """{app}\Patinae-launcher.exe"" ""%1"""; Tasks: fileassociations
Root: HKLM; Subkey: "Software\Classes\.{#Extension}\OpenWithProgids"; ValueType: string; ValueName: "Patinae.{#Extension}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty; Tasks: fileassociations
#endsub
#define Extension "prs"
#expr RegisterExtension
#define Extension "pdb"
#expr RegisterExtension
#define Extension "cif"
#expr RegisterExtension
#define Extension "sdf"
#expr RegisterExtension
