param([Parameter(Mandatory)][ValidateSet('Setup', 'Package')][string]$Mode)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path

function Get-VerifiedFile($Url, $Destination, $Sha256) {
    curl.exe --fail --silent --show-error --location --retry 3 $Url --output $Destination
    if ((Get-FileHash $Destination -Algorithm SHA256).Hash -ne $Sha256) {
        throw "SHA256 mismatch: $Destination"
    }
}

function Invoke-Installer($File, $Arguments) {
    $result = Start-Process -FilePath $File -ArgumentList $Arguments -Wait -PassThru
    if ($result.ExitCode -ne 0) { throw "$File exited with $($result.ExitCode)" }
}

if ($Mode -eq 'Setup') {
    $sdk = Join-Path $env:RUNNER_TEMP 'gpui-gstreamer'
    $gstInstaller = Join-Path $env:RUNNER_TEMP 'gstreamer-1.28.0.exe'
    Get-VerifiedFile 'https://gstreamer.freedesktop.org/data/pkg/windows/1.28.0/msvc/gstreamer-1.0-msvc-x86_64-1.28.0.exe' `
        $gstInstaller 'd6aca5785cae9d9ed447db491cc921163ff6af0c657eb733293c55416c8b71a6'
    Invoke-Installer $gstInstaller ('/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /TYPE=full /DIR="{0}"' -f $sdk)
    $archive = Join-Path $env:RUNNER_TEMP 'gpui-pkgconf.tar.zst'
    Get-VerifiedFile 'https://repo.msys2.org/mingw/ucrt64/mingw-w64-ucrt-x86_64-pkgconf-1~3.0.9-1-any.pkg.tar.zst' `
        $archive '4ea9bdd77df366333b973771c2ec377e77a4354e60c16a4556af7d849494366f'
    tar.exe -xf $archive -C $env:RUNNER_TEMP ucrt64/bin
    $env:PKG_CONFIG = Join-Path $env:RUNNER_TEMP 'ucrt64/bin/pkgconf.exe'
    $env:PKG_CONFIG_LIBDIR = (Join-Path $sdk 'lib/pkgconfig') -replace '\\', '/'
    $env:PKG_CONFIG_PATH = $env:PKG_CONFIG_LIBDIR
    & $env:PKG_CONFIG --atleast-version=1.28 gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0
    & $env:PKG_CONFIG --libs gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0
    & (Join-Path $sdk 'bin/gst-inspect-1.0.exe') --version
    "PKG_CONFIG=$env:PKG_CONFIG", "PKG_CONFIG_LIBDIR=$env:PKG_CONFIG_LIBDIR", "PKG_CONFIG_PATH=$env:PKG_CONFIG_PATH" |
        Add-Content $env:GITHUB_ENV -Encoding utf8
    Join-Path $sdk 'bin' | Add-Content $env:GITHUB_PATH -Encoding utf8
    return
}

$binary = Join-Path $repo 'crates/stonks-app/target/x86_64-pc-windows-msvc/release/stonks-app.exe'
$notices = Join-Path $repo 'THIRD_PARTY_NOTICES.md'
$projectLicense = Join-Path $repo 'LICENSE'
$version = (cargo metadata --locked --no-deps --format-version 1 --manifest-path "$repo/crates/stonks-app/Cargo.toml" |
    ConvertFrom-Json).packages.Where({ $_.name -eq 'stonks-app' }).version
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "Unexpected GPUI version: $version" }
$out = Join-Path $repo 'gpui-artifacts'
New-Item -Force -ItemType Directory $out | Out-Null
$iscc = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6/ISCC.exe'
(Get-Item $iscc).VersionInfo.FileVersion | Write-Host


$setup = Join-Path $out "Stonks.GPUI_${version}_x64-setup.exe"
$state = Join-Path $env:RUNNER_TEMP "gpui-smoke-$([guid]::NewGuid())"
$stagedMedia = Join-Path $state 'staged-media'
$installed = Join-Path $state 'installed'
$installedMedia = Join-Path $installed 'resources/media'
$mediaVersionArgs = [ordered]@{
    'ffmpeg.exe' = '-version'
    'ffprobe.exe' = '-version'
    'yt-dlp.exe' = '--version'
    'deno.exe' = '--version'
}
$uninstaller = Join-Path $installed 'unins000.exe'
$process = $null
New-Item -ItemType Directory $state | Out-Null
try {
    & "$PSScriptRoot/prepare-media.ps1" -Destination $stagedMedia
    & $iscc "/DBuildVersion=$version" "/DBinary=$binary" "/DNotices=$notices" "/DProjectLicense=$projectLicense" "/DMediaDir=$stagedMedia" "/DArtifactDir=$out" "$PSScriptRoot/windows.iss"
    Invoke-Installer $setup ('/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /DIR="{0}"' -f $installed)
    $installedBinary = Join-Path $installed 'stonks-app.exe'
    if ((Get-FileHash $installedBinary).Hash -ne (Get-FileHash $binary).Hash) { throw 'Installed binary differs from release build' }
    if ((Get-FileHash (Join-Path $installed 'THIRD_PARTY_NOTICES.md')).Hash -ne (Get-FileHash $notices).Hash) { throw 'Installed notices differ' }
    foreach ($tool in $mediaVersionArgs.Keys) {
        $file = Join-Path $installedMedia $tool
        if ((Get-FileHash -LiteralPath $file).Hash -ne (Get-FileHash -LiteralPath (Join-Path $stagedMedia $tool)).Hash) {
            throw "Installed $tool differs from verified payload"
        }
        $versionCheck = Start-Process -FilePath $file -ArgumentList $mediaVersionArgs[$tool] `
            -WorkingDirectory $state -NoNewWindow -PassThru
        try {
            if (-not $versionCheck.WaitForExit(30000)) { throw "$tool version check timed out" }
            if ($versionCheck.ExitCode -ne 0) { throw "$tool version check exited with $($versionCheck.ExitCode)" }
        } finally {
            if (-not $versionCheck.HasExited) { Stop-Process -Id $versionCheck.Id -Force }
            $versionCheck.Dispose()
        }
    }
    $env:STONKS_ALLOW_MULTI_INSTANCE = '1'
    $savedAppData = $env:APPDATA
    $savedLocalAppData = $env:LOCALAPPDATA
    $savedXdgConfig = $env:XDG_CONFIG_HOME
    try {
        $env:APPDATA = Join-Path $state 'roaming'
        $env:LOCALAPPDATA = Join-Path $state 'local'
        $env:XDG_CONFIG_HOME = $env:APPDATA
        $process = Start-Process -FilePath $installedBinary -ArgumentList '--automatic-renderer' -PassThru
    } finally {
        $env:APPDATA = $savedAppData
        $env:LOCALAPPDATA = $savedLocalAppData
        $env:XDG_CONFIG_HOME = $savedXdgConfig
    }
    $opened = $false
    for ($i = 0; $i -lt 60; $i++) {
        $process.Refresh()
        if ($process.HasExited) { throw "GPUI exited before showing a window: $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne 0 -and $process.MainWindowTitle -eq 'Stonks Chat') {
            $opened = $true
            break
        }
        Start-Sleep -Seconds 1
    }
    if (-not $opened) { throw 'GPUI never opened its main window' }
} finally {
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
    if (Test-Path $uninstaller) { Invoke-Installer $uninstaller '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' }
    if (Test-Path (Join-Path $installed 'stonks-app.exe')) { throw 'Uninstall left the application executable behind' }
    foreach ($tool in $mediaVersionArgs.Keys) {
        if (Test-Path -LiteralPath (Join-Path $installedMedia $tool)) { throw "Uninstall left $tool behind" }
    }
    Remove-Item $state -Recurse -Force
}
