<#
.SYNOPSIS
    Build the Thunderstore package: dist/ValhSync-<version>-thunderstore.zip.

.DESCRIPTION
    Thunderstore wants manifest.json, icon.png and README.md at the *root* of
    the archive -- the contents of a folder, not the folder itself, which is
    the mistake its validator complains about most often. This script writes
    the entries by hand, so there is no folder prefix and no backslash in an
    entry name.

    What goes in is the launcher alone. The publisher is an admin's tool and
    has no business in a mod manager's profile.

    The version comes from Cargo.toml, so the manifest can never claim a
    version the binary is not.

.PARAMETER SkipBuild
    Package the launcher already in target/<triple>/release rather than
    rebuilding it.
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

$target = "x86_64-pc-windows-msvc"
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' |
            Select-Object -First 1).Matches[0].Groups[1].Value
$source = "packaging/thunderstore"

Write-Host "ValhSync $version -> Thunderstore package"

if (-not $SkipBuild) {
    # Same remapping as scripts/package.ps1: a binary must not carry the paths
    # of the machine that built it.
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME ".cargo" }
    $flags = "--remap-path-prefix=$cargoHome=/cargo --remap-path-prefix=$(Get-Location)=/valhsync"
    $previous = $env:RUSTFLAGS
    $env:RUSTFLAGS = if ($previous) { "$previous $flags" } else { $flags }
    try {
        & cargo build -p valhsync --release --locked --target $target
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } finally {
        $env:RUSTFLAGS = $previous
    }
}

$exe = "target/$target/release/valhsync.exe"
if (-not (Test-Path $exe)) {
    throw "no launcher at $exe. Run without -SkipBuild."
}

# The manifest must not claim a version the workspace is not on. Checked
# rather than rewritten: the file is edited deliberately, once per release,
# and a script quietly fixing it up would hide a forgotten bump.
$manifest = Get-Content "$source/manifest.json" -Raw | ConvertFrom-Json
if ($manifest.version_number -ne $version) {
    throw "manifest.json says $($manifest.version_number), Cargo.toml says $version. Bump one."
}
if ($manifest.description.Length -gt 250) {
    throw "description is $($manifest.description.Length) characters; Thunderstore allows 250."
}

Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Image]::FromFile((Resolve-Path "$source/icon.png").Path)
try {
    if ($icon.Width -ne 256 -or $icon.Height -ne 256) {
        throw "icon.png is $($icon.Width)x$($icon.Height); Thunderstore requires exactly 256x256."
    }
} finally {
    $icon.Dispose()
}

$dist = Join-Path (Get-Location).Path "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$zipPath = Join-Path $dist "ValhSync-$version-thunderstore.zip"
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }

# name in the archive -> file on disk. Everything at the root, as required.
$entries = [ordered]@{
    "manifest.json"            = "$source/manifest.json"
    "icon.png"                 = "$source/icon.png"
    "README.md"                = "$source/README.md"
    "CHANGELOG.md"             = "CHANGELOG.md"
    "valhsync.exe"             = $exe
    "player-guide.html"        = "docs/player-guide.html"
    "launcher.png"             = "docs/launcher.png"
    "LICENSE-MIT"              = "LICENSE-MIT"
    "LICENSE-APACHE"           = "LICENSE-APACHE"
    "LICENSE-Cinzel.txt"       = "crates/valhsync-ui/assets/OFL-Cinzel.txt"
    "LICENSE-SourceSerif.txt"  = "crates/valhsync-ui/assets/OFL-SourceSerif.txt"
}

Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::Open(
    $zipPath, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($name in $entries.Keys) {
        $file = (Resolve-Path $entries[$name]).Path
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive, $file, $name,
            [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally {
    $archive.Dispose()
}

$size = [math]::Round((Get-Item $zipPath).Length / 1MB, 1)
Write-Host ("  {0,-44} {1,5} MiB" -f (Split-Path $zipPath -Leaf), $size)
Write-Host "`nUpload at https://thunderstore.io/c/valheim/create/"
Write-Host "Check the manifest first: https://thunderstore.io/tools/manifest-v1-validator/"
