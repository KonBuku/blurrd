#Requires -Version 7.2
<#
.SYNOPSIS
Builds the minimal LGPL FFmpeg that blurrd links and ships, checks its
configuration and packages the binaries and the corresponding source.

.DESCRIPTION
Runs vcpkg in manifest mode on this folder (vcpkg.json pins the port and its
features, vcpkg-configuration.json the vcpkg baseline, triplets/ the
release-only shared triplet). The vcpkg checkout must be at exactly that
baseline commit. The script then

  1. checks the build configuration: LGPL-2.1-or-later only (no gpl,
     nonfree, version3 or openh264) and every required component present
     (Media Foundation, NVENC/NVDEC, AMF, oneVPL/QSV, dav1d, D3D11VA), checked
     on the configure line and on ffprobe's encoder and decoder lists;
  2. packages bin/ (DLLs and ffprobe.exe), include/, lib/ (import libraries),
     licenses/ (every installed package's copyright file) and BUILDINFO.txt
     into ffmpeg-<version>-r<revision>-win64-lgpl-shared.zip;
  3. packages the exact FFmpeg source tarball, the vcpkg port files with
     their patches and this build folder into
     ffmpeg-<version>-r<revision>-source.zip (LGPL source offer);
  4. writes SHA256SUMS.txt for both archives.

The same script runs in .github/workflows/ffmpeg.yml and locally, see
docs/development.md ("FFmpeg").

.PARAMETER VcpkgRoot
The vcpkg checkout (bootstrapped) at the baseline commit.

.PARAMETER OutDir
Output folder. vcpkg's work folders go to OutDir/work, the archives to
OutDir/dist. Keep the path short and without spaces (FFmpeg's configure
fails on paths with spaces).

.PARAMETER Revision
Build revision, the N in the release tag deps-ffmpeg-<version>-rN. Use 0 for
a build that is not published.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $VcpkgRoot,
    [Parameter(Mandatory)] [string] $OutDir,
    [Parameter(Mandatory)] [ValidateRange(0, 999)] [int] $Revision
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$manifestDir = $PSScriptRoot
# Target triplet of the shipped libraries. Build tools (pkgconf, meson, the
# bin2c helper) use vcpkg's default host triplet so they never end up next to
# the shipped DLLs.
$triplet = 'x64-windows-blurrd'
$manifest = Get-Content -Raw (Join-Path $manifestDir 'vcpkg.json') | ConvertFrom-Json
$configuration = Get-Content -Raw (Join-Path $manifestDir 'vcpkg-configuration.json') | ConvertFrom-Json
$version = $manifest.'version-string'
$baseline = $configuration.'default-registry'.baseline
$name = "ffmpeg-$version-r$Revision"

$VcpkgRoot = (Resolve-Path $VcpkgRoot).Path
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
if ($OutDir -match ' ') { throw "OutDir must not contain spaces: $OutDir" }

$head = (git -C $VcpkgRoot rev-parse HEAD).Trim()
if ($head -ne $baseline) {
    throw "vcpkg checkout is at $head, but vcpkg-configuration.json pins $baseline"
}

# 1. Build ------------------------------------------------------------------
$work = Join-Path $OutDir 'work'
$installRoot = Join-Path $work 'installed'
& (Join-Path $VcpkgRoot 'vcpkg.exe') install `
    "--x-manifest-root=$manifestDir" `
    "--x-install-root=$installRoot" `
    "--x-buildtrees-root=$(Join-Path $work 'buildtrees')" `
    "--x-packages-root=$(Join-Path $work 'packages')" `
    "--triplet=$triplet" `
    '--host-triplet=x64-windows' `
    '--disable-metrics'
$installed = Join-Path $installRoot $triplet

# 2. Check the configuration ------------------------------------------------
$ffprobe = Join-Path $installed 'tools/ffmpeg/ffprobe.exe'
$buildconf = (& $ffprobe -hide_banner -buildconf) -join "`n"
$license = (& $ffprobe -hide_banner -L) -join "`n"
$encoders = (& $ffprobe -hide_banner -encoders) -join "`n"
$decoders = (& $ffprobe -hide_banner -decoders) -join "`n"

$failures = [System.Collections.Generic.List[string]]::new()
foreach ($flag in '--enable-gpl', '--enable-nonfree', '--enable-version3', '--enable-libopenh264') {
    if ($buildconf.Contains($flag)) { $failures.Add("forbidden configure flag $flag") }
}
foreach ($flag in '--enable-mediafoundation', '--enable-ffnvcodec', '--enable-nvenc', '--enable-nvdec',
    '--enable-cuda', '--enable-amf', '--enable-libvpl', '--enable-libdav1d', '--enable-d3d11va', '--enable-shared') {
    if (-not $buildconf.Contains($flag)) { $failures.Add("missing configure flag $flag") }
}
if ($license -notmatch 'GNU Lesser General Public\s+License as published by the Free Software Foundation; either\s+version 2\.1') {
    $failures.Add('ffprobe -L does not report LGPL version 2.1 or later')
}
foreach ($encoder in 'h264_nvenc', 'hevc_nvenc', 'av1_nvenc', 'h264_amf', 'hevc_amf', 'h264_qsv',
    'hevc_qsv', 'h264_mf', 'hevc_mf') {
    if ($encoders -notmatch "\s$encoder\s") { $failures.Add("missing encoder $encoder") }
}
foreach ($decoder in 'libdav1d', 'h264_cuvid', 'hevc_cuvid', 'av1_cuvid') {
    if ($decoders -notmatch "\s$decoder\s") { $failures.Add("missing decoder $decoder") }
}
if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Error -ErrorAction Continue $_ }
    throw "FFmpeg build configuration check failed ($($failures.Count) problems)"
}
Write-Host 'Configuration check passed: LGPL-2.1-or-later, all required components present.'

# 3. Binary package ---------------------------------------------------------
$dist = Join-Path $OutDir 'dist'
$stage = Join-Path $work "stage/$name-win64-lgpl-shared"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $work 'stage'), $dist
New-Item -ItemType Directory -Force $stage, $dist | Out-Null

$ports = (& (Join-Path $VcpkgRoot 'vcpkg.exe') list "--x-install-root=$installRoot" '--x-full-desc') -join "`n"
$buildInfo = @"
FFmpeg $version, blurrd build revision $Revision
License: LGPL-2.1-or-later (shared libraries, user-replaceable)
Built with vcpkg baseline $baseline, triplet $triplet
Source: $name-source.zip (FFmpeg source tarball, vcpkg port files and patches)

Configuration (ffprobe -buildconf):
$buildconf

Installed vcpkg packages:
$ports
"@
Set-Content -Encoding utf8NoBOM (Join-Path $stage 'BUILDINFO.txt') $buildInfo

New-Item -ItemType Directory -Force (Join-Path $stage 'bin'), (Join-Path $stage 'lib') | Out-Null
Copy-Item (Join-Path $installed 'bin/*.dll') (Join-Path $stage 'bin')
Copy-Item $ffprobe (Join-Path $stage 'bin')
Copy-Item -Recurse (Join-Path $installed 'include') (Join-Path $stage 'include')
foreach ($lib in 'avutil', 'avcodec', 'avformat', 'swscale', 'swresample') {
    Copy-Item (Join-Path $installed "lib/$lib.lib") (Join-Path $stage 'lib')
}
foreach ($copyright in Get-ChildItem (Join-Path $installed 'share') -Recurse -Filter 'copyright') {
    $target = Join-Path $stage "licenses/$($copyright.Directory.Name)"
    New-Item -ItemType Directory -Force $target | Out-Null
    Copy-Item $copyright.FullName $target
}
$binaryZip = Join-Path $dist "$name-win64-lgpl-shared.zip"
[System.IO.Compression.ZipFile]::CreateFromDirectory($stage, $binaryZip,
    [System.IO.Compression.CompressionLevel]::Optimal, $true)

# 4. Source package ---------------------------------------------------------
$sourceStage = Join-Path $work "stage/$name-source"
New-Item -ItemType Directory -Force $sourceStage | Out-Null
$tarball = Get-ChildItem (Join-Path $VcpkgRoot 'downloads') -Filter "ffmpeg-ffmpeg-n$version*.tar.gz" |
    Select-Object -First 1
if ($null -eq $tarball) { throw "FFmpeg source tarball for $version not found in vcpkg downloads" }
Copy-Item $tarball.FullName $sourceStage
New-Item -ItemType Directory -Force (Join-Path $sourceStage 'vcpkg-ports') | Out-Null
Copy-Item -Recurse (Join-Path $VcpkgRoot 'ports/ffmpeg') (Join-Path $sourceStage 'vcpkg-ports/ffmpeg')
Copy-Item -Recurse $manifestDir (Join-Path $sourceStage 'ffmpeg-build')
Copy-Item (Join-Path $stage 'BUILDINFO.txt') $sourceStage
$sourceReadme = @"
Corresponding source of the FFmpeg $version libraries shipped with blurrd
(build revision $Revision), offered under the GNU Lesser General Public
License version 2.1 or later.

  $($tarball.Name)  the unmodified FFmpeg release tarball (tag n$version)
  vcpkg-ports/ffmpeg/  the vcpkg port at baseline $baseline; its patches are
                       applied to the tarball before building
  ffmpeg-build/        blurrd's vcpkg manifest, configuration, triplet and
                       build script
  BUILDINFO.txt        the exact configure line and package versions

To rebuild: check out vcpkg at the baseline above, bootstrap it and run
ffmpeg-build/build.ps1 -VcpkgRoot <vcpkg> -OutDir <dir> -Revision $Revision.
"@
Set-Content -Encoding utf8NoBOM (Join-Path $sourceStage 'README.txt') $sourceReadme
$sourceZip = Join-Path $dist "$name-source.zip"
[System.IO.Compression.ZipFile]::CreateFromDirectory($sourceStage, $sourceZip,
    [System.IO.Compression.CompressionLevel]::Optimal, $true)

# 5. Checksums --------------------------------------------------------------
$sums = foreach ($file in $binaryZip, $sourceZip) {
    $hash = (Get-FileHash -Algorithm SHA256 $file).Hash.ToLowerInvariant()
    "$hash  $(Split-Path -Leaf $file)"
}
Set-Content -Encoding utf8NoBOM (Join-Path $dist 'SHA256SUMS.txt') $sums
$sums | Write-Host
