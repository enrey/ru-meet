$ErrorActionPreference = 'Stop'

# llama.cpp requires the SPIRV-Headers CMake package, not just glslc.
$sdkRoot = $env:VULKAN_SDK
if (-not $sdkRoot -or -not (Test-Path -LiteralPath $sdkRoot -PathType Container)) {
    throw 'VULKAN_SDK must point to an installed SDK.'
}
foreach ($relativePath in @('Bin/glslc.exe', 'Lib/vulkan-1.lib', 'Include/vulkan/vulkan.h', 'Include/vulkan/vulkan.hpp', 'Lib/cmake/SPIRV-Headers/SPIRV-HeadersConfig.cmake', 'Include/spirv/unified1/spirv.hpp')) {
    if (-not (Test-Path -LiteralPath (Join-Path $sdkRoot $relativePath) -PathType Leaf)) {
        throw "Vulkan SDK is missing $relativePath. Install the pinned SDK with its development files."
    }
}
$prefixPath = $sdkRoot.Replace('\', '/')
if ($env:CMAKE_PREFIX_PATH) { $prefixPath += ";$env:CMAKE_PREFIX_PATH" }
if ($env:GITHUB_ENV) {
    "CMAKE_PREFIX_PATH=$prefixPath" | Out-File -LiteralPath $env:GITHUB_ENV -Encoding utf8 -Append
}
$env:CMAKE_PREFIX_PATH = $prefixPath
Write-Host "Vulkan SDK and SPIRV-Headers verified: $sdkRoot"
