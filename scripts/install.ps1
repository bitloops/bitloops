[CmdletBinding()]
param(
  [string]$Repo = $(if ($env:BITLOOPS_REPO) { $env:BITLOOPS_REPO } else { "bitloops/bitloops" }),
  [string]$InstallDir = $(if ($env:BITLOOPS_INSTALL_DIR) { $env:BITLOOPS_INSTALL_DIR } else { Join-Path $env:USERPROFILE ".bitloops\bin" })
)

$ErrorActionPreference = "Stop"

function Get-TargetTriplet {
  $arch = if ($env:PROCESSOR_ARCHITECTURE) { $env:PROCESSOR_ARCHITECTURE.ToLowerInvariant() } else { "unknown" }
  switch ($arch) {
    "amd64" { return "x86_64-pc-windows-msvc" }
    "x86_64" { return "x86_64-pc-windows-msvc" }
    default { throw "Unsupported Windows architecture: $arch" }
  }
}

function Add-ToUserPath {
  param([Parameter(Mandatory = $true)][string]$PathToAdd)

  $current = [Environment]::GetEnvironmentVariable("Path", "User")
  if (-not $current) {
    [Environment]::SetEnvironmentVariable("Path", $PathToAdd, "User")
    return $true
  }

  $segments = $current -split ";"
  $exists = $segments | Where-Object { $_.TrimEnd("\") -ieq $PathToAdd.TrimEnd("\") }
  if ($exists) {
    return $false
  }

  [Environment]::SetEnvironmentVariable("Path", "$current;$PathToAdd", "User")
  return $true
}

$target = Get-TargetTriplet
$assetName = "bitloops-$target.zip"
$checksumsName = "checksums-sha256.txt"
$releaseApiUrl = "https://api.github.com/repos/$Repo/releases/latest"

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("bitloops-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -Path $tempDir -ItemType Directory -Force | Out-Null

try {
  Write-Host "Resolving latest release for $Repo..."
  $latest = Invoke-RestMethod -Uri $releaseApiUrl -Headers @{ "User-Agent" = "bitloops-installer" }
  if (-not $latest.tag_name) {
    throw "Could not resolve latest release tag from $releaseApiUrl"
  }

  $tag = $latest.tag_name
  $assetUrl = "https://github.com/$Repo/releases/download/$tag/$assetName"
  $checksumsUrl = "https://github.com/$Repo/releases/download/$tag/$checksumsName"

  $assetPath = Join-Path $tempDir $assetName
  $checksumsPath = Join-Path $tempDir $checksumsName

  Write-Host "Downloading $assetName ($tag)..."
  Invoke-WebRequest -Uri $assetUrl -OutFile $assetPath
  Invoke-WebRequest -Uri $checksumsUrl -OutFile $checksumsPath

  $line = Get-Content -Path $checksumsPath |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_ -match ("\s+" + [regex]::Escape($assetName) + "$") } |
    Select-Object -First 1

  if (-not $line) {
    throw "Checksum for $assetName not found in $checksumsName"
  }

  $expectedHash = ($line -split "\s+")[0].ToLowerInvariant()
  $actualHash = (Get-FileHash -Path $assetPath -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($expectedHash -ne $actualHash) {
    throw "Checksum mismatch for $assetName. Expected: $expectedHash Actual: $actualHash"
  }

  $extractDir = Join-Path $tempDir "extract"
  Expand-Archive -Path $assetPath -DestinationPath $extractDir -Force

  $binaryPath = Join-Path $extractDir "bitloops.exe"
  if (-not (Test-Path $binaryPath)) {
    $found = Get-ChildItem -Path $extractDir -Filter "bitloops.exe" -Recurse | Select-Object -First 1
    if (-not $found) {
      throw "Extracted archive did not contain bitloops.exe"
    }
    $binaryPath = $found.FullName
  }

  New-Item -Path $InstallDir -ItemType Directory -Force | Out-Null
  $targetPath = Join-Path $InstallDir "bitloops.exe"
  Copy-Item -Path $binaryPath -Destination $targetPath -Force

  $pathChanged = Add-ToUserPath -PathToAdd $InstallDir

  Write-Host "Installed bitloops $tag to $targetPath"
  if ($pathChanged) {
    Write-Host "Added $InstallDir to user PATH. Restart your terminal for PATH changes to apply."
  }
}
finally {
  if (Test-Path $tempDir) {
    Remove-Item -Path $tempDir -Recurse -Force
  }
}
