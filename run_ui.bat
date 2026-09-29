@echo off
title SaveVault Launcher
cd /d "%~dp0"

if not exist "%~dp0target\debug\savevault.exe" (
    echo [SaveVault] Compilando executavel pela primeira vez...
    cargo build --bin savevault
    if %errorlevel% neq 0 (
        echo [ERRO] Falha ao compilar o SaveVault.
        pause
        exit /b %errorlevel%
    )
)

echo [SaveVault] Iniciando SaveVault (Interface Grafica Nativa Tauri)...
start "" "%~dp0target\debug\savevault.exe"
