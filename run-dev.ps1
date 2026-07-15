$ErrorActionPreference = 'Stop'

$dataDir = Join-Path $env:APPDATA 'MineRadio-Tauri'
$env:MINERADIO_SIDECAR_PORT = '11451'
$env:MINERADIO_SESSION_FILE = Join-Path $dataDir 'provider-sessions.json'
$env:MINERADIO_SIDECAR_LOG_FILE = Join-Path $dataDir 'mineradio-sidecar.jsonl'

New-Item -ItemType Directory -Force -Path $dataDir | Out-Null

cargo run @args
