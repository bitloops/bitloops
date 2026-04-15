[CmdletBinding()]
param(
  [string]$Repo = $(if ($env:BITLOOPS_REPO) { $env:BITLOOPS_REPO } else { "bitloops/bitloops" }),
  [string]$InstallDir = $(if ($env:BITLOOPS_INSTALL_DIR) { $env:BITLOOPS_INSTALL_DIR } else { Join-Path $env:USERPROFILE ".bitloops\bin" })
)

$ErrorActionPreference = "Stop"

function Get-TargetTriplet {
  $archCandidates = @()
  if ($env:PROCESSOR_ARCHITEW6432) { $archCandidates += $env:PROCESSOR_ARCHITEW6432.ToLowerInvariant() }
  if ($env:PROCESSOR_ARCHITECTURE) { $archCandidates += $env:PROCESSOR_ARCHITECTURE.ToLowerInvariant() }

  foreach ($arch in $archCandidates) {
    switch ($arch) {
      "arm64" { return "aarch64-pc-windows-msvc" }
      "amd64" { return "x86_64-pc-windows-msvc" }
      "x86_64" { return "x86_64-pc-windows-msvc" }
    }
  }

  $detected = if ($archCandidates.Count -gt 0) { ($archCandidates -join ", ") } else { "unknown" }
  throw "Unsupported Windows architecture: $detected"
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

function Find-ExtractedBinary {
  param(
    [Parameter(Mandatory = $true)][string]$ExtractDir,
    [Parameter(Mandatory = $true)][string]$Name
  )

  $direct = Join-Path $ExtractDir $Name
  if (Test-Path $direct) {
    return $direct
  }

  $found = Get-ChildItem -Path $ExtractDir -Filter $Name -Recurse | Select-Object -First 1
  if ($found) {
    return $found.FullName
  }

  return $null
}

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

  $binaryPath = Find-ExtractedBinary -ExtractDir $extractDir -Name "bitloops.exe"
  if (-not $binaryPath) {
    throw "Extracted archive did not contain bitloops.exe"
  }
  $embeddingsBinaryPath = Find-ExtractedBinary -ExtractDir $extractDir -Name "bitloops-local-embeddings.exe"

  New-Item -Path $InstallDir -ItemType Directory -Force | Out-Null
  $targetPath = Join-Path $InstallDir "bitloops.exe"
  Copy-Item -Path $binaryPath -Destination $targetPath -Force
  if ($embeddingsBinaryPath) {
    $embeddingsTargetPath = Join-Path $InstallDir "bitloops-local-embeddings.exe"
    Copy-Item -Path $embeddingsBinaryPath -Destination $embeddingsTargetPath -Force
  }

  $pathChanged = Add-ToUserPath -PathToAdd $InstallDir

  Write-Host "Installed bitloops $tag to $targetPath"
  if ($embeddingsBinaryPath) {
    Write-Host "Installed bitloops-local-embeddings $tag to $embeddingsTargetPath"
  }
  else {
    Write-Host "Note: this release archive did not contain bitloops-local-embeddings; installing bitloops only."
  }
  if ($pathChanged) {
    Write-Host "Added $InstallDir to user PATH. Restart your terminal for PATH changes to apply."
  }
}
finally {
  if (Test-Path $tempDir) {
    Remove-Item -Path $tempDir -Recurse -Force
  }
}
