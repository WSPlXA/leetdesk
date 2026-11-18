$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskDestination = Join-Path $taskRoot 'tools\jdtls'
$taskArchive = Join-Path $taskRoot 'target\java-lsp-download\jdtls.tar.gz'
# Pinned Eclipse milestone, also usable by the integration tests.
$taskUrl = 'https://download.eclipse.org/jdtls/milestones/1.63.0/jdt-language-server-1.63.0-202610191322.tar.gz'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $taskArchive) | Out-Null
Invoke-WebRequest -Uri $taskUrl -OutFile $taskArchive
if ((Get-FileHash -LiteralPath $taskArchive -Algorithm SHA256).Hash -ne 'B1F2ABE0B3CCCB462CF64FB4A5587C7D09DF60CF8BE7748B152CBD89D018BDC4') {
    throw 'JDT LS 下载校验失败。'
}
New-Item -ItemType Directory -Force -Path $taskDestination | Out-Null
tar -xzf $taskArchive -C $taskDestination
if ($LASTEXITCODE -ne 0) { throw 'JDT LS 解压失败。' }
Write-Host "JDT LS 已安装到 $taskDestination。请使用 Java 25+ 启动 LeetDesk。"
