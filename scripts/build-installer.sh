#!/bin/bash
# Crate Agent - Build Installer Script
# Creates a portable deployment package

set -e

VERSION="0.1.2"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="$PROJECT_ROOT/dist"
PACKAGE_NAME="agentark-$VERSION"

echo "╔═══════════════════════════════════════════════════════════╗"
echo "║          Crate Agent - Build Installer v$VERSION            ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""

# Clean previous builds
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/$PACKAGE_NAME"

# Build release binary
echo "Building release binary..."
cd "$PROJECT_ROOT"
cargo build --release

# Copy binary
echo "Packaging files..."
cp "$PROJECT_ROOT/target/release/agentark" "$BUILD_DIR/$PACKAGE_NAME/" 2>/dev/null || \
cp "$PROJECT_ROOT/target/release/agentark.exe" "$BUILD_DIR/$PACKAGE_NAME/"

# Copy config templates
mkdir -p "$BUILD_DIR/$PACKAGE_NAME/config"
cp -r "$PROJECT_ROOT/config/"* "$BUILD_DIR/$PACKAGE_NAME/config/" 2>/dev/null || true

# Copy skills
mkdir -p "$BUILD_DIR/$PACKAGE_NAME/skills"
cp -r "$PROJECT_ROOT/skills/"* "$BUILD_DIR/$PACKAGE_NAME/skills/" 2>/dev/null || true

# Create install script
cat > "$BUILD_DIR/$PACKAGE_NAME/install.sh" << 'INSTALL_EOF'
#!/bin/bash
# Crate Agent Installer

set -e

INSTALL_DIR="${1:-$HOME/.agentark}"

echo "Installing Crate Agent to $INSTALL_DIR..."

mkdir -p "$INSTALL_DIR/bin"
mkdir -p "$INSTALL_DIR/config"
mkdir -p "$INSTALL_DIR/data"
mkdir -p "$INSTALL_DIR/skills"

# Copy files
cp agentark* "$INSTALL_DIR/bin/"
cp -r config/* "$INSTALL_DIR/config/" 2>/dev/null || true
cp -r skills/* "$INSTALL_DIR/skills/" 2>/dev/null || true

# Make executable
chmod +x "$INSTALL_DIR/bin/agentark"*

# Create launcher script
cat > "$INSTALL_DIR/run.sh" << 'EOF'
#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export CRATE_AGENT_CONFIG="$SCRIPT_DIR/config"
export CRATE_AGENT_DATA="$SCRIPT_DIR/data"
exec "$SCRIPT_DIR/bin/agentark" "$@"
EOF
chmod +x "$INSTALL_DIR/run.sh"

echo ""
echo "Installation complete!"
echo ""
echo "To run Crate Agent:"
echo "  $INSTALL_DIR/run.sh"
echo ""
echo "Or add to PATH:"
echo "  export PATH=\"\$PATH:$INSTALL_DIR/bin\""
INSTALL_EOF
chmod +x "$BUILD_DIR/$PACKAGE_NAME/install.sh"

# Create Windows install script
cat > "$BUILD_DIR/$PACKAGE_NAME/install.bat" << 'INSTALL_BAT'
@echo off
setlocal

set INSTALL_DIR=%USERPROFILE%\.agentark
if not "%~1"=="" set INSTALL_DIR=%~1

echo Installing Crate Agent to %INSTALL_DIR%...

mkdir "%INSTALL_DIR%\bin" 2>nul
mkdir "%INSTALL_DIR%\config" 2>nul
mkdir "%INSTALL_DIR%\data" 2>nul
mkdir "%INSTALL_DIR%\skills" 2>nul

copy /Y agentark.exe "%INSTALL_DIR%\bin\" >nul
xcopy /E /I /Y config "%INSTALL_DIR%\config" >nul 2>&1
xcopy /E /I /Y skills "%INSTALL_DIR%\skills" >nul 2>&1

echo @echo off > "%INSTALL_DIR%\run.bat"
echo set CRATE_AGENT_CONFIG=%INSTALL_DIR%\config >> "%INSTALL_DIR%\run.bat"
echo set CRATE_AGENT_DATA=%INSTALL_DIR%\data >> "%INSTALL_DIR%\run.bat"
echo "%INSTALL_DIR%\bin\agentark.exe" %%* >> "%INSTALL_DIR%\run.bat"

echo.
echo Installation complete!
echo.
echo To run Crate Agent:
echo   %INSTALL_DIR%\run.bat
echo.
INSTALL_BAT

# Create archive
echo "Creating archive..."
cd "$BUILD_DIR"
tar -czvf "$PACKAGE_NAME.tar.gz" "$PACKAGE_NAME"

# Also create zip for Windows
if command -v zip &> /dev/null; then
    zip -r "$PACKAGE_NAME.zip" "$PACKAGE_NAME"
fi

echo ""
echo "╔═══════════════════════════════════════════════════════════╗"
echo "║                    BUILD COMPLETE!                        ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""
echo "Installer packages created in: $BUILD_DIR"
echo "  - $PACKAGE_NAME.tar.gz (Linux/Mac)"
echo "  - $PACKAGE_NAME.zip (Windows)"
echo ""
echo "To deploy to VPS:"
echo "  scp $BUILD_DIR/$PACKAGE_NAME.tar.gz user@vps:/tmp/"
echo "  ssh user@vps 'cd /tmp && tar xzf $PACKAGE_NAME.tar.gz && cd $PACKAGE_NAME && ./install.sh'"
