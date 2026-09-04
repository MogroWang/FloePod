param([Parameter(Mandatory = $true)][string[]]$Path)

$ErrorActionPreference = "Stop"
if (-not $env:FLOEPOD_SIGN_CERT_PATH) {
  throw "缺少 FLOEPOD_SIGN_CERT_PATH"
}
$signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
if (-not $signtool) { throw "未找到 signtool.exe" }

foreach ($candidate in $Path) {
  $resolved = Resolve-Path -LiteralPath $candidate -ErrorAction Stop
  $arguments = @("sign", "/fd", "SHA256", "/tr", "http://timestamp.digicert.com", "/td", "SHA256", "/f", $env:FLOEPOD_SIGN_CERT_PATH)
  if ($env:FLOEPOD_SIGN_CERT_PASSWORD) {
    $arguments += @("/p", $env:FLOEPOD_SIGN_CERT_PASSWORD)
  }
  $arguments += $resolved.Path
  & $signtool.Source @arguments
  if ($LASTEXITCODE -ne 0) { throw "签名失败：$($resolved.Path)" }
  & $signtool.Source verify /pa /all $resolved.Path
  if ($LASTEXITCODE -ne 0) { throw "签名验证失败：$($resolved.Path)" }
}
