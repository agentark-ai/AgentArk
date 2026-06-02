param(
    [ValidateSet("check", "clippy", "build", "test")]
    [string]$Mode = "check"
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $RepoRoot
$IsWindowsHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Windows
)

function Resolve-CargoPath {
    if ($IsWindowsHost) {
        $preferred = @(
            "F:\rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe",
            "C:\Users\User\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe"
        )
        foreach ($path in $preferred) {
            if (Test-Path -LiteralPath $path) {
                return $path
            }
        }
    }

    $command = Get-Command cargo -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) {
        return $command.Source
    }

    Write-Error "Cargo executable not found. Install Rust or update build.ps1 with the local toolchain path."
    exit 127
}

$CargoPath = Resolve-CargoPath
$ToolchainDir = Split-Path -Parent $CargoPath
$HasFDrive = $IsWindowsHost -and (Test-Path -LiteralPath "F:\")
$LogDir = if ($HasFDrive) { "F:\build-logs" } else { Join-Path $RepoRoot ".build-logs" }
$LogPath = Join-Path $LogDir "agentark-$Mode.log"
$TargetDir = if (![string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR
} elseif ($HasFDrive) {
    "F:\target\agentark"
} else {
    Join-Path $RepoRoot "target"
}

$runningRust = Get-Process -ErrorAction SilentlyContinue |
    Where-Object {
        $_.ProcessName -in @("cargo", "rustc", "rustdoc", "clippy-driver", "link")
    }

if ($runningRust) {
    $summary = ($runningRust |
        Select-Object Id, ProcessName, CPU, Path |
        Format-Table -AutoSize |
        Out-String).Trim()
    Write-Error "Rust build process already running; aborting to preserve the single-instance build rule.`n$summary"
    exit 75
}

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null

$env:CARGO_BUILD_JOBS = "1"
$env:CARGO_TARGET_DIR = $TargetDir
$RustcPath = Join-Path $ToolchainDir "rustc.exe"
$RustdocPath = Join-Path $ToolchainDir "rustdoc.exe"
if (Test-Path -LiteralPath $RustcPath) {
    $env:RUSTC = $RustcPath
}
if (Test-Path -LiteralPath $RustdocPath) {
    $env:RUSTDOC = $RustdocPath
}
$env:PATH = "$ToolchainDir$([System.IO.Path]::PathSeparator)$env:PATH"
if ((!(Test-Path Env:\RUSTUP_HOME) -or [string]::IsNullOrWhiteSpace($env:RUSTUP_HOME)) -and $HasFDrive -and (Test-Path -LiteralPath "F:\rustup")) {
    $env:RUSTUP_HOME = "F:\rustup"
}
if ((!(Test-Path Env:\CARGO_HOME) -or [string]::IsNullOrWhiteSpace($env:CARGO_HOME)) -and $HasFDrive -and (Test-Path -LiteralPath "F:\cargo")) {
    $env:CARGO_HOME = "F:\cargo"
}

$CargoArgs = switch ($Mode) {
    "check" { @("check", "--locked") }
    "clippy" { @("clippy", "--locked", "--all-targets", "--all-features") }
    "build" { @("build", "--locked") }
    "test" { @("test", "--locked", "--all-targets") }
}

"[$(Get-Date -Format o)] $CargoPath $($CargoArgs -join ' ')" | Set-Content -Path $LogPath -Encoding UTF8
"CARGO_BUILD_JOBS=$env:CARGO_BUILD_JOBS" | Add-Content -Path $LogPath -Encoding UTF8
"CARGO_TARGET_DIR=$env:CARGO_TARGET_DIR" | Add-Content -Path $LogPath -Encoding UTF8
if (Test-Path Env:\RUSTC) {
    "RUSTC=$env:RUSTC" | Add-Content -Path $LogPath -Encoding UTF8
}

$PreviousErrorActionPreference = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    & $CargoPath @CargoArgs 2>&1 |
        ForEach-Object { "$_" } |
        Tee-Object -FilePath $LogPath -Append
    $ExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $PreviousErrorActionPreference
}

if ($ExitCode -ne 0) {
    Write-Error "build.ps1 $Mode failed with exit code $ExitCode. See $LogPath"
    exit $ExitCode
}

"build.ps1 $Mode completed successfully. Log: $LogPath"
