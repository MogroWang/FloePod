param([string]$Destination = "dist/github-artifacts", [string]$Commit = $(git rev-parse HEAD))
$ErrorActionPreference = "Stop"
$version = (Get-Content package.json -Raw | ConvertFrom-Json).version
$binary = Get-Item "src-tauri/target/release/FloePod.exe"
function Require-One([string]$Folder, [string]$Filter) {
  $items = @(Get-ChildItem -LiteralPath $Folder -Filter $Filter -File)
  if ($items.Count -ne 1) { throw "Expected one $Filter in $Folder; found $($items.Count)" }
  return $items[0]
}
$nsis = Require-One "src-tauri/target/release/bundle/nsis" "*.exe"
$msi = Require-One "src-tauri/target/release/bundle/msi" "*.msi"
$portable = Require-One "dist" "FloePod-$version-win-x64-portable.zip"
$msix = Require-One "dist" "FloePod-$version-win-x64.msix"
foreach ($pe in @($binary, $nsis)) {
  $actual = [Diagnostics.FileVersionInfo]::GetVersionInfo($pe.FullName).ProductVersion
  if ($actual -ne $version -and $actual -ne "$version.0") { throw "PE version mismatch: $($pe.Name) = $actual" }
}
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($msi.FullName, 0)
$view = $database.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = 'ProductVersion'")
$view.Execute()
$record = $view.Fetch()
if ($record.StringData(1) -ne $version) { throw "MSI version mismatch" }
$view.Close()

Add-Type -AssemblyName System.IO.Compression.FileSystem
$binaryHash = (Get-FileHash -LiteralPath $binary.FullName -Algorithm SHA256).Hash
foreach ($archiveFile in @($portable, $msix)) {
  $archive = [IO.Compression.ZipFile]::OpenRead($archiveFile.FullName)
  try {
    $entryPath = if ($archiveFile.Extension -eq ".zip") { "FloePod/FloePod.exe" } else { "FloePod.exe" }
    $entry = $archive.GetEntry($entryPath)
    if (-not $entry) { throw "Missing executable in $($archiveFile.Name)" }
    $stream = $entry.Open()
    try { $hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)) } finally { $stream.Dispose() }
    if ($hash -ne $binaryHash) { throw "Packaged executable differs from final executable" }
    if ($archiveFile.Extension -eq ".msix") {
      $reader = [IO.StreamReader]::new($archive.GetEntry("AppxManifest.xml").Open())
      try { [xml]$manifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
      if ($manifest.Package.Identity.Version -ne "$version.0") { throw "MSIX version mismatch" }
    }
  } finally { $archive.Dispose() }
}
$signing = "unsigned"
if ($env:FLOEPOD_SIGN_CERT_PATH) {
  ./scripts/sign-windows-artifacts.ps1 -Path @($binary.FullName, $nsis.FullName, $msi.FullName, $msix.FullName) -VerifyOnly
  $signing = "verified"
}
New-Item -ItemType Directory -Path $Destination -Force | Out-Null
if (@(Get-ChildItem -LiteralPath $Destination -File).Count -ne 0) { throw "Artifact destination must be empty" }
Copy-Item -LiteralPath $binary.FullName -Destination (Join-Path $Destination "FloePod-$version-windows-x64.exe")
foreach ($file in @($nsis, $msi, $portable, $msix)) { Copy-Item -LiteralPath $file.FullName -Destination $Destination }
@{ version = $version; commit = $Commit; signing = $signing; pullRequest = $env:RELEASE_PR; notificationRun = $env:NOTIFICATION_RUN } |
  ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Destination "release-manifest.json") -Encoding utf8NoBOM
$checksums = Get-ChildItem -LiteralPath $Destination -File | Sort-Object Name | ForEach-Object {
  "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $($_.Name)"
}
$checksums | Set-Content -LiteralPath (Join-Path $Destination "SHA256SUMS.txt") -Encoding utf8NoBOM
Write-Output "Verified version $version, five Windows packages, signing=$signing, SHA-256 checksums"
