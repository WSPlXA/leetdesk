$ErrorActionPreference = 'Stop'
$taskProjectRoot = $PSScriptRoot
Push-Location -LiteralPath $taskProjectRoot
try {
    cargo +stable-x86_64-pc-windows-msvc build --locked --profile desktop
    if ($LASTEXITCODE -ne 0) { throw '构建失败，请检查 Rust MSVC 和 C++ Build Tools。' }
    $taskExecutable = Join-Path $taskProjectRoot 'target\desktop\leetdesk.exe'
    Start-Process -FilePath $taskExecutable -WorkingDirectory $taskProjectRoot -WindowStyle Hidden
}
finally {
    Pop-Location
}
