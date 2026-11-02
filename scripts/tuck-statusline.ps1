<#
.SYNOPSIS
Tuck Claude Code statusline forwarder.

.DESCRIPTION
Receives session JSON payload from Claude Code via stdin and persists it to
%APPDATA%\Tuck\claude-status.json for Tuck's AI usage backend.
Prints 'Tuck' as the status line text in Claude Code.
#>

[Console]::InputEncoding = [System.Text.Encoding]::UTF8
$json = [Console]::In.ReadToEnd()

if (-not [string]::IsNullOrWhiteSpace($json)) {
    $dir = Join-Path $env:APPDATA 'Tuck'
    if (-not (Test-Path -Path $dir)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }
    $targetFile = Join-Path $dir 'claude-status.json'
    [System.IO.File]::WriteAllText($targetFile, $json, [System.Text.Encoding]::UTF8)
}

Write-Output 'Tuck'
