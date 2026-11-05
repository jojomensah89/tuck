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
if (-not (Get-Command cmake.exe -ErrorAction SilentlyContinue)) {
    $candidates = @(
        'C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin',
        'C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin',
        'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath (Join-Path $candidate 'cmake.exe')) { $env:PATH = "$candidate;$env:PATH"; break }
    }
}
if (-not (Get-Command cmake.exe -ErrorAction SilentlyContinue)) { throw 'Install CMake or add its bin directory to PATH.' }
Write-Output 'Native speech build environment ready for this PowerShell session.'

# Portable Parakeet CPU build; never tune the shipped backend to this host.
$env:TRANSCRIBE_CMAKE_ARGS = "-DGGML_NATIVE=OFF -DGGML_AVX512=OFF -DGGML_AVX512_VBMI=OFF -DGGML_AVX512_VNNI=OFF"
