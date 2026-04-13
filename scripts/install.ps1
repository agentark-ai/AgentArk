# AgentArk Installer for Windows
# Think. Act. Remember. Securely.
#
# Usage: irm https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.ps1 | iex

$ErrorActionPreference = "Stop"

$InstallDir = "$env:USERPROFILE\agentark"
$SourceDir = Join-Path $InstallDir "source"
$RepoUrl = "https://github.com/agentark-ai/AgentArk.git"

Write-Host ""
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor White
Write-Host "  AgentArk Installer" -ForegroundColor White
Write-Host "  Think. Act. Remember. Securely."
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor White
Write-Host ""

# ── Step 1: Check Docker ────────────────────────────────────────────────────

$docker = Get-Command docker -ErrorAction SilentlyContinue
if (-not $docker) {
    Write-Host "Docker not found." -ForegroundColor Red
    Write-Host "Please install Docker Desktop: https://docs.docker.com/desktop/install/windows-install/" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "After installing, restart this terminal and run the command again." -ForegroundColor Yellow
    exit 1
}
Write-Host "[1/4] Docker found." -ForegroundColor Green

# Verify docker compose
$composeCheck = docker compose version 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "Docker Compose not found. Please install Docker Desktop (includes Compose)." -ForegroundColor Red
    exit 1
}
Write-Host "[2/4] Docker Compose found." -ForegroundColor Green

# ── Step 2: Create install directory ────────────────────────────────────────

if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

if (-not (Test-Path (Join-Path $SourceDir ".git"))) {
    Write-Host "Cloning AgentArk source into $SourceDir..." -ForegroundColor Cyan
    docker run --rm -v "${InstallDir}:/work" -w /work alpine/git clone --depth 1 $RepoUrl source
} else {
    Write-Host "Existing source checkout found at $SourceDir" -ForegroundColor Green
}

# ── Step 3: Verify source checkout ───────────────────────────────────────────

$ComposeFile = Join-Path $SourceDir "docker-compose.yml"
if (-not (Test-Path $ComposeFile)) {
    Write-Host "Missing $ComposeFile after clone." -ForegroundColor Red
    exit 1
}
Write-Host "[3/4] Source checkout ready at $SourceDir" -ForegroundColor Green

# ── Step 4: Create agentark.cmd CLI wrapper ───────────────────────────────────

$cliContent = @'
@echo off
REM AgentArk CLI — simple commands for your AI agent
REM Usage: agentark chat | pulse | start | stop | logs | status | update

set "CMD=%~1"
if "%CMD%"=="" set "CMD=help"

if "%CMD%"=="chat" (
    docker exec -it agentark-control /app/agentark --chat
    goto :eof
)
if "%CMD%"=="pulse" (
    docker exec agentark-control /app/agentark --pulse
    goto :eof
)
if "%CMD%"=="start" (
    docker compose -f "%~dp0source\docker-compose.yml" up -d
    echo.
    echo AgentArk is running!
    echo   Web UI: http://localhost:8990
    goto :eof
)
if "%CMD%"=="stop" (
    docker compose -f "%~dp0source\docker-compose.yml" down
    echo Stopped. Your data is preserved.
    goto :eof
)
if "%CMD%"=="restart" (
    docker compose -f "%~dp0source\docker-compose.yml" down
    docker compose -f "%~dp0source\docker-compose.yml" up -d
    goto :eof
)
if "%CMD%"=="logs" (
    docker compose -f "%~dp0source\docker-compose.yml" logs -f --tail=100
    goto :eof
)
if "%CMD%"=="status" (
    docker compose -f "%~dp0source\docker-compose.yml" ps
    goto :eof
)
if "%CMD%"=="update" (
    docker run --rm -v "%~dp0:/work" -w /work alpine/git git -C /work/source pull --ff-only
    docker compose -f "%~dp0source\docker-compose.yml" pull
    docker compose -f "%~dp0source\docker-compose.yml" up -d
    echo Update complete!
    goto :eof
)
if "%CMD%"=="setup" (
    docker exec -it agentark-control /app/agentark --setup
    goto :eof
)

echo AgentArk CLI
echo.
echo Usage: agentark ^<command^>
echo.
echo   chat       Interactive CLI chat with your agent
echo   pulse      Run ArkPulse health check
echo   start      Start AgentArk
echo   stop       Stop AgentArk
echo   restart    Restart AgentArk
echo   logs       View live logs
echo   status     Show running containers
echo   update     Pull latest image and restart
echo   setup      Run setup wizard
'@

Set-Content -Path "$InstallDir\agentark.cmd" -Value $cliContent -Encoding ASCII

# Add install dir to user PATH so 'agentark' works from anywhere
$userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("PATH", "$userPath;$InstallDir", "User")
    $env:PATH = "$env:PATH;$InstallDir"
    Write-Host "Added $InstallDir to your PATH." -ForegroundColor Green
    Write-Host "  (Open a new terminal if 'agentark' isn't recognized immediately)" -ForegroundColor Yellow
}

Write-Host "CLI installed." -ForegroundColor Green

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

# ── Step 5: Build and start ────────────────────────────────────────────────

Write-Host "Downloading AgentArk container image (first run may take a few minutes)..." -ForegroundColor Cyan
$postgresPort = 5432
if ($env:AGENTARK_POSTGRES_PORT -match '^\d+$') {
    $postgresPort = [int]$env:AGENTARK_POSTGRES_PORT
}
Write-AgentArkPortWarning -Port $postgresPort -ServiceName "Postgres"
Write-AgentArkPortWarning -Port 8990 -ServiceName "AgentArk Web UI"
Push-Location $InstallDir
try {
    Write-Host "[4/4] Starting AgentArk..." -ForegroundColor Green
    Push-Location $SourceDir
    try {
        docker compose pull
        docker compose up -d
    } finally {
        Pop-Location
    }
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor White
Write-Host "  AgentArk is running!" -ForegroundColor Green
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor White
Write-Host ""
Write-Host "  Web UI:  http://localhost:8990" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Commands (run from anywhere):" -ForegroundColor White
Write-Host "    agentark chat       Interactive CLI chat"
Write-Host "    agentark pulse      Run ArkPulse health check"
Write-Host "    agentark stop       Stop AgentArk"
Write-Host "    agentark update     Pull latest image and restart"
Write-Host "    agentark logs       View logs"
Write-Host "    agentark status     Show status"
Write-Host ""
Write-Host "  App data is stored in Docker volumes and survives updates." -ForegroundColor Yellow
Write-Host "  Postgres has its own volume; use 'docker compose down -v' to reset everything." -ForegroundColor Yellow
Write-Host ""
