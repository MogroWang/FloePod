param(
  [string]$Publisher = $(if ($env:FLOEPOD_MSIX_PUBLISHER) { $env:FLOEPOD_MSIX_PUBLISHER } else { "CN=MogroWang Studio" })
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = Join-Path $repoRoot "src-tauri\target\release"
$exe = Join-Path $targetRoot "FloePod.exe"
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
  throw "未找到发布版 FloePod.exe，请先运行 pnpm tauri build"
}

$package = Get-Content -Raw -LiteralPath (Join-Path $repoRoot "package.json") | ConvertFrom-Json
if ($package.version -notmatch '^(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)$') {
  throw "MSIX 仅接受稳定三段版本号，当前为 $($package.version)"
}
$msixVersion = "$($Matches.major).$($Matches.minor).$($Matches.patch).0"
$manifestPublisher = [System.Security.SecurityElement]::Escape($Publisher)
if (-not $manifestPublisher) {
  throw "MSIX Publisher 不能为空"
}

$dist = Join-Path $repoRoot "dist"
$stage = Join-Path $dist "msix-stage"
$output = Join-Path $dist "FloePod-$($package.version)-win-x64.msix"
$resolvedDist = [System.IO.Path]::GetFullPath($dist)
$resolvedStage = [System.IO.Path]::GetFullPath($stage)
if (-not $resolvedStage.StartsWith($resolvedDist + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "MSIX 暂存目录越出 dist：$resolvedStage"
}
if (Test-Path -LiteralPath $stage) {
  Remove-Item -LiteralPath $stage -Recurse -Force
}
New-Item -ItemType Directory -Path (Join-Path $stage "Assets") -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $stage "FloePod.exe")
Copy-Item -LiteralPath (Join-Path $repoRoot "src-tauri\icons\Square44x44Logo.png") -Destination (Join-Path $stage "Assets\Square44x44Logo.png")
Copy-Item -LiteralPath (Join-Path $repoRoot "src-tauri\icons\Square150x150Logo.png") -Destination (Join-Path $stage "Assets\Square150x150Logo.png")
Copy-Item -LiteralPath (Join-Path $repoRoot "src-tauri\icons\StoreLogo.png") -Destination (Join-Path $stage "Assets\StoreLogo.png")

$manifest = @"
<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap rescap">
  <Identity Name="MogroWang.FloePod" Publisher="$manifestPublisher" Version="$msixVersion" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>浮匣 FloePod</DisplayName>
    <PublisherDisplayName>MogroWang Studio</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26100.0" />
  </Dependencies>
  <Resources><Resource Language="zh-CN" /></Resources>
  <Applications>
    <Application Id="FloePod" Executable="FloePod.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements
        DisplayName="浮匣 FloePod"
        Description="本地优先的 Windows 文件安心工作台"
        BackgroundColor="transparent"
        Square44x44Logo="Assets\Square44x44Logo.png"
        Square150x150Logo="Assets\Square150x150Logo.png" />
    </Application>
  </Applications>
  <Capabilities><rescap:Capability Name="runFullTrust" /></Capabilities>
</Package>
"@
$manifest | Set-Content -LiteralPath (Join-Path $stage "AppxManifest.xml") -Encoding utf8NoBOM

$makeAppxCommand = Get-Command MakeAppx.exe -ErrorAction SilentlyContinue
$makeAppxPath = if ($makeAppxCommand) { $makeAppxCommand.Source } else { $null }
if (-not $makeAppxPath) {
  $kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
  $makeAppxPath = Get-ChildItem -LiteralPath $kits -Filter MakeAppx.exe -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\MakeAppx\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
}
if (-not $makeAppxPath) {
  throw "未找到 MakeAppx.exe，请安装 Windows SDK"
}
New-Item -ItemType Directory -Path $dist -Force | Out-Null
& $makeAppxPath pack /d $stage /p $output /o
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $output)) {
  throw "MakeAppx 打包失败"
}

if ($env:FLOEPOD_SIGN_CERT_PATH) {
  $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if (-not $signtool) {
    throw "已请求签名但未找到 signtool.exe"
  }
  $arguments = @("sign", "/fd", "SHA256", "/tr", "http://timestamp.digicert.com", "/td", "SHA256", "/f", $env:FLOEPOD_SIGN_CERT_PATH)
  if ($env:FLOEPOD_SIGN_CERT_PASSWORD) {
    $arguments += @("/p", $env:FLOEPOD_SIGN_CERT_PASSWORD)
  }
  $arguments += $output
  & $signtool.Source @arguments
  if ($LASTEXITCODE -ne 0) { throw "MSIX Authenticode 签名失败" }
}

Remove-Item -LiteralPath $stage -Recurse -Force
Write-Output "OK $output"
