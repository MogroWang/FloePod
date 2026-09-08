$ErrorActionPreference = 'Stop'
$arguments = @('tauri', 'bundle', '--ci', '--bundles', 'nsis,msi')
$signingConfig = $null
try {
  if ($env:FLOEPOD_SIGN_CERT_PATH) {
    # Tauri patches the executable separately for NSIS and MSI. Its callback
    # signs each patched executable and installer, including the uninstaller.
    $signingConfig = Join-Path ([IO.Path]::GetTempPath()) ("floepod-signing-{0}.json" -f [Guid]::NewGuid())
    $config = @{ bundle = @{ windows = @{ signCommand = @{
      cmd = 'pwsh'
      args = @('-NoProfile', '-File', (Join-Path $PSScriptRoot 'sign-windows-artifacts.ps1'), '-Path', '%1')
    } } } }
    $config | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $signingConfig -Encoding utf8
    $arguments += @('--config', $signingConfig)
  }
  & pnpm @arguments
  if ($LASTEXITCODE -ne 0) { throw 'Tauri bundle failed' }
  # Tauri restores the original executable after bundling. Sign this restored
  # file before MSIX/portable packaging so those formats contain final bytes.
  if ($env:FLOEPOD_SIGN_CERT_PATH) {
    & (Join-Path $PSScriptRoot 'sign-windows-artifacts.ps1') -Path 'src-tauri/target/release/FloePod.exe'
  }
  & (Join-Path $PSScriptRoot 'package-msix.ps1')
} finally {
  if ($signingConfig -and (Test-Path -LiteralPath $signingConfig)) {
    Remove-Item -LiteralPath $signingConfig -Force
  }
}
