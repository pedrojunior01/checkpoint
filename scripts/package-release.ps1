# checkpoint - Script de Empacotamento de Release
# Uso: powershell -ExecutionPolicy Bypass -File scripts\package-release.ps1

$ErrorActionPreference = "Stop"
$Version = "0.9.1"
$DistName = "checkpoint-v" + $Version + "-windows-x64"
$RootDir = Split-Path -Parent $PSScriptRoot
$DistDir = Join-Path $RootDir "dist"
$StagingDir = Join-Path $DistDir $DistName
$ZipPath = Join-Path $DistDir ($DistName + ".zip")
$ShaPath = Join-Path $DistDir ($DistName + ".zip.sha256")

Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "  checkpoint v$Version - Release Packaging" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan

# 1. Compilar binario release
Write-Host ""
Write-Host "[1/5] Compilando binario release otimizado..." -ForegroundColor Yellow
Set-Location $RootDir
cargo build --package savevault-app --release
if ($LASTEXITCODE -ne 0) {
    Write-Error "Falha na compilacao do release."
}

# 2. Localizar o binario gerado
Write-Host ""
Write-Host "[2/5] Localizando binario checkpoint.exe..." -ForegroundColor Yellow
$CandidatePaths = @(
    (Join-Path $RootDir "target\release\checkpoint.exe"),
    "C:\svt\release\checkpoint.exe"
)
$ExePath = $null
foreach ($p in $CandidatePaths) {
    if (Test-Path $p) {
        $ExePath = $p
        break
    }
}

if (-not $ExePath) {
    Write-Error "Executavel checkpoint.exe nao encontrado em target\release nem em C:\svt\release."
}
$ExeLen = (Get-Item $ExePath).Length
Write-Host "  Encontrado: $ExePath ($ExeLen bytes)" -ForegroundColor Green

# 3. Preparar diretorio de distribuicao
Write-Host ""
Write-Host "[3/5] Preparando pasta de distribuicao ($StagingDir)..." -ForegroundColor Yellow
if (Test-Path $StagingDir) {
    Remove-Item -Path $StagingDir -Recurse -Force
}
New-Item -ItemType Directory -Path $StagingDir -Force | Out-Null

Copy-Item $ExePath (Join-Path $StagingDir "checkpoint.exe") -Force
Copy-Item (Join-Path $RootDir "README.md") (Join-Path $StagingDir "README.md") -Force
Copy-Item (Join-Path $RootDir "LICENSE") (Join-Path $StagingDir "LICENSE") -Force

$DocsStaging = Join-Path $StagingDir "docs"
New-Item -ItemType Directory -Path $DocsStaging -Force | Out-Null
Copy-Item (Join-Path $RootDir "docs\*") $DocsStaging -Recurse -Force

# 4. Gerar pacote ZIP e Checksum SHA-256
Write-Host ""
Write-Host "[4/5] Gerando arquivo compactado (.zip)..." -ForegroundColor Yellow
if (Test-Path $ZipPath) {
    Remove-Item $ZipPath -Force
}
Compress-Archive -Path (Join-Path $StagingDir "*") -DestinationPath $ZipPath -CompressionLevel Optimal
$ZipLen = (Get-Item $ZipPath).Length
Write-Host "  ZIP gerado: $ZipPath ($ZipLen bytes)" -ForegroundColor Green

$Hash = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash
$HashContent = $Hash + "  " + $DistName + ".zip"
Set-Content -Path $ShaPath -Value $HashContent
Write-Host "  SHA256: $Hash" -ForegroundColor Green

# 5. Compilar Instalador Inno Setup se ISCC estiver disponivel
Write-Host ""
Write-Host "[5/5] Verificando compilador de instalador (Inno Setup)..." -ForegroundColor Yellow
$IsccPaths = @(
    (Get-Command iscc.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
)
$IsccExe = $null
foreach ($p in $IsccPaths) {
    if ($p -and (Test-Path $p)) {
        $IsccExe = $p
        break
    }
}

if ($IsccExe) {
    Write-Host "  Compilando instalador com $IsccExe..." -ForegroundColor Cyan
    & $IsccExe (Join-Path $RootDir "installer\checkpoint.iss")
    if ($LASTEXITCODE -eq 0) {
        Write-Host "  Instalador gerado com sucesso em dist!" -ForegroundColor Green
    } else {
        Write-Warning "Falha ao gerar o instalador via Inno Setup."
    }
} else {
    Write-Host "  Inno Setup nao encontrado localmente. O script de instalacao esta pronto em 'installer/checkpoint.iss'." -ForegroundColor Gray
}

Write-Host ""
Write-Host "==========================================" -ForegroundColor Green
Write-Host "  Release v$Version empacotada com sucesso!" -ForegroundColor Green
Write-Host "  Arquivos em: $DistDir" -ForegroundColor Green
Write-Host "==========================================" -ForegroundColor Green

