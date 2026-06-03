<#
Packages a Windows release binary as a zip archive with a SHA-256 file.

The script checks that the target `.exe` exists, stages it under the expected
binary name, writes a zip archive, and emits a checksum beside it. That keeps
Windows release artifacts aligned with the Unix packaging scripts while using
PowerShell-native archive and hash commands.
#>
param(
  [Parameter(Mandatory = $true)][string]$Target,
  [Parameter(Mandatory = $true)][string]$Version,
  [string]$BinName = 'vent',
  [string]$OutDir = 'dist'
)

$ErrorActionPreference = 'Stop'

$binPath = "target/$Target/release/$BinName.exe"
if (-not (Test-Path $binPath)) {
  throw "Binary not found: $binPath"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$outDirFull = (Resolve-Path $OutDir).Path

$archiveName = "$BinName-v$Version-$Target.zip"
$archivePath = Join-Path $outDirFull $archiveName

$tempDir = Join-Path $env:TEMP ("vent-" + [Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
try {
  Copy-Item -Path $binPath -Destination (Join-Path $tempDir "$BinName.exe") -Force
  Push-Location $tempDir
  Compress-Archive -Path "$BinName.exe" -DestinationPath $archivePath -Force
} finally {
  Pop-Location
  Remove-Item -Recurse -Force $tempDir
}

$hashPath = "$archivePath.sha256"
$hash = Get-FileHash -Algorithm SHA256 -Path $archivePath
"$($hash.Hash)  $archiveName" | Out-File -FilePath $hashPath -Encoding ascii
