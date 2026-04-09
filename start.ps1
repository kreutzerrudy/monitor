#Requires -Version 5.1
<#
.SYNOPSIS
    Launches the Virtual LAN Monitor stack: MediaMTX + Preset Manager.
.DESCRIPTION
    Kills stale instances, starts MediaMTX and the Python preset manager,
    waits for each to come up, then prints the stream URLs.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$BASE          = "C:\monitor"
$LOGS          = "$BASE\logs"
$MEDIAMTX      = "$BASE\mediamtx\mediamtx.exe"
$MEDIAMTX_BAT  = "$BASE\mediamtx\start.bat"
$FFMPEG        = "$BASE\ffmpeg\bin\ffmpeg.exe"
$PYTHON_SCRIPT = "$BASE\preset_manager.py"

$RTSP_PORT = 8554
$HTTP_PORT = 8889   # MediaMTX WebRTC / HLS
$API_PORT  = 9090   # Preset Manager control API

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

function Write-Step([string]$msg) {
    Write-Host "`n==> $msg" -ForegroundColor Cyan
}

function Write-Ok([string]$msg) {
    Write-Host "    [OK] $msg" -ForegroundColor Green
}

function Write-Warn([string]$msg) {
    Write-Host "    [!!] $msg" -ForegroundColor Yellow
}

function Write-Fail([string]$msg) {
    Write-Host "    [FAIL] $msg" -ForegroundColor Red
}

function Kill-ByName([string]$name) {
    $procs = @(Get-Process -Name $name -ErrorAction SilentlyContinue)
    if ($procs.Count -gt 0) {
        $procs | Stop-Process -Force
        Write-Warn "killed $($procs.Count) existing '$name' process(es)"
    }
}

function Wait-Port([int]$port, [int]$timeoutSec = 15) {
    $deadline = [datetime]::UtcNow.AddSeconds($timeoutSec)
    while ([datetime]::UtcNow -lt $deadline) {
        $conn = Test-NetConnection -ComputerName 127.0.0.1 -Port $port `
                    -InformationLevel Quiet -WarningAction SilentlyContinue
        if ($conn) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

# ---------------------------------------------------------------------------
# dependency checks
# ---------------------------------------------------------------------------

Write-Step "Checking dependencies"

$missing = @()
if (-not (Test-Path $MEDIAMTX))      { $missing += "MediaMTX ($MEDIAMTX)" }
if (-not (Test-Path $FFMPEG))        { $missing += "FFmpeg ($FFMPEG)" }
if (-not (Test-Path $PYTHON_SCRIPT)) { $missing += "Preset Manager ($PYTHON_SCRIPT)" }

$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) { $missing += "Python (not on PATH)" }

if ($missing.Count -gt 0) {
    foreach ($m in $missing) { Write-Fail "Missing: $m" }
    exit 1
}
Write-Ok "FFmpeg, MediaMTX, Python all found"

# ---------------------------------------------------------------------------
# ensure directories
# ---------------------------------------------------------------------------

Write-Step "Ensuring runtime directories"
New-Item -ItemType Directory -Force -Path $LOGS        | Out-Null
New-Item -ItemType Directory -Force -Path "$BASE\media" | Out-Null
Write-Ok "logs -> $LOGS"

# ---------------------------------------------------------------------------
# kill stale processes
# ---------------------------------------------------------------------------

Write-Step "Stopping stale processes"
Kill-ByName "mediamtx"
Kill-ByName "ffmpeg"

Get-Process -Name python,python3 -ErrorAction SilentlyContinue | ForEach-Object {
    try {
        $cmd = (Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)" `
                    -ErrorAction Stop).CommandLine
        if ($cmd -like "*preset_manager*") {
            Stop-Process -Id $_.Id -Force
            Write-Warn "killed stale preset_manager (pid $($_.Id))"
        }
    } catch { }
}
Start-Sleep -Milliseconds 500

# ---------------------------------------------------------------------------
# start MediaMTX
# ---------------------------------------------------------------------------

Write-Step "Starting MediaMTX"
$mediamtxLog = "$LOGS\mediamtx.log"
$mtxProc = Start-Process -FilePath "cmd.exe" `
    -ArgumentList "/c `"$MEDIAMTX_BAT`"" `
    -WindowStyle Hidden -PassThru
Write-Ok "MediaMTX launched (pid $($mtxProc.Id))"

Write-Host "    waiting for RTSP port $RTSP_PORT..." -NoNewline
if (Wait-Port $RTSP_PORT 20) {
    Write-Host " up" -ForegroundColor Green
} else {
    Write-Host ""
    Write-Fail "MediaMTX did not open port $RTSP_PORT within 20s -- check $mediamtxLog"
    exit 1
}

Write-Host "    waiting for HTTP port $HTTP_PORT..." -NoNewline
if (Wait-Port $HTTP_PORT 10) {
    Write-Host " up" -ForegroundColor Green
} else {
    Write-Host ""
    Write-Warn "MediaMTX HTTP port $HTTP_PORT not open (WebRTC/HLS may be disabled)"
}

# ---------------------------------------------------------------------------
# start Preset Manager
# ---------------------------------------------------------------------------

Write-Step "Starting Preset Manager"
$pmLog = "$LOGS\preset_manager.log"
$pmProc = Start-Process -FilePath "cmd.exe" `
    -ArgumentList "/c python `"$PYTHON_SCRIPT`" > `"$pmLog`" 2>&1" `
    -WindowStyle Hidden -PassThru
Write-Ok "Preset Manager launched (pid $($pmProc.Id))"

Write-Host "    waiting for API port $API_PORT..." -NoNewline
if (Wait-Port $API_PORT 20) {
    Write-Host " up" -ForegroundColor Green
} else {
    Write-Host ""
    Write-Fail "Preset Manager did not open port $API_PORT within 20s -- check $pmLog"
    exit 1
}

# ---------------------------------------------------------------------------
# confirm active preset
# ---------------------------------------------------------------------------

try {
    $status = (Invoke-WebRequest -Uri "http://localhost:$API_PORT/status" `
                   -UseBasicParsing -TimeoutSec 5).Content.Trim()
    Write-Ok "Active preset: $status"
} catch {
    Write-Warn "Could not query preset status"
}

# ---------------------------------------------------------------------------
# print URLs
# ---------------------------------------------------------------------------

Write-Host ""
Write-Host "  Stream URLs" -ForegroundColor White
Write-Host "  -------------------------------------------------"
Write-Host "  WebRTC / HLS  http://localhost:$HTTP_PORT/display/" -ForegroundColor White
Write-Host "  RTSP          rtsp://localhost:$RTSP_PORT/display"  -ForegroundColor White
Write-Host "  Control API   http://localhost:$API_PORT/presets"   -ForegroundColor White
Write-Host ""
Write-Host "  Switch preset:"                                      -ForegroundColor DarkGray
Write-Host "    curl http://localhost:$API_PORT/switch?preset=steam" -ForegroundColor DarkGray
Write-Host "    curl http://localhost:$API_PORT/switch?preset=image" -ForegroundColor DarkGray
Write-Host ""
Write-Ok "Stack is running. Press Ctrl+C to exit this window (processes continue in background)."
