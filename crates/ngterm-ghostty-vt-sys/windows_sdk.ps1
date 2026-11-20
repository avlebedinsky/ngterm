$ErrorActionPreference = 'Stop'
$roots = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots'
$registry = Get-Item 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots'
foreach ($key in @($registry) + @(Get-ChildItem $registry.PSPath -Recurse | Sort-Object Name)) {
    foreach ($name in ($key.Property | Sort-Object)) {
        Write-Output "registry=$($key.Name)|$name|$($key.GetValue($name))"
    }
}
$foundSdk = $false
foreach ($name in @('KitsRoot10', 'KitsRoot81')) {
    $root = $roots.$name
    if (-not $root) { continue }
    Write-Output "$name=$root"
    $lib = Join-Path $root 'Lib'
    Write-Output "watch=$lib"
    Get-ChildItem -Directory $lib | Sort-Object Name | ForEach-Object {
        Write-Output "sdk=$($_.Name)"
        $foundSdk = $true
    }
}
if (-not $foundSdk) { throw 'No Windows SDK version' }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$instances = & $vswhere -all -products '*' -format json | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or -not $instances) { throw 'No Visual Studio installation' }
$foundMsvc = $false
foreach ($instance in ($instances | Sort-Object installationPath)) {
    $root = $instance.installationPath
    Write-Output "vs=$root $($instance.installationVersion)"
    $default = Join-Path $root 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt'
    if (-not (Test-Path $default)) { continue }
    Write-Output "watch=$default"
    Write-Output "default=$((Get-Content -Raw $default).Trim())"
    $foundMsvc = $true
    $tools = Join-Path $root 'VC\Tools\MSVC'
    Write-Output "watch=$tools"
    Get-ChildItem -Directory $tools | Sort-Object Name | ForEach-Object { Write-Output "msvc=$($_.Name)" }
}
if (-not $foundMsvc) { throw 'No MSVC tool version' }
