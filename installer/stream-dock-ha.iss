; Inno Setup script for Stream Dock HA. make-setup.ps1 builds the program and
; then compiles this with:
;   ISCC.exe /DAppVersion=<version> installer\stream-dock-ha.iss
;
; It installs for the current Windows user only, so it never asks for
; administrator rights, into the folder the app's own installer used, so it
; updates an existing copy in place and keeps its settings.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppName "Stream Dock HA"
#define AppExe "Stream Dock HA.exe"
; The app's tray window and its "please quit" message (src/tray.rs).
#define TrayClass "TrayDockTrayWindow"
#define QuitRequest "32772"
; What the app's own installer wrote, before this one (src/setup.rs, src/tray.rs).
#define OldUninstallKey "Software\Microsoft\Windows\CurrentVersion\Uninstall\StreamDockHA"
#define RunValue "StreamDockHA"

[Setup]
AppId={{B6F3A1D2-7C4E-4F8A-9E21-5D0C3B8A6E47}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppName}
AppPublisherURL=https://github.com/Max-src/stream-dock-ha
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
UsePreviousTasks=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename={#AppName} Setup {#AppVersion}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; The app is asked to quit in [Code], as it would quit from its tray menu.
CloseApplications=no
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "Start {#AppName} with Windows, near the clock"; GroupDescription: "Startup:"; Flags: unchecked
; Mini Monitor needs Parsec's free display driver, which isn't ours to ship: the
; app downloads Parsec's own installer, checks Parsec's signature and runs it
; (driver.rs). Windows asks for administrator rights for that step only. When
; the driver is already there, the step shows ticked and greyed out, and is skipped.
Name: "driver"; Description: "Mini Monitor's display driver (Parsec's free Virtual Display Driver, downloaded from Parsec, if it isn't installed yet)"; GroupDescription: "Mini Monitor:"

[Files]
Source: "..\target\setup-build\release\stream-dock-ha.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Parameters: "--show"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Parameters: "--show"; Tasks: desktopicon

[Registry]
; The app's own installer had its own entry in Windows' Apps list: this one replaces it.
Root: HKCU; Subkey: "{#OldUninstallKey}"; ValueType: none; Flags: deletekey
; Starting with Windows, as the tray menu's "Start with Windows" sets it.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#RunValue}"; ValueData: """{app}\{#AppExe}"" --hidden"; Tasks: autostart
; Removing the app takes it out of start-up, whoever turned that on.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "{#RunValue}"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Parameters: "--install-driver"; StatusMsg: "Installing Mini Monitor's display driver..."; Flags: waituntilterminated; Tasks: driver; Check: not DriverWasThere
Filename: "{app}\{#AppExe}"; Parameters: "--show"; Description: "Open {#AppName} now"; Flags: nowait postinstall skipifsilent

[Code]
{ Asks a running copy to quit as its tray menu does, so its settings are saved
  and its pads let go, and waits for it. A copy that doesn't answer is stopped. }
procedure CloseRunningApp();
var
  Wnd: HWND;
  Waited, ResultCode: Integer;
begin
  Wnd := FindWindowByClassName('{#TrayClass}');
  if Wnd = 0 then
    Exit;
  PostMessage(Wnd, {#QuitRequest}, 0, 0);
  Waited := 0;
  while (FindWindowByClassName('{#TrayClass}') <> 0) and (Waited < 10000) do
  begin
    Sleep(250);
    Waited := Waited + 250;
  end;
  if FindWindowByClassName('{#TrayClass}') <> 0 then
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM "{#AppExe}" /F', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  { The program's file is let go a moment after its window. }
  Sleep(500);
end;

var
  DriverChecked, DriverThere: Boolean;

{ Whether Mini Monitor's driver is already on this PC, by the app's own check:
  the program in this setup answers --driver-present with 0 when it is. Asked
  once, before the options are shown. }
function DriverWasThere(): Boolean;
var
  ResultCode: Integer;
begin
  if not DriverChecked then
  begin
    DriverChecked := True;
    DriverThere := False;
    try
      { Inno finds and writes the file by the name it installs under. }
      ExtractTemporaryFile('{#AppExe}');
      if Exec(ExpandConstant('{tmp}\{#AppExe}'), '--driver-present', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
        DriverThere := ResultCode = 0;
    except
      DriverThere := False;
    end;
  end;
  Result := DriverThere;
end;

{ With the driver already there, its step shows as done: ticked, greyed out. }
procedure CurPageChanged(CurPageID: Integer);
var
  I: Integer;
begin
  if (CurPageID = wpSelectTasks) and DriverWasThere() then
    for I := 0 to WizardForm.TasksList.Items.Count - 1 do
      if Pos('display driver', WizardForm.TasksList.ItemCaption[I]) > 0 then
      begin
        WizardForm.TasksList.ItemCaption[I] := 'Mini Monitor''s display driver (already installed)';
        WizardForm.TasksList.Checked[I] := True;
        WizardForm.TasksList.ItemEnabled[I] := False;
      end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseRunningApp();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  CloseRunningApp();
  Result := True;
end;
