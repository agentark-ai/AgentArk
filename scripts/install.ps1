# AgentArk Docker installer for Windows.
#
# Usage:
#   irm https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.ps1 | iex

$ErrorActionPreference = "Stop"

$InstallDir = Join-Path $env:USERPROFILE "agentark"
$RuntimeDir = Join-Path $InstallDir "runtime"
$ReleaseRepo = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_RELEASE_REPO)) { "agentark-ai/AgentArk" } else { $env:AGENTARK_RELEASE_REPO.Trim() }
$ImageRepository = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_IMAGE_REPOSITORY)) { "ghcr.io/agentark-ai/agentark" } else { $env:AGENTARK_IMAGE_REPOSITORY.Trim() }
$RuntimeRef = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_RUNTIME_REF)) { "main" } else { $env:AGENTARK_RUNTIME_REF.Trim() }

function Test-AgentArkTruthyEnv {
    param([string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value)) { return $false }
    return @("1", "true", "yes", "y", "on") -contains $Value.Trim().ToLowerInvariant()
}

function Confirm-AgentArkAction {
    param([Parameter(Mandatory = $true)][string]$Prompt)
    if (Test-AgentArkTruthyEnv $env:AGENTARK_ASSUME_YES) { return $true }
    $answer = Read-Host $Prompt
    return @("y", "yes") -contains $answer.Trim().ToLowerInvariant()
}

function Get-AgentArkLatestReleaseTag {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$ReleaseRepo/releases/latest" -UseBasicParsing
    return [string]$release.tag_name
}

function Get-AgentArkReleaseVersionFromTag {
    param([string]$Tag)
    if ([string]::IsNullOrWhiteSpace($Tag)) { return "" }
    return $Tag.TrimStart("v", "V")
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
        if (Test-Path $candidate) { return $candidate }
    }
    return $null
}

function Test-AgentArkDockerEngine {
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) { return $false }
    & docker info *> $null
    return $LASTEXITCODE -eq 0
}

function Wait-AgentArkDockerEngine {
    param([int]$Attempts = 90)
    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        if (Test-AgentArkDockerEngine) { return $true }
        Start-Sleep -Seconds 2
    }
    return $false
}

function Install-AgentArkDockerDesktop {
    if (-not (Confirm-AgentArkAction "Docker Desktop is required. Install it now with winget? [y/N]")) {
        Write-Host "Install Docker Desktop manually, open it once, then rerun this installer:" -ForegroundColor Yellow
        Write-Host "  https://docs.docker.com/desktop/install/windows-install/" -ForegroundColor Cyan
        exit 1
    }

    $winget = Get-Command winget.exe -ErrorAction SilentlyContinue
    if (-not $winget) {
        Write-Host "winget is unavailable." -ForegroundColor Red
        Write-Host "Install Docker Desktop manually: https://docs.docker.com/desktop/install/windows-install/" -ForegroundColor Cyan
        exit 1
    }

    & winget install --id Docker.DockerDesktop -e --source winget --accept-package-agreements --accept-source-agreements
    if ($LASTEXITCODE -ne 0) {
        throw "Docker Desktop installation failed."
    }
    Add-AgentArkDockerCliPath
}

function Ensure-AgentArkDockerReady {
    Add-AgentArkDockerCliPath
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        Install-AgentArkDockerDesktop
    }
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        Write-Host "Docker was installed, but docker.exe is not visible in this shell yet." -ForegroundColor Yellow
        Write-Host "Open a new PowerShell after Docker Desktop finishes setup, then rerun this installer." -ForegroundColor Cyan
        exit 1
    }

    & docker compose version *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Docker Compose not found. Open Docker Desktop once, wait until it is running, then rerun this installer." -ForegroundColor Red
        exit 1
    }

    if (Test-AgentArkDockerEngine) { return }

    $exe = Resolve-AgentArkDockerDesktopExe
    if ([string]::IsNullOrWhiteSpace($exe)) {
        Write-Host "Docker is installed, but Docker Desktop was not found." -ForegroundColor Red
        Write-Host "Start Docker Desktop, wait until it is running, then rerun this installer." -ForegroundColor Cyan
        exit 1
    }

    Write-Host "Starting Docker Desktop..." -ForegroundColor Cyan
    Start-Process -FilePath $exe | Out-Null
    if (-not (Wait-AgentArkDockerEngine)) {
        Write-Host "Docker Desktop did not become ready in time." -ForegroundColor Red
        Write-Host "Open Docker Desktop, finish any setup prompts, then rerun this installer." -ForegroundColor Cyan
        exit 1
    }
}

function Write-AgentArkPortWarning {
    param([int]$Port, [string]$ServiceName)
    try {
        $listeners = Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction Stop
    } catch {
        $listeners = @()
    }
    if ($listeners.Count -gt 0) {
        Write-Host "Warning: TCP port $Port is already in use. $ServiceName may fail to start." -ForegroundColor Yellow
    }
}

function Save-AgentArkRuntimeFiles {
    $scriptsDir = Join-Path $RuntimeDir "scripts"
    New-Item -ItemType Directory -Path $scriptsDir -Force | Out-Null
    $rawBase = "https://raw.githubusercontent.com/$ReleaseRepo/$RuntimeRef"
    Invoke-WebRequest -Uri "$rawBase/docker-compose.yml" -OutFile (Join-Path $RuntimeDir "docker-compose.yml") -UseBasicParsing
    Invoke-WebRequest -Uri "$rawBase/scripts/start.bat" -OutFile (Join-Path $scriptsDir "start.bat") -UseBasicParsing
}

function Write-AgentArkCommand {
    param([Parameter(Mandatory = $true)][string]$TargetReleaseTag)

    $version = Get-AgentArkReleaseVersionFromTag $TargetReleaseTag
    $cmdWrapper = @"
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0agentark.ps1" %*
"@
    Set-Content -Path (Join-Path $InstallDir "agentark.cmd") -Value $cmdWrapper -Encoding ASCII

    $psWrapper = @"
`$ErrorActionPreference = "Stop"
`$AgentArkDir = Split-Path -Parent `$MyInvocation.MyCommand.Path
`$env:AGENTARK_RELEASE_REPO = if ([string]::IsNullOrWhiteSpace(`$env:AGENTARK_RELEASE_REPO)) { "$ReleaseRepo" } else { `$env:AGENTARK_RELEASE_REPO }
`$env:AGENTARK_RELEASE_TAG = if ([string]::IsNullOrWhiteSpace(`$env:AGENTARK_RELEASE_TAG)) { "$TargetReleaseTag" } else { `$env:AGENTARK_RELEASE_TAG }
`$env:AGENTARK_IMAGE = if ([string]::IsNullOrWhiteSpace(`$env:AGENTARK_IMAGE)) { "${ImageRepository}:$version" } else { `$env:AGENTARK_IMAGE }
if (`$args.Count -eq 0) { `$args = @("start") }
if (`$args[0].ToLowerInvariant() -eq "update") {
    irm "https://raw.githubusercontent.com/`$env:AGENTARK_RELEASE_REPO/main/scripts/install.ps1" | iex
    exit
}
Push-Location (Join-Path `$AgentArkDir "runtime")
try {
    & (Join-Path `$AgentArkDir "runtime\scripts\start.bat") @args
} finally {
    Pop-Location
}
"@
    Set-Content -Path (Join-Path $InstallDir "agentark.ps1") -Value $psWrapper -Encoding ASCII

    $userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($userPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("PATH", "$userPath;$InstallDir", "User")
        $env:PATH = "$env:PATH;$InstallDir"
        Write-Host "Added $InstallDir to your PATH." -ForegroundColor Green
    }
}

function Get-AgentArkComposeProjectName {
    if (-not [string]::IsNullOrWhiteSpace($env:COMPOSE_PROJECT_NAME)) {
        return $env:COMPOSE_PROJECT_NAME.Trim()
    }
    $existing = & docker ps -a --filter "name=^/agentark-control$" --format '{{.Label "com.docker.compose.project"}}' 2>$null | Select-Object -First 1
    if (-not [string]::IsNullOrWhiteSpace($existing)) {
        return $existing.Trim()
    }
    return "agentark"
}

Write-Host ""
Write-Host "=========================================" -ForegroundColor White
Write-Host "  AgentArk Installer" -ForegroundColor White
Write-Host "  Docker image install, no source clone." -ForegroundColor White
Write-Host "=========================================" -ForegroundColor White
Write-Host ""

Ensure-AgentArkDockerReady
Write-Host "[1/4] Docker is ready." -ForegroundColor Green

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
$TargetReleaseTag = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_RELEASE_TAG)) { Get-AgentArkLatestReleaseTag } else { $env:AGENTARK_RELEASE_TAG.Trim() }
if ([string]::IsNullOrWhiteSpace($TargetReleaseTag)) {
    throw "Unable to resolve the latest AgentArk release."
}

Save-AgentArkRuntimeFiles
Write-AgentArkCommand -TargetReleaseTag $TargetReleaseTag
Write-Host "[2/4] Runtime files ready at $RuntimeDir." -ForegroundColor Green

$env:AGENTARK_IMAGE = "${ImageRepository}:$(Get-AgentArkReleaseVersionFromTag $TargetReleaseTag)"
$env:AGENTARK_RELEASE_REPO = $ReleaseRepo
$env:AGENTARK_RELEASE_TAG = $TargetReleaseTag
$env:COMPOSE_PROJECT_NAME = Get-AgentArkComposeProjectName

$postgresPort = 5432
if ($env:AGENTARK_POSTGRES_PORT -match '^\d+$') { $postgresPort = [int]$env:AGENTARK_POSTGRES_PORT }
Write-AgentArkPortWarning -Port $postgresPort -ServiceName "Postgres"
Write-AgentArkPortWarning -Port 8990 -ServiceName "AgentArk Web UI"

Push-Location $RuntimeDir
try {
    Write-Host "[3/4] Pulling AgentArk image $env:AGENTARK_IMAGE..." -ForegroundColor Cyan
    & docker compose pull postgres agentark-control agentark-embeddings agentark-executor agentark-workspace
    if ($LASTEXITCODE -ne 0) { throw "Failed to pull AgentArk runtime images." }

    Write-Host "[4/4] Starting AgentArk..." -ForegroundColor Green
    & docker compose up -d
    if ($LASTEXITCODE -ne 0) { throw "Failed to start AgentArk." }
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "=========================================" -ForegroundColor White
Write-Host "  AgentArk is running!" -ForegroundColor Green
Write-Host "=========================================" -ForegroundColor White
Write-Host ""
Write-Host "  Web UI:  http://localhost:8990" -ForegroundColor Cyan
Write-Host "  Image:   $env:AGENTARK_IMAGE" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Commands:" -ForegroundColor White
Write-Host "    agentark logs"
Write-Host "    agentark status"
Write-Host "    agentark stop"
Write-Host "    agentark update"
Write-Host ""
Write-Host "  App data is stored in Docker volumes and survives updates." -ForegroundColor Yellow
