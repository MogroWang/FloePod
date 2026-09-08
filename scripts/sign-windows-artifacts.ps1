param([Parameter(Mandatory = $true)][string[]]$Path, [switch]$VerifyOnly)

$ErrorActionPreference = "Stop"
if (-not $env:FLOEPOD_SIGN_CERT_PATH) {
  throw "缺少 FLOEPOD_SIGN_CERT_PATH"
}
$signtool = (Get-Command signtool.exe -ErrorAction SilentlyContinue).Source
if (-not $signtool) {
  $kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
  $signtool = Get-ChildItem -LiteralPath $kits -Filter signtool.exe -Recurse -File |
    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
    Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $signtool) { throw "未找到 Windows SDK signtool.exe" }

foreach ($candidate in $Path) {
  $resolved = Resolve-Path -LiteralPath $candidate -ErrorAction Stop
  $arguments = @("sign", "/fd", "SHA256", "/tr", "http://timestamp.digicert.com", "/td", "SHA256", "/f", $env:FLOEPOD_SIGN_CERT_PATH)
  if ($env:FLOEPOD_SIGN_CERT_PASSWORD) {
    $arguments += @("/p", $env:FLOEPOD_SIGN_CERT_PASSWORD)
  }
  $arguments += $resolved.Path
  if (-not $VerifyOnly) {
    & $signtool @arguments
    if ($LASTEXITCODE -ne 0) { throw "签名失败：$($resolved.Path)" }
  }
  & $signtool verify /pa /all $resolved.Path
  if ($LASTEXITCODE -ne 0) { throw "签名验证失败：$($resolved.Path)" }
}
