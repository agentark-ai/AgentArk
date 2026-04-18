@echo off
REM AgentArk Easy Start Script for Windows
REM
REM Commands:
REM   scripts\start.bat              - Start AgentArk (local access only)
REM   scripts\start.bat tunnel       - Start with remote access (tunnel managed from Web UI)
REM   scripts\start.bat tunnel setup - Set up permanent custom domain (free Cloudflare account)
REM   scripts\start.bat stop         - Stop AgentArk
REM   scripts\start.bat restart      - Restart AgentArk
REM   scripts\start.bat logs         - View logs
REM   scripts\start.bat update       - Pull latest image and restart (preserves data)
REM   scripts\start.bat build        - Build from this checkout and restart
REM   scripts\start.bat status       - Show running containers

setlocal enabledelayedexpansion

set "AGENTARK_LOCAL_ENV=.agentark\local.env"

if "%1"=="" goto start
if "%1"=="start" goto start
if "%1"=="tunnel" goto tunnel
if "%1"=="stop" goto stop
if "%1"=="restart" goto restart
if "%1"=="logs" goto logs
if "%1"=="update" goto update
if "%1"=="build" goto build
if "%1"=="status" goto status
if "%1"=="lowmem" goto lowmem
goto usage

:start
call :ensure_postgres_password || exit /b 1
echo Starting AgentArk...
docker compose --env-file "%AGENTARK_LOCAL_ENV%" up -d
call :verify_lightpanda || exit /b 1
echo.
echo AgentArk is running!
echo   Web UI:  http://localhost:8990
echo.
echo Your data is safely stored in Docker volumes.
echo.
echo Want to access from anywhere? Enable the tunnel from the Web UI
echo   or run: scripts\start.bat tunnel
goto end

:tunnel
if "%2"=="setup" goto tunnel_setup

call :ensure_postgres_password || exit /b 1
echo Starting AgentArk with remote access...
set AGENTARK_TUNNEL=true
docker compose --env-file "%AGENTARK_LOCAL_ENV%" up -d
call :verify_lightpanda || exit /b 1
echo.
echo AgentArk is starting with secure tunnel!
echo.
echo   Local:   http://localhost:8990
echo   Remote:  Your Cloudflare URL will appear in the Web UI
echo.
echo   You can also manage the tunnel from:
echo     Web UI ^> Settings ^> Remote Access
echo.
echo All traffic is encrypted. API key protects all endpoints.
goto end

:tunnel_setup
echo.
echo === Permanent Custom Domain Setup ===
echo.
echo This gives you a permanent URL like agent.yourdomain.com
echo instead of a random URL that changes on restart.
echo.
echo Setup (5 minutes, free^):
echo.
echo   1. Go to https://one.dash.cloudflare.com
echo   2. Sign up / log in (free plan works^)
echo   3. Go to: Networks ^> Tunnels ^> Create a tunnel
echo   4. Name it "agentark"
echo   5. Copy the tunnel token
echo   6. Add a public hostname pointing to: http://localhost:8990
echo.
set /p TOKEN="Paste your Tunnel Token here (or press Enter to cancel): "
echo.
if "!TOKEN!"=="" (
    echo Cancelled. You can run this again anytime.
    goto end
)
call :upsert_managed_env TUNNEL_TOKEN "!TOKEN!" || exit /b 1
echo Token saved to %AGENTARK_LOCAL_ENV%
echo.
call :ensure_postgres_password || exit /b 1
echo Starting AgentArk with permanent tunnel...
set AGENTARK_TUNNEL=true
docker compose --env-file "%AGENTARK_LOCAL_ENV%" up -d
echo.
echo AgentArk is running with your custom domain!
echo Check your Cloudflare dashboard for the URL.
goto end

:stop
echo Stopping AgentArk...
docker compose --env-file "%AGENTARK_LOCAL_ENV%" down
echo AgentArk stopped. Your data is preserved.
goto end

:restart
echo Restarting AgentArk...
docker compose --env-file "%AGENTARK_LOCAL_ENV%" restart agentark-control agentark-workspace agentark-executor
call :verify_lightpanda || exit /b 1
goto end

:logs
docker compose --env-file "%AGENTARK_LOCAL_ENV%" logs -f
goto end

:update
call :ensure_postgres_password || exit /b 1
echo Updating AgentArk (your data will be preserved)...
docker compose --env-file "%AGENTARK_LOCAL_ENV%" pull
docker compose --env-file "%AGENTARK_LOCAL_ENV%" up -d
call :verify_lightpanda || exit /b 1
echo Update complete! Your data is intact.
goto end

:build
call :ensure_postgres_password || exit /b 1
echo Building AgentArk from this checkout and force-recreating containers (your data will be preserved)...
if "%AGENTARK_IMAGE%"=="" set AGENTARK_IMAGE=agentark:dev
docker compose --env-file "%AGENTARK_LOCAL_ENV%" -f docker-compose.yml -f docker-compose.dev.yml up -d --build --force-recreate
call :verify_lightpanda || exit /b 1
echo Local build complete! Your data is intact.
goto end

:status
echo AgentArk Status:
docker compose --env-file "%AGENTARK_LOCAL_ENV%" ps
goto end

:lowmem
echo.
echo === Low-Memory Build Setup ===
echo.
echo This limits Docker Desktop to 2GB RAM + 2 CPUs for building on low-spec machines.
echo.
if exist "%USERPROFILE%\.wslconfig" (
    echo WARNING: %USERPROFILE%\.wslconfig already exists.
    set /p OVERWRITE="Overwrite? (y/N): "
    if /i not "!OVERWRITE!"=="y" (
        echo Cancelled.
        goto end
    )
)
copy /y "%~dp0low-memory-build.wslconfig" "%USERPROFILE%\.wslconfig" >nul
echo Installed .wslconfig to %USERPROFILE%\.wslconfig
echo Restarting WSL2...
wsl --shutdown
echo.
echo Done! Docker Desktop will use 2GB RAM / 2 CPUs / 4GB swap.
echo Now run: scripts\start.bat
echo.
echo To restore full resources later:
echo   del %USERPROFILE%\.wslconfig
echo   wsl --shutdown
goto end

:verify_lightpanda
echo Verifying bundled Lightpanda runtime...
set "LIGHTPANDA_RETRIES=20"
:verify_lightpanda_loop
docker compose --env-file "%AGENTARK_LOCAL_ENV%" exec -T agentark-control sh -lc "command -v lightpanda >/dev/null 2>&1" >nul 2>&1
if %ERRORLEVEL% EQU 0 (
    echo Lightpanda is available inside the AgentArk runtime.
    exit /b 0
)
set /a LIGHTPANDA_RETRIES-=1
if %LIGHTPANDA_RETRIES% LEQ 0 (
    echo Lightpanda is missing from the bundled AgentArk runtime. Update or rebuild before relying on the free search fallback.
    exit /b 1
)
timeout /t 2 >nul
goto verify_lightpanda_loop

:ensure_postgres_password
set "PGPASS="
for /f "usebackq delims=" %%P in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "$dir='.agentark'; $envPath=Join-Path $dir 'local.env'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; function ReadValue($path,$key){ if(Test-Path $path){ foreach($line in Get-Content $path){ if($line -match ('^' + [regex]::Escape($key) + '=(.*)$')){ return $matches[1] } } } return $null }; $value=ReadValue $envPath 'AGENTARK_POSTGRES_PASSWORD'; if([string]::IsNullOrWhiteSpace($value)){ $value=ReadValue '.env' 'AGENTARK_POSTGRES_PASSWORD' }; if([string]::IsNullOrWhiteSpace($value)){ $rng=[Security.Cryptography.RandomNumberGenerator]::Create(); $bytes=New-Object byte[] 24; $rng.GetBytes($bytes); $value=(($bytes | ForEach-Object { $_.ToString('x2') }) -join ''); $rng.Dispose() }; $lines=@(); if(Test-Path $envPath){ $lines=Get-Content $envPath | Where-Object { $_ -notmatch '^AGENTARK_POSTGRES_PASSWORD=' } }; $lines += ('AGENTARK_POSTGRES_PASSWORD=' + $value); Set-Content -Path $envPath -Value $lines; Write-Output $value"`) do set "PGPASS=%%P"
if not defined PGPASS (
    echo Failed to generate a local Postgres password.
    exit /b 1
)
echo Local Postgres password is managed in %AGENTARK_LOCAL_ENV%
exit /b 0

:upsert_managed_env
set "AA_ENV_KEY=%~1"
set "AA_ENV_VALUE=%~2"
powershell -NoProfile -ExecutionPolicy Bypass -Command "$envPath='.agentark\local.env'; New-Item -ItemType Directory -Force -Path (Split-Path $envPath) | Out-Null; $key=$env:AA_ENV_KEY; $value=$env:AA_ENV_VALUE; $lines=@(); if(Test-Path $envPath){ $lines=Get-Content $envPath | Where-Object { $_ -notmatch ('^' + [regex]::Escape($key) + '=') } }; $lines += ($key + '=' + $value); Set-Content -Path $envPath -Value $lines"
exit /b %ERRORLEVEL%

:usage
echo Usage: scripts\start.bat [start^|tunnel^|stop^|restart^|logs^|update^|build^|status^|lowmem]
echo.
echo   start          Start AgentArk (local access only)
echo   tunnel         Start with remote access (auto-starts Cloudflare tunnel)
echo   tunnel setup   Set up permanent custom domain (free Cloudflare account)
echo   stop           Stop AgentArk
echo   restart        Restart AgentArk
echo   logs           View logs
echo   update         Pull latest image and restart (preserves data)
echo   build          Build from this checkout and restart
echo   status         Show running containers
echo   lowmem         Install low-memory config (2GB RAM / 2 CPUs) for Docker
goto end

:end
endlocal
