# AgentArk Installer for Windows
#
# Usage: irm https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.ps1 | iex

$ErrorActionPreference = "Stop"

$InstallDir = Join-Path $env:USERPROFILE "agentark"
$SourceDir = Join-Path $InstallDir "source"
$ReleaseRepo = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_RELEASE_REPO)) { "agentark-ai/AgentArk" } else { $env:AGENTARK_RELEASE_REPO.Trim() }
$RepoUrl = "https://github.com/$ReleaseRepo.git"
$ImageRepository = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_IMAGE_REPOSITORY)) { "ghcr.io/agentark-ai/agentark" } else { $env:AGENTARK_IMAGE_REPOSITORY.Trim() }
$LocalSourceImage = "agentark:dev"

function Get-AgentArkLatestReleaseTag {
    $refs = & docker run --rm alpine/git ls-remote --tags --refs $RepoUrl "v*" 2>$null
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($refs)) {
        return $null
    }

    $tags = $refs |
        ForEach-Object {
            $parts = ($_ -split '\s+')
            if ($parts.Length -gt 1) { $parts[-1] -replace '^refs/tags/', '' } else { $null }
        } |
        Where-Object { $_ -match '^v\d+\.\d+\.\d+$' }

    if (-not $tags) {
        return $null
    }

    return $tags |
        Sort-Object { [version]($_.Substring(1)) } |
        Select-Object -Last 1
}

function Get-AgentArkReleaseVersionFromTag {
    param([string]$Tag)
    if ([string]::IsNullOrWhiteSpace($Tag)) {
        return ""
    }
    return $Tag.TrimStart("v", "V")
}

function Assert-AgentArkCleanCheckout {
    $status = & docker run --rm -v "${InstallDir}:/work" -w /work alpine/git git -C /work/source status --porcelain --untracked-files=no 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to inspect the AgentArk source checkout."
    }
    if (-not [string]::IsNullOrWhiteSpace(($status | Out-String))) {
        throw "Tracked local changes were found in $SourceDir. Resolve them before reinstalling."
    }
}

function Write-AgentArkPortWarning {
    param(
        [int]$Port,
        [string]$ServiceName
    )

    try {
        $listeners = Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction Stop
    } catch {
        $listeners = @()
    }

    if ($listeners.Count -gt 0) {
        Write-Host "Warning: TCP port $Port is already in use. $ServiceName may fail to start unless you stop the existing listener or override the port." -ForegroundColor Yellow
    }
}

function Test-AgentArkTruthyEnv {
    param([string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value)) {
        return $false
    }
    return @("1", "true", "yes", "y", "on") -contains $Value.Trim().ToLowerInvariant()
}

function Confirm-AgentArkAction {
    param([Parameter(Mandatory = $true)][string]$Prompt)

    if (Test-AgentArkTruthyEnv $env:AGENTARK_ASSUME_YES) {
        return $true
    }

    $answer = Read-Host $Prompt
    return @("y", "yes") -contains $answer.Trim().ToLowerInvariant()
}

function Get-AgentArkWslReadiness {
    $wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
    if (-not $wsl) {
        return [pscustomobject]@{
            Ready = $false
            Detail = "wsl.exe was not found."
        }
    }

    $output = & wsl.exe --status 2>&1
    $detail = ($output | Out-String).Trim()
    return [pscustomobject]@{
        Ready = ($LASTEXITCODE -eq 0)
        Detail = $detail
    }
}

function Write-AgentArkWslHelp {
    Write-Host "Docker Desktop uses the WSL 2 backend for AgentArk's Linux containers." -ForegroundColor Yellow
    Write-Host "If Docker Desktop reports that WSL is missing or outdated, run this from an elevated PowerShell:" -ForegroundColor Yellow
    Write-Host "  wsl --install --no-distribution" -ForegroundColor Cyan
    Write-Host "Then reboot, open Docker Desktop once, and rerun this installer." -ForegroundColor Yellow
}

function Add-AgentArkDockerCliPath {
    $paths = @()
    if (-not [string]::IsNullOrWhiteSpace($env:ProgramFiles)) {
        $paths += (Join-Path $env:ProgramFiles "Docker\Docker\resources\bin")
    }
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    if (-not [string]::IsNullOrWhiteSpace($programFilesX86)) {
        $paths += (Join-Path $programFilesX86 "Docker\Docker\resources\bin")
    }

    foreach ($path in $paths) {
        if ((Test-Path $path) -and ($env:PATH -notlike "*$path*")) {
            $env:PATH = "$env:PATH;$path"
        }
    }
}

function Resolve-AgentArkDockerDesktopExe {
    $candidates = @()
    if (-not [string]::IsNullOrWhiteSpace($env:ProgramFiles)) {
        $candidates += (Join-Path $env:ProgramFiles "Docker\Docker\Docker Desktop.exe")
    }
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    if (-not [string]::IsNullOrWhiteSpace($programFilesX86)) {
        $candidates += (Join-Path $programFilesX86 "Docker\Docker\Docker Desktop.exe")
    }
    if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        $candidates += (Join-Path $env:LOCALAPPDATA "Docker\Docker Desktop.exe")
    }

    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            return $candidate
        }
    }
    return $null
}

function Install-AgentArkDockerDesktop {
    $winget = Get-Command winget.exe -ErrorAction SilentlyContinue
    if (-not $winget) {
        Write-Host "Docker not found and winget is unavailable." -ForegroundColor Red
        Write-Host "Install Docker Desktop manually: https://docs.docker.com/desktop/install/windows-install/" -ForegroundColor Cyan
        exit 1
    }

    if (-not (Confirm-AgentArkAction "Docker Desktop is required. Install it now with winget? [y/N]")) {
        Write-Host "Install Docker Desktop manually, open it once, then rerun this installer:" -ForegroundColor Yellow
        Write-Host "  https://docs.docker.com/desktop/install/windows-install/" -ForegroundColor Cyan
        exit 1
    }

    Write-Host "Installing Docker Desktop with winget..." -ForegroundColor Cyan
    & winget install --id Docker.DockerDesktop -e --source winget --accept-package-agreements --accept-source-agreements
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Docker Desktop installation failed." -ForegroundColor Red
        Write-Host "Install it manually, open it once, then rerun this installer:" -ForegroundColor Yellow
        Write-Host "  https://docs.docker.com/desktop/install/windows-install/" -ForegroundColor Cyan
        exit 1
    }

    Add-AgentArkDockerCliPath
}

function Test-AgentArkDockerEngine {
    $docker = Get-Command docker -ErrorAction SilentlyContinue
    if (-not $docker) {
        return $false
    }
    & docker info *> $null
    return $LASTEXITCODE -eq 0
}

function Start-AgentArkDockerDesktop {
    $exe = Resolve-AgentArkDockerDesktopExe
    if ([string]::IsNullOrWhiteSpace($exe)) {
        return $false
    }

    Write-Host "Starting Docker Desktop..." -ForegroundColor Cyan
    Start-Process -FilePath $exe | Out-Null
    return $true
}

function Wait-AgentArkDockerEngine {
    param([int]$Attempts = 90)

    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        if (Test-AgentArkDockerEngine) {
            return $true
        }
        Start-Sleep -Seconds 2
    }
    return $false
}

function Ensure-AgentArkDockerReady {
    $wslStatus = Get-AgentArkWslReadiness
    if ($wslStatus.Ready) {
        Write-Host "[1/5] WSL available." -ForegroundColor Green
    } else {
        Write-Host "[1/5] WSL is not ready." -ForegroundColor Yellow
        if (-not [string]::IsNullOrWhiteSpace($wslStatus.Detail)) {
            Write-Host $wslStatus.Detail -ForegroundColor DarkYellow
        }
        Write-AgentArkWslHelp
    }

    Add-AgentArkDockerCliPath
    $docker = Get-Command docker -ErrorAction SilentlyContinue
    if (-not $docker) {
        Write-Host "Docker not found." -ForegroundColor Yellow
        Install-AgentArkDockerDesktop
        $docker = Get-Command docker -ErrorAction SilentlyContinue
    }

    if (-not $docker) {
        Write-Host "Docker Desktop was installed, but docker.exe is not visible in this shell yet." -ForegroundColor Yellow
        Write-Host "Open a new PowerShell after Docker Desktop finishes setup, then rerun this installer." -ForegroundColor Cyan
        exit 1
    }
    Write-Host "[2/5] Docker CLI found." -ForegroundColor Green

    $composeCheck = docker compose version 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Docker Compose not found. Docker Desktop may not have finished setup." -ForegroundColor Red
        Write-Host "Open Docker Desktop once, wait until it says it is running, then rerun this installer." -ForegroundColor Cyan
        exit 1
    }
    Write-Host "[3/5] Docker Compose found." -ForegroundColor Green

    if (-not (Test-AgentArkDockerEngine)) {
        if (Start-AgentArkDockerDesktop) {
            if (-not (Wait-AgentArkDockerEngine)) {
                Write-Host "Docker Desktop did not become ready in time." -ForegroundColor Red
                Write-AgentArkWslHelp
                exit 1
            }
        } else {
            Write-Host "Docker is installed, but the Docker engine is not running." -ForegroundColor Red
            Write-Host "Start Docker Desktop, wait until it is running, then rerun this installer." -ForegroundColor Cyan
            Write-AgentArkWslHelp
            exit 1
        }
    }
    Write-Host "Docker engine is running." -ForegroundColor Green
}

function Select-AgentArkInstallKind {
    Write-Host ""
    Write-Host "Choose install method:" -ForegroundColor White
    Write-Host "  1. Fast install - download the published AgentArk image (recommended)" -ForegroundColor Green
    Write-Host "  2. Source build - clone AgentArk and build the local image on this machine" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Source build avoids pulling the AgentArk image from GHCR, but it is slower and still downloads Docker build base images and package dependencies." -ForegroundColor Yellow

    if (Test-AgentArkTruthyEnv $env:AGENTARK_ASSUME_YES) {
        Write-Host "Using fast install because AGENTARK_ASSUME_YES is set." -ForegroundColor Green
        return "image"
    }

    while ($true) {
        $choice = Read-Host "Install method [1]"
        $normalized = $choice.Trim().ToLowerInvariant()
        if ([string]::IsNullOrWhiteSpace($normalized) -or $normalized -eq "1" -or $normalized -eq "fast" -or $normalized -eq "image") {
            return "image"
        }
        if ($normalized -eq "2" -or $normalized -eq "source" -or $normalized -eq "build") {
            return "source"
        }
        Write-Host "Choose 1 for fast install or 2 for source build." -ForegroundColor Yellow
    }
}

function Invoke-AgentArkRuntimeImagePull {
    & docker compose pull postgres agentark-control agentark-embeddings agentark-executor agentark-workspace
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to pull AgentArk runtime images."
    }
}

Write-Host ""
Write-Host "=========================================" -ForegroundColor White
Write-Host "  AgentArk Installer" -ForegroundColor White
Write-Host "  Think. Act. Remember. Securely." -ForegroundColor White
Write-Host "=========================================" -ForegroundColor White
Write-Host ""

Ensure-AgentArkDockerReady
$InstallKind = Select-AgentArkInstallKind

if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$TargetReleaseTag = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_RELEASE_TAG)) { Get-AgentArkLatestReleaseTag } else { $env:AGENTARK_RELEASE_TAG.Trim() }
if ([string]::IsNullOrWhiteSpace($TargetReleaseTag)) {
    throw "Unable to resolve the latest tagged AgentArk release."
}

if (-not (Test-Path (Join-Path $SourceDir ".git"))) {
    Write-Host "Cloning AgentArk $TargetReleaseTag into $SourceDir..." -ForegroundColor Cyan
    & docker run --rm -v "${InstallDir}:/work" -w /work alpine/git clone --branch $TargetReleaseTag --depth 1 $RepoUrl source
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to clone the AgentArk release checkout."
    }
} else {
    Write-Host "Existing source checkout found at $SourceDir" -ForegroundColor Green
    Assert-AgentArkCleanCheckout
    & docker run --rm -v "${InstallDir}:/work" -w /work alpine/git git -C /work/source fetch --tags --force origin
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to fetch AgentArk release tags."
    }
    & docker run --rm -v "${InstallDir}:/work" -w /work alpine/git git -C /work/source checkout --force $TargetReleaseTag
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to switch the AgentArk checkout to $TargetReleaseTag."
    }
}

if (-not (Test-Path (Join-Path $SourceDir "docker-compose.yml"))) {
    throw "Missing $SourceDir\docker-compose.yml after checkout."
}

Write-Host "[4/5] Source checkout ready at $SourceDir" -ForegroundColor Green

$cmdWrapper = @'
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0source\scripts\agentark-release-cli.ps1" %*
'@
Set-Content -Path (Join-Path $InstallDir "agentark.cmd") -Value $cmdWrapper -Encoding ASCII

$userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("PATH", "$userPath;$InstallDir", "User")
    $env:PATH = "$env:PATH;$InstallDir"
    Write-Host "Added $InstallDir to your PATH." -ForegroundColor Green
}

if ($InstallKind -eq "source") {
    Write-Host "Building AgentArk from source checkout $TargetReleaseTag..." -ForegroundColor Cyan
} else {
    Write-Host "Downloading AgentArk container image for $TargetReleaseTag..." -ForegroundColor Cyan
}
$postgresPort = 5432
if ($env:AGENTARK_POSTGRES_PORT -match '^\d+$') {
    $postgresPort = [int]$env:AGENTARK_POSTGRES_PORT
}
Write-AgentArkPortWarning -Port $postgresPort -ServiceName "Postgres"
Write-AgentArkPortWarning -Port 8990 -ServiceName "AgentArk Web UI"

$previousImage = $env:AGENTARK_IMAGE
$previousRepo = $env:AGENTARK_RELEASE_REPO
$previousTag = $env:AGENTARK_RELEASE_TAG
if ($InstallKind -eq "source") {
    $env:AGENTARK_IMAGE = $LocalSourceImage
} else {
    $env:AGENTARK_IMAGE = "${ImageRepository}:$(Get-AgentArkReleaseVersionFromTag $TargetReleaseTag)"
}
$env:AGENTARK_RELEASE_REPO = $ReleaseRepo
$env:AGENTARK_RELEASE_TAG = $TargetReleaseTag

Push-Location $SourceDir
try {
    Write-Host "[5/5] Starting AgentArk..." -ForegroundColor Green
    if ($InstallKind -eq "source") {
        & docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d --build --force-recreate
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to build and start AgentArk from source."
        }
    } else {
        Invoke-AgentArkRuntimeImagePull
        & docker compose up -d
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to start AgentArk."
        }
    }

    $lightpandaReady = $false
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        & docker compose exec -T agentark-control sh -lc "command -v lightpanda >/dev/null 2>&1" *> $null
        if ($LASTEXITCODE -eq 0) {
            $lightpandaReady = $true
            break
        }
        Start-Sleep -Seconds 2
    }
    if (-not $lightpandaReady) {
        throw "Lightpanda is missing from the bundled AgentArk runtime. Update or rebuild before relying on the free search fallback."
    }

    $gepaReady = $false
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        & docker compose exec -T agentark-control sh -lc "/opt/agentark-gepa/bin/python -c 'import dspy' >/dev/null 2>&1" *> $null
        if ($LASTEXITCODE -eq 0) {
            $gepaReady = $true
            break
        }
        Start-Sleep -Seconds 2
    }
    if (-not $gepaReady) {
        throw "GEPA optimizer is missing from the bundled AgentArk runtime. Update or rebuild before running ArkEvolve GEPA."
    }
} finally {
    Pop-Location
    if ($null -eq $previousImage) { Remove-Item Env:\AGENTARK_IMAGE -ErrorAction SilentlyContinue } else { $env:AGENTARK_IMAGE = $previousImage }
    if ($null -eq $previousRepo) { Remove-Item Env:\AGENTARK_RELEASE_REPO -ErrorAction SilentlyContinue } else { $env:AGENTARK_RELEASE_REPO = $previousRepo }
    if ($null -eq $previousTag) { Remove-Item Env:\AGENTARK_RELEASE_TAG -ErrorAction SilentlyContinue } else { $env:AGENTARK_RELEASE_TAG = $previousTag }
}

Write-Host ""
Write-Host "=========================================" -ForegroundColor White
Write-Host "  AgentArk is running!" -ForegroundColor Green
Write-Host "=========================================" -ForegroundColor White
Write-Host ""
Write-Host "  Web UI:  http://localhost:8990" -ForegroundColor Cyan
if ($InstallKind -eq "source") {
    Write-Host "  Install: source build using local image $LocalSourceImage" -ForegroundColor Cyan
} else {
    Write-Host "  Install: published image pinned to $TargetReleaseTag" -ForegroundColor Cyan
}
Write-Host ""
Write-Host "  Commands (run from anywhere):" -ForegroundColor White
Write-Host "    agentark chat       Interactive CLI chat"
Write-Host "    agentark pulse      Run ArkPulse health check"
Write-Host "    agentark stop       Stop AgentArk"
Write-Host "    agentark update     Install the latest tagged release and restart"
Write-Host "    agentark logs       View logs"
Write-Host "    agentark status     Show status"
Write-Host "    agentark backup     Backup Docker volumes"
Write-Host ""
Write-Host "  App data is stored in Docker volumes and survives updates." -ForegroundColor Yellow
Write-Host "  Postgres and install secrets live in Docker volumes; use 'agentark backup' before moving installs." -ForegroundColor Yellow
Write-Host "  Use 'docker compose down -v' only for a full reset." -ForegroundColor Yellow
Write-Host ""
