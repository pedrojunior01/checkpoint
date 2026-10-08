; Inno Setup Script para o instalador oficial do checkpoint (SaveVault)
; Gera um instalador Windows nativo, limpo, ultra-leve e sem dependências externas.

#define MyAppName "checkpoint"
#define MyAppVersion "0.9.0"
#define MyAppPublisher "SaveVault Team"
#define MyAppURL "https://github.com/mathe/Savevalt"
#define MyAppExeName "savevault.exe"

[Setup]
AppId={{9C52E4B1-3E49-43E2-8119-DE36F4932F1A}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=..\README.md
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist_installer
OutputBaseFilename=checkpoint-setup-v{#MyAppVersion}
SetupIconFile=..\crates\savevault-app\icons\icon.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startupicon"; Description: "Iniciar o checkpoint automaticamente com o Windows (em segundo plano)"; GroupDescription: "Inicialização:"; Flags: checkedonce

[Files]
Source: "..\C:\svt\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion; DestName: "{#MyAppExeName}"
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\crates\savevault-app\icons\icon.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#MyAppName}"; ValueData: """{app}\{#MyAppExeName}"" --tray"; Flags: uninsdeletevalue; Tasks: startupicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

