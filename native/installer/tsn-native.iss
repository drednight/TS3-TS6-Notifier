; Inno Setup script for the native TS3-TS6 Notifier.
; Build (after native/tools/package-win.ps1):
;   iscc native/installer/tsn-native.iss
; Optional: /DAppVersion=x.y.z to override the version.

#ifndef StageDir
  #define StageDir "..\..\dist-win\tsn-native"
#endif
#ifndef AppVersion
  #define AppVersion "2.0.13"
#endif

[Setup]
AppName=TS3-TS6 Notifier
AppVersion={#AppVersion}
AppPublisher=TS3-TS6 Notifier
DefaultDirName={autopf}\TS3-TS6 Notifier
DefaultGroupName=TS3-TS6 Notifier
PrivilegesRequired=admin
OutputDir=..\..\dist-win
OutputBaseFilename=TS3-TS6-Notifier-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\TS3-TS6-Notifier-Overlay.exe

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{autoprograms}\TS3-TS6 Notifier"; Filename: "{app}\TS3-TS6-Notifier-Overlay.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\TS3-TS6 Notifier"; Filename: "{app}\TS3-TS6-Notifier-Overlay.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\TS3-TS6-Notifier-Overlay.exe"; Description: "Запустить TS3-TS6 Notifier"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent
