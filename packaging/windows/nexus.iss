; Nexus's Windows installer, with Inno Setup 6 (https://jrsoftware.org/isinfo.php), from the
; Windows folder scripts/package.sh made. Run from the repository root:
;   iscc /DAppVersion=0.1.0 /DFolder=dist\nexus-windows-x86_64 /DOutputDir=dist packaging\windows\nexus.iss
; It installs for the current user only, into their local programs folder, so neither installing
; nor updating needs an administrator. It adds a Start menu entry, an optional desktop shortcut,
; and an uninstaller in Windows' installed apps. The uninstaller removes the program folder only:
; settings, plugins' data and emulators stay in the data folder.
;
; A marker file, installed-by-setup, tells Nexus it was installed this way: its update then runs
; the next release's installer silently (/VERYSILENT), and the [Run] entry starts the new version.

#ifndef AppVersion
  #error Pass the version: /DAppVersion=0.1.0
#endif
#ifndef Folder
  #define Folder "dist\nexus-windows-x86_64"
#endif
#ifndef OutputDir
  #define OutputDir "dist"
#endif

[Setup]
; Never change AppId: Windows tells an update from a second install by it.
AppId={{7C8A92C7-DBE5-40DC-A375-F6E74CCBE879}
AppName=Nexus
AppVersion={#AppVersion}
AppVerName=Nexus {#AppVersion}
AppPublisher=Nexus
AppPublisherURL=https://github.com/mohamed-rekiba/nexus
AppSupportURL=https://github.com/mohamed-rekiba/nexus/issues
AppUpdatesURL=https://github.com/mohamed-rekiba/nexus/releases
DefaultDirName={localappdata}\Programs\Nexus
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=nexus-windows-x86_64-setup
SetupIconFile=..\..\assets\nexus.ico
UninstallDisplayIcon={app}\nexus.exe
UninstallDisplayName=Nexus
LicenseFile=..\..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Nexus closes before its files are replaced, and the update starts it again ([Run]).
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\{#Folder}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Dirs]
; Third-party plugins and themes, found next to nexus.exe.
Name: "{app}\extensions"

[Icons]
Name: "{autoprograms}\Nexus"; Filename: "{app}\nexus.exe"
Name: "{autodesktop}\Nexus"; Filename: "{app}\nexus.exe"; Tasks: desktopicon

[Run]
; Offered after an install; after a silent update it runs by itself, starting the new version.
Filename: "{app}\nexus.exe"; Description: "{cm:LaunchProgram,Nexus}"; Flags: nowait postinstall

[UninstallDelete]
Type: files; Name: "{app}\installed-by-setup"
Type: dirifempty; Name: "{app}\extensions"
Type: dirifempty; Name: "{app}"

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    SaveStringToFile(ExpandConstant('{app}\installed-by-setup'), 'Windows installer', False);
end;
