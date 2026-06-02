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
REM   scripts\start.bat backup       - Backup Docker volumes
REM   scripts\start.bat status       - Show running containers

setlocal enabledelayedexpansion

if "%1"=="" goto start
if "%1"=="start" goto start
if "%1"=="tunnel" goto tunnel
if "%1"=="stop" goto stop
if "%1"=="restart" goto restart
if "%1"=="logs" goto logs
if "%1"=="update" goto update
if "%1"=="build" goto build
if "%1"=="backup" goto backup
if "%1"=="status" goto status
if "%1"=="lowmem" goto lowmem
if "%1"=="verify-lightpanda" goto verify_lightpanda
if "%1"=="verify-gepa" goto verify_gepa
goto usage

:start
echo Starting AgentArk...
docker compose up -d
call :verify_lightpanda_async
call :verify_gepa_async
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

echo Starting AgentArk with remote access...
set AGENTARK_TUNNEL=true
docker compose up -d
call :verify_lightpanda_async
call :verify_gepa_async
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
echo Permanent tunnel tokens are stored inside AgentArk settings.
echo Open Web UI ^> Settings ^> Remote Access and paste the token there.
echo.
echo Starting AgentArk with permanent tunnel...
set AGENTARK_TUNNEL=true
docker compose up -d
echo.
echo AgentArk is running with your custom domain!
echo Check your Cloudflare dashboard for the URL.
goto end

:stop
echo Stopping AgentArk...
docker compose down
echo AgentArk stopped. Your data is preserved.
goto end

:restart
echo Restarting AgentArk...
docker compose restart agentark-control agentark-workspace agentark-executor agentark-embeddings
call :verify_lightpanda_async
call :verify_gepa_async
goto end

:logs
docker compose logs -f
goto end

:update
echo Updating AgentArk (your data will be preserved)...
docker compose pull postgres agentark-control agentark-embeddings agentark-executor agentark-workspace
docker compose up -d --build
call :verify_lightpanda_async
call :verify_gepa_async
echo Update complete! Your data is intact.
goto end

:build
echo Building AgentArk from this checkout and force-recreating containers (your data will be preserved)...
if "%AGENTARK_IMAGE%"=="" set AGENTARK_IMAGE=agentark:dev
docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d --build --force-recreate
call :verify_lightpanda_async
call :verify_gepa_async
echo Local build complete! Your data is intact.
goto end

:backup
for /f %%I in ('powershell -NoProfile -Command "Get-Date -Format yyyyMMdd_HHmmss"') do set BACKUP_STAMP=%%I
set "BACKUP_DIR=backups\%BACKUP_STAMP%"
mkdir "%BACKUP_DIR%" >nul 2>&1
echo Backing up AgentArk volumes to %BACKUP_DIR%...
call :backup_volume agentark-data agentark-data.tar.gz
if errorlevel 1 goto backup_failed
call :backup_volume agentark-config agentark-config.tar.gz
if errorlevel 1 goto backup_failed
call :backup_volume agentark-postgres-data agentark-postgres-data.tar.gz
if errorlevel 1 goto backup_failed
call :backup_volume agentark-secrets agentark-secrets.tar.gz
if errorlevel 1 goto backup_failed
echo Backup complete!
echo Keep agentark-secrets.tar.gz with the Postgres/config backups; it is required to unlock install-managed encrypted data.
goto end

:backup_failed
echo Backup failed.
goto end

:status
echo AgentArk Status:
docker compose ps
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
docker compose exec -T agentark-control sh -lc "command -v lightpanda >/dev/null 2>&1" >nul 2>&1
if %ERRORLEVEL% EQU 0 (
    echo Lightpanda is available inside the AgentArk runtime.
    exit /b 0
)
set /a LIGHTPANDA_RETRIES-=1
if %LIGHTPANDA_RETRIES% LEQ 0 (
    echo Lightpanda is missing from the bundled AgentArk runtime. Update or rebuild before relying on the free search fallback.
    exit /b 1
)
ping -n 2 127.0.0.1 >nul
goto verify_lightpanda_loop

:verify_lightpanda_async
start "" /b /d "%CD%" "%ComSpec%" /c call "%~f0" verify-lightpanda ^>nul 2^>^&1
exit /b 0

:verify_gepa
echo Verifying bundled GEPA optimizer runtime...
set "GEPA_RETRIES=20"
:verify_gepa_loop
docker compose exec -T agentark-control sh -lc "/opt/agentark-gepa/bin/python -c 'import dspy' >/dev/null 2>&1" >nul 2>&1
if %ERRORLEVEL% EQU 0 (
    echo GEPA optimizer is available inside the AgentArk runtime.
    exit /b 0
)
set /a GEPA_RETRIES-=1
if %GEPA_RETRIES% LEQ 0 (
    echo GEPA optimizer is missing from the bundled AgentArk runtime. Update or rebuild before running ArkEvolve GEPA.
    exit /b 1
)
ping -n 2 127.0.0.1 >nul
goto verify_gepa_loop

:verify_gepa_async
start "" /b /d "%CD%" "%ComSpec%" /c call "%~f0" verify-gepa ^>nul 2^>^&1
exit /b 0

:backup_volume
echo   %1 -^> %2
docker run --rm -v %1:/data:ro -v "%CD%\%BACKUP_DIR%":/backup alpine tar czf /backup/%2 -C /data .
exit /b %ERRORLEVEL%

:usage
echo Usage: scripts\start.bat [start^|tunnel^|stop^|restart^|logs^|update^|build^|backup^|status^|lowmem]
echo.
echo   start          Start AgentArk (local access only)
echo   tunnel         Start with remote access (auto-starts Cloudflare tunnel)
echo   tunnel setup   Set up permanent custom domain (free Cloudflare account)
echo   stop           Stop AgentArk
echo   restart        Restart AgentArk
echo   logs           View logs
echo   update         Pull latest image and restart (preserves data)
echo   build          Build from this checkout and restart
echo   backup         Backup Docker volumes
echo   status         Show running containers
echo   lowmem         Install low-memory config (2GB RAM / 2 CPUs) for Docker
goto end

:end
endlocal
