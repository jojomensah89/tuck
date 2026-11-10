param([switch]$InstallBuildTools)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$clangRoot = Join-Path $projectRoot 'artifacts\build-tools'
if ($InstallBuildTools) {
    python -m pip install --target $clangRoot libclang==18.1.1
    if ($LASTEXITCODE -ne 0) { throw 'libclang installation failed' }
}
$clangNative = Join-Path $clangRoot 'clang\native'
if (-not (Test-Path -LiteralPath (Join-Path $clangNative 'libclang.dll'))) {
    throw 'Run .\scripts\prepare-native.ps1 -InstallBuildTools to install the native speech build dependency.'
}
$env:LIBCLANG_PATH = $clangNative
$candidates = @('C:\Program Files\CMake\bin')
$onPath = Get-Command cmake.exe -ErrorAction SilentlyContinue
if ($onPath) { $candidates += Split-Path -Parent $onPath.Source }
$candidates += @(
    'C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin',
    'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin',
    'C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
)
$foundCmake = $false
foreach ($candidate in ($candidates | Select-Object -Unique)) {
    $exe = Join-Path $candidate 'cmake.exe'
    if (-not (Test-Path -LiteralPath $exe)) { continue }
    $versionText = & $exe --version
    if ($LASTEXITCODE -eq 0 -and ($versionText -join "`n") -match 'cmake version (\d+\.\d+\.\d+)' -and [version]$Matches[1] -ge [version]'3.22.0') {
        $env:PATH = "$candidate;$env:PATH"
        $foundCmake = $true
        break
    }
    Write-Warning "Rejected CMake at $exe. Tuck requires CMake 3.22 or newer. Install with: winget install Kitware.CMake"
}
if (-not $foundCmake) { throw 'CMake 3.22 or newer is required. Install with: winget install Kitware.CMake, then open a new shell.' }

# Portable speech build; never tune the shipped backend to this host.
$env:TRANSCRIBE_CMAKE_ARGS = "-DGGML_NATIVE=OFF -DGGML_AVX512=OFF -DGGML_AVX512_VBMI=OFF -DGGML_AVX512_VNNI=OFF"

$vulkanSdk = $env:VULKAN_SDK
if (-not $vulkanSdk) { $vulkanSdk = [Environment]::GetEnvironmentVariable('VULKAN_SDK', 'Machine') }
if (-not $vulkanSdk) {
    $vulkanSdk = Get-ChildItem 'C:\VulkanSDK' -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $vulkanSdk -and $InstallBuildTools) {
    winget install KhronosGroup.VulkanSDK --silent --accept-package-agreements --accept-source-agreements
    if ($LASTEXITCODE -ne 0) { throw 'Vulkan SDK installation failed' }
    $vulkanSdk = [Environment]::GetEnvironmentVariable('VULKAN_SDK', 'Machine')
}
if (-not $vulkanSdk -or -not (Test-Path "$vulkanSdk\Bin\glslc.exe")) { throw 'Vulkan SDK required: run prepare-native.ps1 -InstallBuildTools.' }
$env:VULKAN_SDK = $vulkanSdk

Write-Output 'Native speech build environment ready for this PowerShell session.'
