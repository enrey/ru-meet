param([string]$SdkRoot)
$ErrorActionPreference = 'Stop'
$version = '1.4.357.0'
if (-not $SdkRoot) {
    $SdkRoot = Join-Path ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))) 'VULKAN_SDK'
}
$SdkRoot = [IO.Path]::GetFullPath($SdkRoot)
$temporaryRoot = $env:RUNNER_TEMP
if (-not $temporaryRoot) { $temporaryRoot = [IO.Path]::GetTempPath() }
$installerDirectory = Join-Path $temporaryRoot 'vulkan-installer'
New-Item -ItemType Directory -Path $installerDirectory -Force | Out-Null
$installerPath = Join-Path $installerDirectory "VulkanSDK-$version.exe"
if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
    Invoke-WebRequest -Uri "https://sdk.lunarg.com/sdk/download/$version/windows/vulkan_sdk.exe?Human=true" -OutFile $installerPath
}
$signature = Get-AuthenticodeSignature -LiteralPath $installerPath
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'LunarG') {
    throw 'Vulkan SDK installer does not have a valid LunarG signature.'
}
# Run the real installer: archive extraction omits headers and import libraries.
# copy_only avoids registry, system PATH, driver/layer and shortcut changes.
$installation = Start-Process -FilePath $installerPath -ArgumentList @('--root', "`"$SdkRoot`"", '--accept-licenses', '--default-answer', '--confirm-command', 'install', 'copy_only=1') -WindowStyle Hidden -Wait -PassThru
if ($installation.ExitCode -ne 0) { throw "Vulkan SDK installation failed: $($installation.ExitCode)" }
foreach ($relativePath in @('Bin/glslc.exe', 'Bin/glslangValidator.exe', 'Lib/vulkan-1.lib', 'Include/vulkan/vulkan.h', 'Include/vulkan/vulkan.hpp')) {
    if (-not (Test-Path -LiteralPath (Join-Path $SdkRoot $relativePath) -PathType Leaf)) {
        throw "Official Vulkan SDK installation is missing $relativePath"
    }
}
$env:VULKAN_SDK = $SdkRoot.Replace('\', '/')
$env:VK_SDK_PATH = $env:VULKAN_SDK
if ($env:GITHUB_ENV) {
    @("VULKAN_SDK=$env:VULKAN_SDK", "VK_SDK_PATH=$env:VK_SDK_PATH", "VULKAN_SDK_VERSION=$version") | Out-File -LiteralPath $env:GITHUB_ENV -Encoding utf8 -Append
}
if ($env:GITHUB_PATH) { (Join-Path $SdkRoot 'Bin') | Out-File -LiteralPath $env:GITHUB_PATH -Encoding utf8 -Append }
$env:PATH = (Join-Path $SdkRoot 'Bin') + [IO.Path]::PathSeparator + $env:PATH
Write-Host "Official Vulkan SDK $version installed and development files verified: $SdkRoot"
