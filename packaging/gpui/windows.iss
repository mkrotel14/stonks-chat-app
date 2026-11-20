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
