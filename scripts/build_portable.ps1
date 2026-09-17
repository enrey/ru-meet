$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repo
$version = (Get-Content 'frontend\src-tauri\tauri.conf.json' -Raw | ConvertFrom-Json).version
$destination = Join-Path $repo "portable\meetily-$version-cpu"
$portableExe = Join-Path $destination 'meetily.exe'
$running = Get-Process meetily -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $portableExe }
if ($running) { throw "Close the running portable Meetily before rebuilding: $portableExe" }

# MSBuild treats environment variable names case-insensitively. Some shells
# provide both Path/PATH and lower/upper proxy names, which makes CL.exe fail.
$savedPath = $env:PATH
$savedHttpProxy = $env:HTTP_PROXY
$savedHttpsProxy = $env:HTTPS_PROXY
Remove-Item Env:Path, Env:http_proxy, Env:https_proxy -ErrorAction SilentlyContinue
$env:PATH = $savedPath
if ($savedHttpProxy) { $env:HTTP_PROXY = $savedHttpProxy }
if ($savedHttpsProxy) { $env:HTTPS_PROXY = $savedHttpsProxy }

$cmake = Get-Command cmake -ErrorAction SilentlyContinue
if (-not $cmake -and (Test-Path 'C:\Program Files\CMake\bin\cmake.exe')) {
    $env:PATH = "C:\Program Files\CMake\bin;$env:PATH"
    $cmake = Get-Command cmake -ErrorAction SilentlyContinue
}
if (-not $cmake) { throw 'CMake is required. Install it before building.' }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Rust cargo is required.' }
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) { throw 'pnpm is required.' }

if (-not $env:LIBCLANG_PATH -and (Test-Path 'C:\Program Files\LLVM\bin\libclang.dll')) {
    $env:LIBCLANG_PATH = 'C:\Program Files\LLVM\bin'
}
$env:CMAKE_PROJECT_INCLUDE = (Resolve-Path '.github\force-portable-ggml.cmake').Path.Replace('\', '/')
$env:RUSTFLAGS = '-C target-cpu=x86-64-v2'
# Cargo defaults to one job per logical core. Forcing 1 here serializes the
# entire release build (whisper.cpp/onnxruntime included) onto a single
# core, which is the main reason this build takes ~30 minutes. Cap at 8
# instead of leaving it fully unbounded, to leave headroom on this machine
# for the parallel cl.exe/link.exe processes MSVC spawns per crate.
$env:CARGO_BUILD_JOBS = '8'

$buildTimeline = [System.Collections.Generic.List[object]]::new()
$buildStopwatch = [System.Diagnostics.Stopwatch]::StartNew()

function Invoke-Checked([string]$description, [scriptblock]$command) {
    Write-Host "`n== $description =="
    $stepStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    & $command
    $stepStopwatch.Stop()
    $buildTimeline.Add([PSCustomObject]@{ Step = $description; Seconds = [math]::Round($stepStopwatch.Elapsed.TotalSeconds, 1) })
    Write-Host ("-- {0}: {1:N1}s --" -f $description, $stepStopwatch.Elapsed.TotalSeconds)
    if ($LASTEXITCODE -ne 0) { throw "$description failed (exit code $LASTEXITCODE)" }
}

Invoke-Checked 'Install frontend dependencies' { pnpm --dir frontend install --frozen-lockfile }
Invoke-Checked 'Build release llama-helper (CPU)' { cargo build --release -p llama-helper }

$binaries = Join-Path $repo 'frontend\src-tauri\binaries'
New-Item -ItemType Directory -Path $binaries -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'target\release\llama-helper.exe') -Destination (Join-Path $binaries 'llama-helper-x86_64-pc-windows-msvc.exe') -Force

Push-Location (Join-Path $repo 'frontend')
try {
    Invoke-Checked 'Build production frontend' { pnpm build }

    # The frontend is already built. Skip Tauri's second pnpm build.
    $configPath = Join-Path $env:TEMP 'meetily-portable-tauri-config.json'
    [System.IO.File]::WriteAllText($configPath, '{"build":{"beforeBuildCommand":""}}', (New-Object System.Text.UTF8Encoding($false)))
    Invoke-Checked 'Build release Meetily without an installer' {
        pnpm run tauri -- build --no-bundle --config $configPath
    }
} finally {
    Pop-Location
}

Invoke-Checked 'Verify portable Whisper CPU build' { node .github\verify-portable-ggml.cjs target\release }

$source = Join-Path $repo 'target\release'
New-Item -ItemType Directory -Path $destination -Force | Out-Null

foreach ($name in @('meetily.exe', 'ffmpeg.exe', 'onnxruntime.dll', 'onnxruntime_providers_shared.dll', 'onnxruntime-LICENSE.txt')) {
    $sourceFile = Join-Path $source $name
    if (-not (Test-Path -LiteralPath $sourceFile)) { throw "Missing build output: $sourceFile" }
    Copy-Item -LiteralPath $sourceFile -Destination (Join-Path $destination $name) -Force
}
Copy-Item -LiteralPath (Join-Path $binaries 'llama-helper-x86_64-pc-windows-msvc.exe') -Destination (Join-Path $destination 'llama-helper-x86_64-pc-windows-msvc.exe') -Force

$templates = Join-Path $destination 'templates'
New-Item -ItemType Directory -Path $templates -Force | Out-Null
Copy-Item -Path (Join-Path $repo 'frontend\src-tauri\templates\*.json') -Destination $templates -Force

$portableConfig = Join-Path $destination 'portable.json'
if (-not (Test-Path -LiteralPath $portableConfig)) {
    [System.IO.File]::WriteAllText($portableConfig, '{"dataDir":"data"}', (New-Object System.Text.UTF8Encoding($false)))
}
$config = Get-Content -LiteralPath $portableConfig -Raw | ConvertFrom-Json
if (-not $config.dataDir) { throw "$portableConfig must contain dataDir" }

$buildStopwatch.Stop()
Write-Host "`n== Timeline =="
$buildTimeline | ForEach-Object { Write-Host ("{0,7:N1}s  {1}" -f $_.Seconds, $_.Step) }
Write-Host ("{0,7:N1}s  TOTAL" -f $buildStopwatch.Elapsed.TotalSeconds)

Write-Host "`nPortable build ready: $destination"
Write-Host "Run: $destination\meetily.exe"
Write-Host 'Existing portable data was preserved.'
