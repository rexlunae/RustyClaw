# RustyClaw Install Script for Windows
# Installs prerequisites and RustyClaw via PowerShell
#
# Usage (PowerShell as Admin):
#   irm https://rexlunae.github.io/RustyClaw/install.ps1 | iex
#
# Or save and run:
#   .\install.ps1 [-Features "matrix,browser"] [-Full]

param(
    [string]$Features = "default",
    [switch]$Full,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

function Write-Info { param($msg) Write-Host "[INFO] " -ForegroundColor Blue -NoNewline; Write-Host $msg }
function Write-OK { param($msg) Write-Host "[OK] " -ForegroundColor Green -NoNewline; Write-Host $msg }
function Write-Warn { param($msg) Write-Host "[WARN] " -ForegroundColor Yellow -NoNewline; Write-Host $msg }
function Write-Err { param($msg) Write-Host "[ERROR] " -ForegroundColor Red -NoNewline; Write-Host $msg; exit 1 }

Write-Host ""
Write-Host "🦀🦞 RustyClaw Installer for Windows" -ForegroundColor Cyan
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host ""

if ($Help) {
    Write-Host "Usage: .\install.ps1 [options]"
    Write-Host ""
    Write-Host "Options:"
    Write-Host "  -Features <list>  Comma-separated features (default: default)"
    Write-Host "  -Full             Install with all features"
    Write-Host "  -Help             Show this help"
    Write-Host ""
    Write-Host "Examples:"
    Write-Host "  .\install.ps1                     # Basic install"
    Write-Host "  .\install.ps1 -Features matrix    # With Matrix support"
    Write-Host "  .\install.ps1 -Full               # All features"
    exit 0
}

if ($Full) {
    $Features = "full"
}

# Check for admin (needed for some installers)
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Warn "Not running as Administrator. Some installations may require elevation."
}

# Check for Rust
Write-Info "Checking for Rust..."
$rustInstalled = $false
try {
    $rustVersion = (rustc --version 2>$null)
    if ($rustVersion) {
        Write-OK "Rust found: $rustVersion"
        $rustInstalled = $true
    }
} catch {}

if (-not $rustInstalled) {
    Write-Warn "Rust not found. Installing via rustup..."
    
    # Download and run rustup-init
    $rustupUrl = "https://win.rustup.rs/x86_64"
    $rustupPath = "$env:TEMP\rustup-init.exe"
    
    Write-Info "Downloading rustup..."
    Invoke-WebRequest -Uri $rustupUrl -OutFile $rustupPath -UseBasicParsing
    
    Write-Info "Running rustup installer..."
    Start-Process -FilePath $rustupPath -ArgumentList "-y" -Wait -NoNewWindow
    
    # Update PATH for current session
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    
    Write-OK "Rust installed"
}

# Check Rust version
$rustcOutput = rustc --version
if ($rustcOutput -match "rustc (\d+)\.(\d+)") {
    $major = [int]$Matches[1]
    $minor = [int]$Matches[2]
    if ($major -lt 1 -or ($major -eq 1 -and $minor -lt 85)) {
        Write-Warn "Rust 1.85+ required. Updating..."
        rustup update stable
        Write-OK "Rust updated"
    }
}

# Check for Visual Studio Build Tools
Write-Info "Checking for Visual Studio Build Tools..."
$vsInstalled = $false

# Check common VS paths
$vsPaths = @(
    "${env:ProgramFiles}\Microsoft Visual Studio\2022\*\VC\Tools\MSVC",
    "${env:ProgramFiles}\Microsoft Visual Studio\2019\*\VC\Tools\MSVC",
    "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\*\VC\Tools\MSVC",
    "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2019\*\VC\Tools\MSVC"
)

foreach ($path in $vsPaths) {
    if (Test-Path $path) {
        $vsInstalled = $true
        break
    }
}

# Also check via vswhere
try {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vsPath) {
            $vsInstalled = $true
        }
    }
} catch {}

if ($vsInstalled) {
    Write-OK "Visual Studio Build Tools found"
} else {
    Write-Warn "Visual Studio Build Tools not found."
    Write-Host ""
    Write-Host "RustyClaw requires the MSVC C++ build tools." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Option 1: Install Visual Studio Build Tools (recommended)" -ForegroundColor White
    Write-Host "  Download from: https://visualstudio.microsoft.com/visual-cpp-build-tools/" -ForegroundColor Gray
    Write-Host "  Select: 'Desktop development with C++'" -ForegroundColor Gray
    Write-Host ""
    Write-Host "Option 2: Install full Visual Studio Community (free)" -ForegroundColor White
    Write-Host "  Download from: https://visualstudio.microsoft.com/vs/community/" -ForegroundColor Gray
    Write-Host ""
    
    $response = Read-Host "Would you like to open the download page? (y/N)"
    if ($response -eq "y" -or $response -eq "Y") {
        Start-Process "https://visualstudio.microsoft.com/visual-cpp-build-tools/"
        Write-Host ""
        Write-Err "Please install Build Tools, then re-run this script."
    } else {
        Write-Err "Build Tools required. Please install and re-run."
    }
}

# Install RustyClaw
Write-Host ""
Write-Info "Installing RustyClaw with features: $Features"

if ($Features -eq "default") {
    cargo install rustyclaw
} else {
    cargo install rustyclaw --features $Features
}

Write-OK "RustyClaw installed!"

# Verify
Write-Host ""
try {
    $version = rustyclaw --version
    Write-OK "Installation complete: $version"
} catch {
    Write-OK "Installation complete!"
}

Write-Host ""
Write-Host "Next steps:" -ForegroundColor Cyan
Write-Host "  1. Open a new terminal (to refresh PATH)"
Write-Host "  2. Run: rustyclaw onboard"
Write-Host "  3. Then: rustyclaw tui"
Write-Host ""
Write-Host "Documentation: https://github.com/rexlunae/RustyClaw#readme" -ForegroundColor Gray
