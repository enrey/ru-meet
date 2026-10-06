$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$relativeTarget = 'target/agent-validation/ci-script-tests-' + [guid]::NewGuid().ToString('N')
$testRoot = Join-Path $repositoryRoot $relativeTarget
$broken = Join-Path $testRoot 'release/build/llama-cpp-sys-2-broken/out/build'
$valid = Join-Path $testRoot 'release/build/llama-cpp-sys-2-valid/out/build'
$unrelated = Join-Path $testRoot 'release/build/another-crate/out/build'
$originalSdk = $env:VULKAN_SDK
$originalPrefix = $env:CMAKE_PREFIX_PATH
$originalGithubEnv = $env:GITHUB_ENV
try {
    foreach ($directory in @($broken, $valid, $unrelated)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
        New-Item -ItemType File -Path (Join-Path $directory 'CMakeCache.txt') | Out-Null
    }
    New-Item -ItemType File -Path (Join-Path $valid 'INSTALL.vcxproj') | Out-Null
    & (Join-Path $repositoryRoot '.github/reset-incomplete-llama-cmake.ps1') -TargetDirectory $relativeTarget
    if (Test-Path -LiteralPath (Join-Path $broken 'CMakeCache.txt')) { throw 'Incomplete cache was not reset.' }
    if (-not (Test-Path -LiteralPath (Join-Path $valid 'CMakeCache.txt'))) { throw 'Valid cache was removed.' }
    if (-not (Test-Path -LiteralPath (Join-Path $unrelated 'CMakeCache.txt'))) { throw 'Unrelated cache was removed.' }
    $rejected = $false
    try { & (Join-Path $repositoryRoot '.github/reset-incomplete-llama-cmake.ps1') -TargetDirectory '..' }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'Target outside repository was accepted.' }
    $env:GITHUB_ENV = $null
    $env:VULKAN_SDK = $testRoot
    $rejected = $false
    try { & (Join-Path $repositoryRoot '.github/prepare-vulkan.ps1') }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'Incomplete Vulkan SDK was accepted.' }
    # Reproduce the CI SDK layout: binaries exist, development package does not.
    New-Item -ItemType Directory -Path (Join-Path $testRoot 'Bin') | Out-Null
    New-Item -ItemType File -Path (Join-Path $testRoot 'Bin/glslc.exe') | Out-Null
    & (Join-Path $repositoryRoot '.github/install-spirv-headers.ps1') -SdkRoot $testRoot
    # Installing SPIRV-Headers must not make a binaries-only SDK pass preflight.
    $rejected = $false
    try { & (Join-Path $repositoryRoot '.github/prepare-vulkan.ps1') }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'SDK without Vulkan headers/library was accepted.' }
    cmake -S $PSScriptRoot -B (Join-Path $testRoot 'installed-package-check') "-DCMAKE_PREFIX_PATH=$testRoot" "-DEXPECTED_SPIRV_PREFIX=$testRoot"
    if ($LASTEXITCODE -ne 0) { throw 'CMake could not import explicitly installed SPIRV-Headers.' }
    # A second invocation must not download or reinstall the package.
    & (Join-Path $repositoryRoot '.github/install-spirv-headers.ps1') -SdkRoot $testRoot
    if ($originalSdk) {
        $env:VULKAN_SDK = $originalSdk
        & (Join-Path $repositoryRoot '.github/prepare-vulkan.ps1')
        cmake -S $PSScriptRoot -B (Join-Path $testRoot 'package-check') "-DCMAKE_PREFIX_PATH=$originalSdk" "-DEXPECTED_VULKAN_PREFIX=$originalSdk" -DREQUIRE_VULKAN=ON
        if ($LASTEXITCODE -ne 0) { throw 'CMake could not import SPIRV-Headers.' }
        cmake --build (Join-Path $testRoot 'package-check') --config Release
        if ($LASTEXITCODE -ne 0) { throw 'Vulkan development headers/library failed the compile/link check.' }
    }
    Write-Host 'Native CI script tests passed.'
} finally {
    $env:VULKAN_SDK = $originalSdk
    $env:CMAKE_PREFIX_PATH = $originalPrefix
    $env:GITHUB_ENV = $originalGithubEnv
    # Delete only the exact, newly created fixture tree inside agent-validation.
    $expectedParent = [IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target/agent-validation'))
    if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($testRoot)) -ne $expectedParent) {
        throw 'Unsafe test cleanup path.'
    }
    if (Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force }
}
