param([Parameter(Mandatory)][int]$ProcessId)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$artifacts = Join-Path $root 'artifacts\file-drop-probe'
New-Item -ItemType Directory -Force $artifacts | Out-Null
$exe = Join-Path $artifacts 'ole-drop-probe.exe'
$source = Join-Path $PSScriptRoot 'ole-drop-probe.cpp'
$vcvars = 'C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat'
# Compiler environment and compilation stay in one child shell.
$ErrorActionPreference = 'Continue'
& cmd /c "`"$vcvars`" >nul && cl /nologo /EHsc `"$source`" /Fo`"$artifacts\probe.obj`" /Fe`"$exe`" ole32.lib user32.lib shell32.lib uuid.lib gdi32.lib"
$compileCode = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($compileCode -ne 0) { throw "Probe compilation failed: $compileCode" }
$cases = @(
    @{Name='idle-collapsed'; Idle=35; Y=120; Open=$false},
    @{Name='second-file-strip'; Idle=2; Y=80; Open=$false},
    @{Name='fresh-collapsed'; Idle=2; Y=120; Open=$false},
    @{Name='already-open'; Idle=2; Y=120; Open=$true},
    @{Name='late-child'; Idle=35; Y=80; Open=$false}
)
try {
    foreach ($case in $cases) {
        $file = Join-Path $artifacts "$($case.Name).txt"
        Set-Content -LiteralPath $file "Tuck OLE probe $($case.Name)"
        $arguments = @($ProcessId, $file, $case.Y, $case.Idle)
        if ($case.Open) { $arguments += 'open' }
        if ($case.Name -eq 'late-child') { $arguments += 'late-child' }
        $log = Join-Path $artifacts "$($case.Name).log"
        & $exe @arguments | Tee-Object -FilePath $log
        $code = $LASTEXITCODE
        Write-Output "$($case.Name): exit=$code"
        if ($code -ne 0) { throw "OLE case failed: $($case.Name), exit=$code" }
    }
} finally {
    # These exact files are created by this script, never the user's shelf files.
    foreach ($case in $cases) {
        Remove-Item -LiteralPath (Join-Path $artifacts "$($case.Name).txt") -ErrorAction SilentlyContinue
    }
}
