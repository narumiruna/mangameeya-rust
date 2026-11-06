[CmdletBinding()]
param([string]$InstallerPath)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'This installer test requires Windows.'
}
if (-not $InstallerPath -or -not (Test-Path -LiteralPath $InstallerPath -PathType Leaf)) {
    throw 'Supply -InstallerPath with the NSIS installer path. No interactive input is supported.'
}
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$appKey = 'HKCU:\Software\io.github.mangameeya.rust'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.mangameeya.rust'
$progId = 'MangaMeeyaRust.Comic'
$extensions = @('jpg', 'jpeg', 'jpe', 'png', 'gif', 'bmp', 'webp', 'zip', 'cbz', 'rar', 'cbr')
foreach ($key in @($appKey, $uninstallKey, "HKCU:\Software\Classes\$progId",
        'HKCU:\Software\Classes\Applications\mangameeya-rust.exe')) {
    if (Test-Path -LiteralPath $key) { throw 'Use a clean CI user profile, without an existing installation.' }
}

function Get-ExtensionDefault([string]$Extension) {
    $key = Get-Item -LiteralPath "HKCU:\Software\Classes\.$Extension" -ErrorAction SilentlyContinue
    if ($key) { return $key.GetValue('') }
    return $null
}

function Invoke-Setup([string]$Path, [string[]]$Arguments) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Installer process failed with exit code $($process.ExitCode)." }
}

$defaults = @{}
foreach ($extension in $extensions) { $defaults[$extension] = Get-ExtensionDefault $extension }
$installDir = Join-Path ([IO.Path]::GetTempPath()) ('mangameeya-install-' + [Guid]::NewGuid().ToString('N'))
$uninstaller = Join-Path $installDir 'Uninstall.exe'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'MangaMeeya Rust/MangaMeeya Rust.lnk'
try {
    # NSIS requires /D to be the last argument, without enclosing quotes.
    Invoke-Setup $InstallerPath @('/S', "/D=$installDir")
    foreach ($path in @((Join-Path $installDir 'mangameeya-rust.exe'), $uninstaller, $shortcut)) {
        if (-not (Test-Path -LiteralPath $path)) { throw "Missing installed file: $path" }
    }
    if (-not (Test-Path -LiteralPath $uninstallKey)) { throw 'Current-user uninstall registration is missing.' }
    $commandKey = Get-Item -LiteralPath "HKCU:\Software\Classes\$progId\shell\open\command"
    if ($commandKey.GetValue('') -ne ('"{0}\mangameeya-rust.exe" "%1"' -f $installDir)) {
        throw 'The file-open command does not quote the executable and launch path correctly.'
    }
    foreach ($extension in $extensions) {
        $key = Get-Item -LiteralPath "HKCU:\Software\Classes\.$extension\OpenWithProgids"
        if ($key.GetValueNames() -notcontains $progId) { throw "Missing OpenWithProgids entry: $extension" }
        if ((Get-ExtensionDefault $extension) -ne $defaults[$extension]) {
            throw "The installer changed the default association: $extension"
        }
    }
    Invoke-Setup $uninstaller @('/S')
    # The NSIS uninstaller can run a temporary child process.
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while ((Test-Path -LiteralPath $uninstallKey) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
    foreach ($path in @($appKey, $uninstallKey, "HKCU:\Software\Classes\$progId",
            'HKCU:\Software\Classes\Applications\mangameeya-rust.exe',
            (Join-Path $installDir 'mangameeya-rust.exe'), $shortcut)) {
        if (Test-Path -LiteralPath $path) { throw "Uninstall did not remove: $path" }
    }
    foreach ($extension in $extensions) {
        $key = Get-Item -LiteralPath "HKCU:\Software\Classes\.$extension\OpenWithProgids" -ErrorAction SilentlyContinue
        if ($key -and $key.GetValueNames() -contains $progId) { throw "Association was not removed: $extension" }
        if ((Get-ExtensionDefault $extension) -ne $defaults[$extension]) {
            throw "Uninstall changed the default association: $extension"
        }
    }
    Write-Output 'Current-user install, shortcuts, all 11 Open With associations, and uninstall passed.'
} finally {
    if ((Test-Path -LiteralPath $uninstaller) -and (Test-Path -LiteralPath $uninstallKey)) {
        Invoke-Setup $uninstaller @('/S')
    }
    if (Test-Path -LiteralPath $installDir) { Remove-Item -LiteralPath $installDir -Recurse -Force }
}
