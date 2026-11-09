param(
    [string]$Exe,
    [string]$OutputDirectory,
    [string]$IdentityName = $env:MSIX_IDENTITY_NAME,
    [string]$Publisher = $env:MSIX_PUBLISHER,
    [string]$PublisherDisplayName = $env:MSIX_PUBLISHER_DISPLAY_NAME,
    # The product name reserved in Partner Center; Properties/DisplayName must match it exactly
    [string]$StoreName = $env:MSIX_STORE_NAME
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (!$Exe) { $Exe = Join-Path $root 'src-tauri/target/release/tuck.exe' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $root 'artifacts/msix' }
if (!(Test-Path -LiteralPath $Exe)) { throw "Release executable missing: $Exe. Run bun run tauri build first." }
$identity = Get-Content (Join-Path $root 'src-tauri/msix/identity.json') -Raw | ConvertFrom-Json
if (!$IdentityName) { $IdentityName = $identity.name; Write-Warning 'Using placeholder MSIX Identity Name' }
if (!$Publisher) { $Publisher = $identity.publisher; Write-Warning 'Using placeholder MSIX Publisher' }
if (!$PublisherDisplayName) { $PublisherDisplayName = $identity.publisherDisplayName; Write-Warning 'Using placeholder MSIX PublisherDisplayName' }
if (!$StoreName) { $StoreName = $identity.storeName; Write-Warning 'Using placeholder MSIX store name' }
$version = (Get-Content (Join-Path $root 'package.json') -Raw | ConvertFrom-Json).version
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'MSIX requires a numeric major.minor.patch package version' }
$version = "$version.0"
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$layout = Join-Path ([IO.Path]::GetTempPath()) ('tuck-msix-' + [guid]::NewGuid())
New-Item -ItemType Directory -Force (Join-Path $layout 'Assets') | Out-Null
Copy-Item -LiteralPath $Exe -Destination (Join-Path $layout 'tuck.exe')
foreach ($logo in @('Square44x44Logo.png','Square150x150Logo.png','StoreLogo.png')) {
    Copy-Item -LiteralPath (Join-Path $root "src-tauri/icons/$logo") -Destination (Join-Path $layout "Assets/$logo")
}
# Compose the wide tile from the existing artwork without stretching it.
Add-Type -AssemblyName System.Drawing
$source = [Drawing.Image]::FromFile((Join-Path $root 'src-tauri/icons/Square150x150Logo.png'))
$bitmap = New-Object Drawing.Bitmap 310,150
$graphics = [Drawing.Graphics]::FromImage($bitmap)
try { $graphics.Clear([Drawing.Color]::Transparent); $graphics.DrawImage($source,80,0,150,150); $bitmap.Save((Join-Path $layout 'Assets/Wide310x150Logo.png'),[Drawing.Imaging.ImageFormat]::Png) }
finally { $graphics.Dispose(); $bitmap.Dispose(); $source.Dispose() }
$manifest = Get-Content (Join-Path $root 'src-tauri/msix/AppxManifest.xml') -Raw
foreach ($entry in @{NAME=$IdentityName;PUBLISHER=$Publisher;DISPLAY=$PublisherDisplayName;STORE_NAME=$StoreName;VERSION=$version}.GetEnumerator()) {
    $manifest = $manifest.Replace('{{' + $entry.Key + '}}', [Security.SecurityElement]::Escape($entry.Value))
}
[IO.File]::WriteAllText((Join-Path $layout 'AppxManifest.xml'), $manifest)
$makeappx = Get-Command makeappx.exe -ErrorAction SilentlyContinue
if ($makeappx) { $tool = $makeappx.Source } else {
    $tool = Get-ChildItem 'C:/Program Files (x86)/Windows Kits/10/bin/*/x64/makeappx.exe' | Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
}
if (!$tool) { throw 'Install Windows SDK (makeappx.exe)' }
$package = Join-Path (Resolve-Path $OutputDirectory) "Tuck_${version}_x64.msix"
& $tool pack /d $layout /p $package /o
if ($LASTEXITCODE -ne 0) { throw "makeappx failed; staged layout retained at $layout" }
# This run's staging copy is only needed when packing fails
Remove-Item -LiteralPath $layout -Recurse -Force
Write-Output "Unsigned package: $package"
