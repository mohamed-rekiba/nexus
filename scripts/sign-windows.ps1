# Signs Windows files with Authenticode when WINDOWS_CERT_PFX (the certificate, base64) and
# WINDOWS_CERT_PASSWORD are set. Without them it warns and signs nothing: SmartScreen then
# warns on the first start.
#   pwsh scripts/sign-windows.ps1 <file or wildcard>...
if (-not $env:WINDOWS_CERT_PFX) {
    Write-Output "::warning::no WINDOWS_CERT_PFX: $args not signed"
    exit 0
}
$pfx = Join-Path $env:RUNNER_TEMP "cert.pfx"
[IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERT_PFX))
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
    Sort-Object FullName | Select-Object -Last 1
Get-ChildItem $args | ForEach-Object {
    & $signtool.FullName sign /f $pfx /p $env:WINDOWS_CERT_PASSWORD /fd sha256 `
        /tr http://timestamp.digicert.com /td sha256 $_.FullName
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
Remove-Item $pfx
