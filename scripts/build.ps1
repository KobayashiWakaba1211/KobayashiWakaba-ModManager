# 构建脚本：把编译产物放到系统盘的用户目录，避免慢盘/机械盘写小文件拖死编译。
# 用法：powershell -ExecutionPolicy Bypass -File scripts\build.ps1 [-Bundle]
#   -Bundle  打 NSIS 安装包（默认只编译可执行文件，更快）

param([switch]$Bundle)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# 产物目录：优先用环境变量，其次放 %LOCALAPPDATA%（比项目所在的机械盘/慢盘快很多）
if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA 'emm-build-target'
}
# 并行度：显式用满逻辑核心（Cargo 默认就是，这里固定下来避免被环境变量压低）
if (-not $env:CARGO_BUILD_JOBS) {
    $env:CARGO_BUILD_JOBS = "$([Environment]::ProcessorCount)"
}

Write-Host "产物目录: $env:CARGO_TARGET_DIR"
Write-Host "并行线程: $env:CARGO_BUILD_JOBS"

$sw = [Diagnostics.Stopwatch]::StartNew()
if ($Bundle) {
    npx tauri build
} else {
    # 只编译 Rust 可执行文件（不重新构建前端、不打包），日常验证用这个
    Push-Location src-tauri
    cargo build --release
    Pop-Location
}
$sw.Stop()
Write-Host ("完成，用时 {0:N0} 秒" -f $sw.Elapsed.TotalSeconds)
