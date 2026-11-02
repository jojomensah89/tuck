<#
.SYNOPSIS
Tuck Claude Code statusline forwarder.

.DESCRIPTION
Receives session JSON payload from Claude Code via stdin and atomically persists it to
%APPDATA%\Tuck\claude-status.json for Tuck's AI usage backend.
Prints 'Tuck' as the status line text in Claude Code.
#>

[Console]::InputEncoding = [System.Text.Encoding]::UTF8
$json = [Console]::In.ReadToEnd()

if (-not [string]::IsNullOrWhiteSpace($json)) {
    $dir = Join-Path $env:APPDATA 'Tuck'
    if (-not (Test-Path -LiteralPath $dir)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }
    $targetFile = Join-Path $dir 'claude-status.json'
    $guid = [System.Guid]::NewGuid().ToString('N')
    $tempFile = Join-Path $dir "claude-status.$guid.tmp"

    try {
        [System.IO.File]::WriteAllText($tempFile, $json, [System.Text.Encoding]::UTF8)

        $written = $false
        for ($attempt = 1; $attempt -le 5; $attempt++) {
            try {
                if ([System.IO.File]::Exists($targetFile)) {
                    # Atomic file replace on Windows / NTFS
                    [System.IO.File]::Replace($tempFile, $targetFile, $null)
                } else {
                    [System.IO.File]::Move($tempFile, $targetFile)
                }
                $written = $true
                break
            } catch {
                Start-Sleep -Milliseconds 25
            }
        }

        if (-not $written -and [System.IO.File]::Exists($tempFile)) {
            Remove-Item -LiteralPath $tempFile -Force -ErrorAction SilentlyContinue
        }
    } catch {
        # Never log or output stdin/json secrets on failure
        if ([System.IO.File]::Exists($tempFile)) {
            Remove-Item -LiteralPath $tempFile -Force -ErrorAction SilentlyContinue
        }
    }
}

Write-Output 'Tuck'
