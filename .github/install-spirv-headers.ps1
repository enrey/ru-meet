param([string]$SdkRoot = $env:VULKAN_SDK)
$ErrorActionPreference = 'Stop'

if (-not $SdkRoot -or -not (Test-Path -LiteralPath $SdkRoot -PathType Container)) {
    throw 'VULKAN_SDK must point to an installed SDK.'
}
$configPath = Join-Path $SdkRoot 'Lib/cmake/SPIRV-Headers/SPIRV-HeadersConfig.cmake'
$headerPath = Join-Path $SdkRoot 'Include/spirv/unified1/spirv.hpp'
if ((Test-Path -LiteralPath $configPath) -and (Test-Path -LiteralPath $headerPath)) {
    Write-Host 'SPIRV-Headers development package is already installed.'
    return
}

# install-vulkan-sdk extracts the installer with 7z, omitting development
# packages downloaded by the interactive installer. Install upstream's
# header-only CMake package explicitly, pinned to vulkan-sdk-1.4.357.0.
$revision = '29981f65241605e08b0ede4cfeb999fe3b723c6a'
$temporaryRoot = $env:RUNNER_TEMP
if (-not $temporaryRoot) { $temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) 'meetily-native-ci' }
$workRoot = Join-Path $temporaryRoot ('spirv-headers-' + [guid]::NewGuid().ToString('N'))
$sourceRoot = Join-Path $workRoot 'source'
$buildRoot = Join-Path $workRoot 'build'
try {
    New-Item -ItemType Directory -Path $sourceRoot -Force | Out-Null
    git -C $sourceRoot init --quiet
    if ($LASTEXITCODE -ne 0) { throw 'Could not initialize SPIRV-Headers checkout.' }
    git -C $sourceRoot fetch --quiet --depth 1 https://github.com/KhronosGroup/SPIRV-Headers.git $revision
    if ($LASTEXITCODE -ne 0) { throw 'Could not fetch pinned SPIRV-Headers revision.' }
    git -C $sourceRoot checkout --quiet --detach FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Could not checkout SPIRV-Headers.' }
    $actualRevision = git -C $sourceRoot rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $actualRevision -ne $revision) { throw 'Unexpected SPIRV-Headers revision.' }
    cmake -S $sourceRoot -B $buildRoot "-DCMAKE_INSTALL_PREFIX=$SdkRoot" -DCMAKE_INSTALL_DATADIR=Lib -DSPIRV_HEADERS_ENABLE_TESTS=OFF -DSPIRV_HEADERS_ENABLE_INSTALL=ON
    if ($LASTEXITCODE -ne 0) { throw 'Could not configure SPIRV-Headers installation.' }
    cmake --install $buildRoot --config Release
    if ($LASTEXITCODE -ne 0) { throw 'Could not install SPIRV-Headers development package.' }
    if (-not (Test-Path -LiteralPath $configPath) -or -not (Test-Path -LiteralPath $headerPath)) {
        throw 'SPIRV-Headers installation did not produce the required files.'
    }
    Write-Host "Installed SPIRV-Headers at revision $revision into $SdkRoot"
} finally {
    $resolvedWorkRoot = [IO.Path]::GetFullPath($workRoot)
    if ([IO.Path]::GetDirectoryName($resolvedWorkRoot) -ne [IO.Path]::GetFullPath($temporaryRoot)) {
        throw 'Unsafe SPIRV-Headers temporary cleanup path.'
    }
    if (Test-Path -LiteralPath $resolvedWorkRoot) { Remove-Item -LiteralPath $resolvedWorkRoot -Recurse -Force }
}
