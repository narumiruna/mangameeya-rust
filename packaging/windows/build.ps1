[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'Run this script on Windows with the Rust MSVC toolchain and NSIS installed.'
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$nsis = Get-Command makensis.exe -ErrorAction SilentlyContinue
$compiler = if ($nsis) { $nsis.Source } else { $null }
if (-not $compiler) {
    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if ($base) {
            $candidate = Join-Path $base 'NSIS/makensis.exe'
            if (Test-Path -LiteralPath $candidate) {
                $compiler = $candidate
                break
            }
        }
    }
}
if (-not $compiler) {
    throw 'NSIS was not found. Install it with: choco install nsis --yes --no-progress'
}

Push-Location $root
try {
    $metadataJson = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq 'mangameeya-rust' })
    if ($package.Count -ne 1) { throw 'Expected one mangameeya-rust Cargo package.' }
    $version = $package[0].version
    if ($version -notmatch '^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
        throw 'The Cargo version is not a valid installer version.'
    }
    $numericVersion = '{0}.{1}.{2}.0' -f $Matches[1], $Matches[2], $Matches[3]
    $target = Join-Path $root 'target'
    & cargo build --locked --release --bin mangameeya-rust --target-dir $target
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed.' }

    $executable = Join-Path $target 'release/mangameeya-rust.exe'
    $icon = Join-Path $root 'assets/icon.ico'
    $license = Join-Path $root 'LICENSE'
    foreach ($file in @($executable, $icon, $license)) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing file: $file" }
    }
    $outputDir = Join-Path $target 'release/bundle/nsis'
    New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
    $output = Join-Path $outputDir "MangaMeeya-Rust_${version}_windows-setup.exe"
    & $compiler '/V3' "/DAPP_VERSION=$version" "/DAPP_VERSION_NUMERIC=$numericVersion" `
        "/DAPP_EXE_PATH=$executable" "/DICON_PATH=$icon" "/DLICENSE_PATH=$license" `
        "/DOUTPUT_FILE=$output" (Join-Path $PSScriptRoot 'installer.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'NSIS compilation failed.' }
    if (-not (Test-Path -LiteralPath $output) -or (Get-Item -LiteralPath $output).Length -eq 0) {
        throw 'NSIS did not produce an installer.'
    }
    Write-Output $output
} finally {
    Pop-Location
}
