<#
.SYNOPSIS
    Build the release archives, the same ones CI publishes on a tag.

.DESCRIPTION
    Produces two archives in dist/, because two different people receive them:

      valhsync-<version>-<target>          the admin's: both programs and every guide
      valhsync-launcher-<version>-<target> the player's: the launcher and nothing else

    The player archive exists so an admin can forward one file without also
    handing out the publisher, the deployment notes and the threat model.

    Nothing from this machine goes in: the archives are built from a scratch
    folder holding only what is copied here, so no configuration, no signing
    key and no server ever travels with them.

.PARAMETER Target
    Rust target triple. Defaults to the host's.

.PARAMETER SkipBuild
    Package what is already in target/<triple>/release rather than rebuilding.
#>
[CmdletBinding()]
param(
    [string]$Target = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

if (-not $Target) {
    $hostLine = (& rustc -vV) | Where-Object { $_ -like "host:*" }
    $Target = ($hostLine -split ":\s*")[1].Trim()
}
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' |
            Select-Object -First 1).Matches[0].Groups[1].Value
$exe = if ($Target -like "*windows*") { ".exe" } else { "" }

Write-Host "valhsync $version -> $Target"

if (-not $SkipBuild) {
    # Rust bakes the path of every source file into its panic messages, which
    # for dependencies means the build machine's Cargo home -- and so the name
    # of whoever built the archive. Remap both roots to neutral names: the
    # binaries stop naming a person, and two machines build the same bytes.
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME ".cargo" }
    $flags = "--remap-path-prefix=$cargoHome=/cargo --remap-path-prefix=$(Get-Location)=/valhsync"
    $previous = $env:RUSTFLAGS
    $env:RUSTFLAGS = if ($previous) { "$previous $flags" } else { $flags }
    try {
        # --locked so an archive can never be built from a lockfile this
        # machine happened to have lying around.
        & cargo build --workspace --release --locked --target $Target
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } finally {
        $env:RUSTFLAGS = $previous
    }
}

# cargo --target writes under the triple; a plain host build does not. With
# -SkipBuild either may be what is on disk.
$bin = "target/$Target/release"
if (-not (Test-Path "$bin/valhsync$exe")) { $bin = "target/release" }
foreach ($name in @("valhsync$exe", "valhsync-server$exe")) {
    if (-not (Test-Path "$bin/$name")) { throw "missing $bin/$name" }
}

$dist = "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null

# Compress-Archive writes Windows separators into the entry names, which the
# zip format does not allow: unzip warns, and anything not on Windows unpacks
# one file with backslashes in its name. Write the entries ourselves.
function New-Zip {
    param([string]$Source, [string]$Destination, [string]$Prefix)

    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $root = (Resolve-Path $Source).Path
    $archive = [System.IO.Compression.ZipFile]::Open(
        [System.IO.Path]::GetFullPath($Destination),
        [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in Get-ChildItem -Recurse -File $root) {
            $relative = $file.FullName.Substring($root.Length).TrimStart('\', '/')
            $entry = "$Prefix/" + ($relative -replace '\\', '/')
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive, $file.FullName, $entry,
                [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
    } finally {
        $archive.Dispose()
    }
}

function New-Archive {
    param([string]$Name, [scriptblock]$Fill)

    $stage = Join-Path $dist $Name
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    & $Fill $stage

    $zip = Join-Path $dist "$Name.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    New-Zip -Source $stage -Destination $zip -Prefix $Name
    Remove-Item -Recurse -Force $stage

    $sum = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    "$sum  $Name.zip" | Out-File -FilePath "$zip.sha256" -Encoding ascii
    $size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host ("  {0,-52} {1,5} MiB" -f "$Name.zip", $size)
}

# The licences travel with every archive: the windows embed two typefaces.
$licences = @{
    "LICENSE-MIT"             = "LICENSE-MIT"
    "LICENSE-APACHE"          = "LICENSE-APACHE"
    "LICENSE-Cinzel.txt"      = "crates/valhsync-ui/assets/OFL-Cinzel.txt"
    "LICENSE-SourceSerif.txt" = "crates/valhsync-ui/assets/OFL-SourceSerif.txt"
}

New-Archive "valhsync-$version-$Target" {
    param($stage)
    Copy-Item "$bin/valhsync$exe", "$bin/valhsync-server$exe" $stage
    Copy-Item README.md, SECURITY.md $stage
    foreach ($as in $licences.Keys) { Copy-Item $licences[$as] (Join-Path $stage $as) }
    $docs = Join-Path $stage "docs"
    New-Item -ItemType Directory -Force -Path $docs | Out-Null
    Copy-Item docs/admin-guide.md, docs/player-guide.md $docs
    Copy-Item -Recurse docs/deploy $docs
}

New-Archive "valhsync-launcher-$version-$Target" {
    param($stage)
    Copy-Item "$bin/valhsync$exe" $stage
    Copy-Item docs/player-guide.md (Join-Path $stage "README.md")
    foreach ($as in $licences.Keys) { Copy-Item $licences[$as] (Join-Path $stage $as) }
}

Write-Host "`nArchives in $dist. Check a SHA256 with:"
Write-Host "  Get-FileHash dist\valhsync-$version-$Target.zip -Algorithm SHA256"
