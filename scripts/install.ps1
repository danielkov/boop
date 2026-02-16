# boop installation script for Windows
# Usage: irm https://raw.githubusercontent.com/danielkov/boop/main/scripts/install.ps1 | iex

$ErrorActionPreference = 'Stop'

$Repo = "danielkov/boop"
$BinaryName = "boop"
$InstallDir = if ($env:BOOP_INSTALL_DIR) { $env:BOOP_INSTALL_DIR } else { "$env:USERPROFILE\.boop\bin" }

function Write-Info {
    param([string]$Message)
    Write-Host "info: " -ForegroundColor Blue -NoNewline
    Write-Host $Message
}

function Write-Success {
    param([string]$Message)
    Write-Host "success: " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-Warn {
    param([string]$Message)
    Write-Host "warning: " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

function Write-Err {
    param([string]$Message)
    Write-Host "error: " -ForegroundColor Red -NoNewline
    Write-Host $Message
    exit 1
}

function Get-Architecture {
    $arch = [System.Environment]::GetEnvironmentVariable("PROCESSOR_ARCHITECTURE")
    switch ($arch) {
        "AMD64" { return "x86_64" }
        "x86"   { Write-Err "32-bit Windows is not supported" }
        "ARM64" { Write-Err "ARM64 Windows is not yet supported" }
        default { Write-Err "Unknown architecture: $arch" }
    }
}

function Get-LatestVersion {
    $url = "https://api.github.com/repos/$Repo/releases"
    try {
        $releases = Invoke-RestMethod -Uri $url -Method Get -UseBasicParsing
        foreach ($release in $releases) {
            if (-not $release.prerelease) {
                return $release.tag_name
            }
        }
        Write-Err "No stable releases found. Check https://github.com/$Repo/releases"
    }
    catch {
        Write-Err "Failed to get latest version. Check https://github.com/$Repo/releases"
    }
}

function Install-Boop {
    Write-Info "Installing boop..."

    $arch = Get-Architecture
    $target = "$arch-pc-windows-msvc"

    Write-Info "Detected platform: $target"

    if ($env:BOOP_VERSION) {
        $version = $env:BOOP_VERSION
        if (-not $version.StartsWith("v")) {
            $version = "v$version"
        }
        Write-Info "Installing requested version: $version"
    } else {
        $version = Get-LatestVersion
        Write-Info "Latest version: $version"
    }

    $archiveName = "$BinaryName-$target.zip"
    $downloadUrl = "https://github.com/$Repo/releases/download/$version/$archiveName"

    $tempDir = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ([System.Guid]::NewGuid().ToString()))

    try {
        $archivePath = Join-Path $tempDir $archiveName

        Write-Info "Downloading $archiveName..."

        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

        try {
            Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath -UseBasicParsing
        }
        catch {
            Write-Err "Failed to download from $downloadUrl"
        }

        Write-Info "Extracting..."
        Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force

        if (!(Test-Path $InstallDir)) {
            New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        }

        Write-Info "Installing to $InstallDir..."
        Copy-Item -Path (Join-Path $tempDir "$BinaryName.exe") -Destination (Join-Path $InstallDir "$BinaryName.exe") -Force

        Write-Success "boop $version installed successfully!"

        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($userPath -notlike "*$InstallDir*") {
            Write-Warn "Adding $InstallDir to your PATH..."
            $newPath = "$userPath;$InstallDir"
            [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
            Write-Info "PATH updated. Please restart your terminal for changes to take effect."
        }
        else {
            Write-Info "Installation directory is already in your PATH"
        }

        Write-Host ""
        Write-Info "Get started with: boop --help"
    }
    finally {
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-Boop
