@echo off
setlocal enabledelayedexpansion

:: Crate Agent - Build Installer Script (Windows)
:: Creates a portable deployment package

set VERSION=0.1.2
set SCRIPT_DIR=%~dp0
set PROJECT_ROOT=%SCRIPT_DIR%..
set BUILD_DIR=%PROJECT_ROOT%\dist
set PACKAGE_NAME=nyrbot-%VERSION%

echo ╔═══════════════════════════════════════════════════════════╗
echo ║          Crate Agent - Build Installer v%VERSION%            ║
echo ╚═══════════════════════════════════════════════════════════╝
echo.

:: Clean previous builds
if exist "%BUILD_DIR%" rmdir /s /q "%BUILD_DIR%"
mkdir "%BUILD_DIR%\%PACKAGE_NAME%"

:: Build release binary
echo Building release binary...
cd /d "%PROJECT_ROOT%"
cargo build --release
if errorlevel 1 (
    echo Build failed!
    exit /b 1
)

:: Copy binary
echo Packaging files...
copy "%PROJECT_ROOT%\target\release\nyrbot.exe" "%BUILD_DIR%\%PACKAGE_NAME%\" >nul

:: Copy config templates
mkdir "%BUILD_DIR%\%PACKAGE_NAME%\config" 2>nul
xcopy /E /I /Y "%PROJECT_ROOT%\config" "%BUILD_DIR%\%PACKAGE_NAME%\config" >nul 2>&1

:: Copy skills
mkdir "%BUILD_DIR%\%PACKAGE_NAME%\skills" 2>nul
xcopy /E /I /Y "%PROJECT_ROOT%\skills" "%BUILD_DIR%\%PACKAGE_NAME%\skills" >nul 2>&1

:: Create install script
(
echo @echo off
echo setlocal
echo.
echo set INSTALL_DIR=%%USERPROFILE%%\.nyrbot
echo if not "%%~1"=="" set INSTALL_DIR=%%~1
echo.
echo echo Installing Crate Agent to %%INSTALL_DIR%%...
echo.
echo mkdir "%%INSTALL_DIR%%\bin" 2^>nul
echo mkdir "%%INSTALL_DIR%%\config" 2^>nul
echo mkdir "%%INSTALL_DIR%%\data" 2^>nul
echo mkdir "%%INSTALL_DIR%%\skills" 2^>nul
echo.
echo copy /Y nyrbot.exe "%%INSTALL_DIR%%\bin\" ^>nul
echo xcopy /E /I /Y config "%%INSTALL_DIR%%\config" ^>nul 2^>^&1
echo xcopy /E /I /Y skills "%%INSTALL_DIR%%\skills" ^>nul 2^>^&1
echo.
echo echo @echo off ^> "%%INSTALL_DIR%%\run.bat"
echo echo set CRATE_AGENT_CONFIG=%%INSTALL_DIR%%\config ^>^> "%%INSTALL_DIR%%\run.bat"
echo echo set CRATE_AGENT_DATA=%%INSTALL_DIR%%\data ^>^> "%%INSTALL_DIR%%\run.bat"
echo echo "%%INSTALL_DIR%%\bin\nyrbot.exe" %%%%* ^>^> "%%INSTALL_DIR%%\run.bat"
echo.
echo echo.
echo echo Installation complete!
echo echo.
echo echo To run: %%INSTALL_DIR%%\run.bat
) > "%BUILD_DIR%\%PACKAGE_NAME%\install.bat"

:: Create zip archive
echo Creating archive...
cd /d "%BUILD_DIR%"
powershell -Command "Compress-Archive -Path '%PACKAGE_NAME%' -DestinationPath '%PACKAGE_NAME%.zip' -Force"

echo.
echo ╔═══════════════════════════════════════════════════════════╗
echo ║                    BUILD COMPLETE!                        ║
echo ╚═══════════════════════════════════════════════════════════╝
echo.
echo Installer package created: %BUILD_DIR%\%PACKAGE_NAME%.zip
echo.
echo To install locally:
echo   1. Extract %PACKAGE_NAME%.zip
echo   2. Run install.bat
echo.
echo To deploy to VPS:
echo   1. Copy %PACKAGE_NAME%.zip to VPS
echo   2. Extract and run install.sh (Linux) or use Docker
echo.

pause
