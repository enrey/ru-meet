param([string]$TargetDirectory = 'target')
$ErrorActionPreference = 'Stop'

# A failed CMake configure leaves a cache behind. llama-cpp-sys uses
# always_configure(false), so retries would otherwise skip generation forever.
# Remove only the cache of an incomplete llama build, retaining compiled files
# and valid generated projects (important for retrying shader compilation).
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repositoryRoot $TargetDirectory))
if (-not $targetRoot.StartsWith($repositoryRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Target directory must be inside this repository.'
}
$buildRoot = Join-Path $targetRoot 'release/build'
if (-not (Test-Path -LiteralPath $buildRoot -PathType Container)) { return }
foreach ($directory in Get-ChildItem -LiteralPath $buildRoot -Directory -Filter 'llama-cpp-sys-2-*') {
    $cmakeRoot = Join-Path $directory.FullName 'out/build'
    $cachePath = Join-Path $cmakeRoot 'CMakeCache.txt'
    if (-not (Test-Path -LiteralPath $cachePath -PathType Leaf)) { continue }
    $generated = @('INSTALL.vcxproj', 'build.ninja', 'Makefile') | Where-Object {
        Test-Path -LiteralPath (Join-Path $cmakeRoot $_) -PathType Leaf
    }
    if (-not $generated) {
        Write-Warning "Resetting incomplete llama CMake configuration: $cachePath"
        Remove-Item -LiteralPath $cachePath
    }
}
