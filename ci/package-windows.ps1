$ErrorActionPreference = 'Stop'
$project = Join-Path (Split-Path -Parent $PSScriptRoot) 'vndb-explorer'
$version = (Get-Content -LiteralPath (Join-Path $project 'VERSION') -Raw).Trim()
if ($version -notmatch '^[0-9]+(?:\.[0-9]+)+$') {
    throw "Unexpected desktop version: $version"
}

$name = "VNShelf-$version-windows-x64"
$dist = Join-Path $project 'target/dist'
$package = Join-Path $dist $name
$archive = "$package.zip"
$server = Join-Path $package 'server'
$client = Join-Path $package 'client'
$data = Join-Path $server 'data'

if (Test-Path -LiteralPath $package) {
    Remove-Item -LiteralPath $package -Recurse -Force
}
New-Item -ItemType Directory -Path $data, (Join-Path $client 'cli') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $project 'target/release/vndb-web.exe') -Destination $server
Copy-Item -LiteralPath (Join-Path $project 'target/release/vndb-explorer.exe') -Destination (Join-Path $client 'cli')
Copy-Item -LiteralPath (Join-Path $project 'web') -Destination $client -Recurse
Copy-Item -Path (Join-Path $project 'desktop/windows/*') -Destination $package -Recurse -Force
foreach ($asset in 'shelves', 'sources') {
    Copy-Item -LiteralPath (Join-Path $project "crates/core/assets/$asset.json") -Destination (Join-Path $server "$asset.example.json")
}

$utf8 = New-Object System.Text.UTF8Encoding($false)
$readme = [IO.File]::ReadAllText((Join-Path $project 'desktop/README.txt'))
[IO.File]::WriteAllText((Join-Path $package 'README.txt'), $readme.Replace('{VERSION}', $version), $utf8)
$dataReadme = 'VNDB database (catalog.sqlite), user data (user.sqlite), and cover cache are stored here.'
[IO.File]::WriteAllText((Join-Path $data 'README.txt'), $dataReadme, $utf8)
$crlf = [string][char]13 + [char]10
Get-ChildItem -LiteralPath $package -Recurse -File |
    Where-Object { $_.Extension -eq '.bat' -or $_.Name -eq 'README.txt' } |
    ForEach-Object {
        $content = [IO.File]::ReadAllText($_.FullName)
        [IO.File]::WriteAllText($_.FullName, ($content -replace '\r?\n', $crlf), $utf8)
    }

Compress-Archive -LiteralPath $package -DestinationPath $archive -Force
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
    $entries = @($zip.Entries | ForEach-Object { $_.FullName.Replace('\', '/') })
    foreach ($file in 'Start VN Shelf.bat', 'server/vndb-web.exe', 'client/cli/vndb-explorer.exe', 'client/web/index.html') {
        if ($entries -cnotcontains "$name/$file") {
            throw "Missing archive entry: $file"
        }
    }
} finally {
    $zip.Dispose()
}
Write-Host "Package: $archive"
