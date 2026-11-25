# 构建脚本：编译产物默认放在项目同级目录（F:\项目文件\emm-build-target），
# 不进项目文件夹、不进 git。用 SSD 且空间充足时，本地盘的写入速度最快。
# 用法：powershell -ExecutionPolicy Bypass -File scripts\build.ps1 [-Bundle]
#   -Bundle  打 NSIS 安装包（默认只编译可执行文件，更快）

param([switch]$Bundle)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# 产物目录：优先用环境变量，其次放在项目文件夹旁边（与项目同盘，方便管理）
if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = Join-Path (Split-Path -Parent $root) 'emm-build-target'
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
