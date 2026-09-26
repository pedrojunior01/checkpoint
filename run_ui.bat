@echo off
title SaveVault Launcher
cd /d "%~dp0"
echo Iniciando SaveVault DEV-UI-01 (Tauri)...
start "" "%~dp0target\debug\savevault.exe"
