@echo off
REM AgentArk CLI wrapper for Windows
REM Usage: agentark chat | pulse | start | stop | logs | status

set "CMD=%~1"
if "%CMD%"=="" set "CMD=help"

if "%CMD%"=="chat" (
    docker exec -it agentark /app/agentark --chat
    goto :eof
)
if "%CMD%"=="pulse" (
    docker exec agentark /app/agentark --pulse
    goto :eof
)
if "%CMD%"=="start" (
    docker compose up -d --build
    echo.
    echo AgentArk is running!
    echo   Web UI: http://localhost:8990
    goto :eof
)
if "%CMD%"=="stop" (
    docker compose down
    goto :eof
)
if "%CMD%"=="restart" (
    docker compose down
    docker compose up -d
    goto :eof
)
if "%CMD%"=="logs" (
    docker compose logs -f --tail=100
    goto :eof
)
if "%CMD%"=="status" (
    docker compose ps
    goto :eof
)
if "%CMD%"=="update" (
    docker compose build agentark
    docker compose up -d agentark
    echo Update complete!
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
echo   update     Rebuild and restart
