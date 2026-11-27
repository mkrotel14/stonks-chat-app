[Setup]
AppId=com.rdxlabs.stonks.gpui
AppName=Stonks Chat (GPUI)
AppVersion={#BuildVersion}
DefaultDirName={localappdata}\Programs\Stonks GPUI
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#ArtifactDir}
OutputBaseFilename=Stonks.GPUI_{#BuildVersion}_x64-setup
UninstallDisplayIcon={app}\stonks-app.exe

[Files]
Source: "{#ProjectLicense}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Binary}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Notices}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MediaDir}\ffmpeg.exe"; DestDir: "{app}\resources\media"; Flags: ignoreversion
Source: "{#MediaDir}\ffprobe.exe"; DestDir: "{app}\resources\media"; Flags: ignoreversion
Source: "{#MediaDir}\yt-dlp.exe"; DestDir: "{app}\resources\media"; Flags: ignoreversion
Source: "{#MediaDir}\deno.exe"; DestDir: "{app}\resources\media"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Stonks Chat (GPUI)"; Filename: "{app}\stonks-app.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\stonks-app.exe"; WorkingDir: "{app}"; Flags: nowait runasoriginaluser; Check: IsUpdate

[Code]
const
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\com.rdxlabs.stonks.gpui_is1';
  SynchronizeAccess = $00100000;
  InvalidParameter = 87;
  WaitObject0 = 0;

var
  UpdateDir: String;
  UpdateFrom: String;

function OpenProcess(Access: DWORD; Inherit: DWORD; ProcessId: DWORD): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(Handle: THandle; Milliseconds: DWORD): DWORD;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): DWORD;
  external 'CloseHandle@kernel32.dll stdcall';

function IsUpdate: Boolean;
var
  I: Integer;
begin
  Result := True;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/UPDATE') = 0 then Exit;
  Result := False;
end;

function WaitForApp(Pid: Integer): Boolean;
var
  AppProcess: THandle;
begin
  AppProcess := OpenProcess(SynchronizeAccess, 0, Pid);
  if AppProcess = 0 then begin
    { The updater may have finished quitting before Setup reached this point. }
    Result := DLLGetLastError = InvalidParameter;
    Exit;
  end;
  try
    Result := WaitForSingleObject(AppProcess, 30000) = WaitObject0;
  finally
    CloseHandle(AppProcess);
  end;
end;

function RegisteredInstallMatches: Boolean;
var
  Dir, Version: String;
begin
  Result := RegQueryStringValue(HKCU, UninstallKey, 'Inno Setup: App Path', Dir) and
    RegQueryStringValue(HKCU, UninstallKey, 'DisplayVersion', Version) and
    (CompareText(Dir, UpdateDir) = 0) and (Version = UpdateFrom) and
    FileExists(AddBackslash(UpdateDir) + 'stonks-app.exe');
end;

function InitializeSetup: Boolean;
var
  Pid: Integer;
begin
  Result := True;
  if not IsUpdate then Exit;
  Pid := StrToIntDef(ExpandConstant('{param:UPDATEPID|0}'), 0);
  UpdateFrom := ExpandConstant('{param:UPDATEFROM|}');
  Result := (Pid > 0) and (UpdateFrom <> '') and
    RegQueryStringValue(HKCU, UninstallKey, 'Inno Setup: App Path', UpdateDir) and
    RegisteredInstallMatches;
  if not Result then begin
    MsgBox('The registered Stonks Chat installation does not match this update.', mbError, MB_OK);
    Exit;
  end;
  Result := WaitForApp(Pid);
  if not Result then
    MsgBox('Stonks Chat did not close. Close it and try the update again.', mbError, MB_OK);
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := IsUpdate and (PageID = wpSelectDir);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if IsUpdate and ((CompareText(ExpandConstant('{app}'), UpdateDir) <> 0) or
    (not RegisteredInstallMatches)) then
    Result := 'The registered Stonks Chat installation changed. Check for updates again.';
end;
