param([Parameter(Mandatory)][string]$Destination)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
. "$PSScriptRoot/../../scripts/media-checksums.ps1"

function Save-File([string]$Url, [string]$Path) {
    curl.exe --fail --silent --show-error --location --retry 3 $Url --output $Path
}

function Read-ReleaseHash([string]$Path, [string]$Asset) {
    $pattern = '^([a-fA-F0-9]{64})[ \t]+\*?' + [regex]::Escape($Asset) + '$'
    $matchesForAsset = @(Select-String -LiteralPath $Path -Pattern $pattern -CaseSensitive)
    if ($matchesForAsset.Count -ne 1) { throw "Missing or ambiguous checksum for $Asset in $Path" }
    return $matchesForAsset[0].Matches[0].Groups[1].Value
}

$work = Join-Path $env:RUNNER_TEMP "gpui-media-download-$([guid]::NewGuid())"
New-Item -ItemType Directory $work | Out-Null
try {
    New-Item -ItemType Directory $Destination | Out-Null
    $ytRelease = 'https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19'
    Save-File "$ytRelease/yt-dlp.exe" "$work/yt-dlp.exe"
    Save-File "$ytRelease/SHA2-256SUMS" "$work/yt-dlp.sha256"
    Test-Sha256 "$work/yt-dlp.exe" (Read-ReleaseHash "$work/yt-dlp.sha256" 'yt-dlp.exe')
    Copy-Item "$work/yt-dlp.exe" $Destination

    $denoRelease = 'https://github.com/denoland/deno/releases/download/v2.9.4'
    $denoAsset = 'deno-x86_64-pc-windows-msvc.zip'
    Save-File "$denoRelease/$denoAsset" "$work/deno.zip"
    Save-File "$denoRelease/$denoAsset.sha256sum" "$work/deno.sha256"
    Test-Sha256 "$work/deno.zip" (Read-DenoSha256 "$work/deno.sha256")
    Expand-Archive "$work/deno.zip" "$work/deno"
    Copy-Item "$work/deno/deno.exe" $Destination

    $ffRelease = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest'
    $ffAsset = 'ffmpeg-master-latest-win64-lgpl.zip'
    Save-File "$ffRelease/$ffAsset" "$work/ffmpeg.zip"
    Save-File "$ffRelease/checksums.sha256" "$work/ffmpeg.sha256"
    Test-Sha256 "$work/ffmpeg.zip" (Read-ReleaseHash "$work/ffmpeg.sha256" $ffAsset)
    Expand-Archive "$work/ffmpeg.zip" "$work/ffmpeg"
    $bins = @(Get-ChildItem -LiteralPath "$work/ffmpeg" -Directory | ForEach-Object {
        Join-Path $_.FullName 'bin'
    } | Where-Object { Test-Path -LiteralPath $_ -PathType Container })
    if ($bins.Count -ne 1) { throw 'Expected one bin directory in the verified FFmpeg archive' }
    foreach ($tool in 'ffmpeg.exe', 'ffprobe.exe') {
        Copy-Item -LiteralPath (Join-Path $bins[0] $tool) -Destination $Destination
    }
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force
}
