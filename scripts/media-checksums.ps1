# Deno's Windows checksum is formatted Get-FileHash output; other releases
# use a bare digest or sha256sum output. Accept exactly one SHA256 digest.
function Read-DenoSha256([string]$path) {
    $content = (Get-Content -LiteralPath $path -Raw).Trim()
    $hashes = [regex]::Matches($content, '(?im)^\s*Hash\s*:\s*([a-f0-9]{64})\s*$')
    if ($hashes.Count -eq 1 -and $content -match '(?im)^\s*Algorithm\s*:\s*SHA256\s*$') {
        return $hashes[0].Groups[1].Value
    }
    if ($content -match '\A([a-fA-F0-9]{64})(?:[ \t]+\*?deno-x86_64-pc-windows-msvc\.zip)?\z') {
        return $Matches[1]
    }
    throw "Missing or ambiguous SHA256 checksum in $path"
}

function Test-Sha256([string]$path, [string]$expected) {
    if ($expected -notmatch '\A[a-fA-F0-9]{64}\z') {
        throw "Invalid SHA256 checksum for $path"
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
    if ($actual -ne $expected.ToLowerInvariant()) {
        throw "SHA256 mismatch for $path : expected $expected, got $actual"
    }
}
