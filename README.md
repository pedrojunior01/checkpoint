# checkpoint 🎮

<div align="center">

[![CI](https://github.com/pedrojunior01/Savevalt/actions/workflows/ci.yml/badge.svg)](https://github.com/pedrojunior01/Savevalt/actions/workflows/ci.yml)
[![Release](https://github.com/pedrojunior01/Savevalt/actions/workflows/release.yml/badge.svg)](https://github.com/pedrojunior01/Savevalt/releases)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Tauri](https://img.shields.io/badge/tauri-v2-blue.svg)](https://v2.tauri.app)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078d7.svg)](https://microsoft.com/windows)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**English | [Português (Brasil)](README.pt-BR.md)**

<p align="center">
  <b>Smart, lightweight automatic game save backup & synchronization engine for Windows.</b><br>
  Built 100% in <b>Rust</b> with a native <b>Tauri v2</b> desktop UI, ultra-light footprint (<b>15MB to 25MB RAM</b>), quiet <b>System Tray</b> operation, dual-layer restore protection (<b>Latest & Daily</b>), and hybrid cloud integration (<b>Google Drive</b> & <b>Microsoft OneDrive</b>).
</p>

</div>

---

## 🚀 What is checkpoint?

**checkpoint** runs silently in the background while you game, driven by one primary mission: **to ensure that not a single minute of your progress is ever lost**.

It automatically discovers your installed games and their save folders, monitors file writes in real time with adaptive debouncing, verifies integrity using **BLAKE3** cryptographic hashes, maintains versioned local snapshots across two safety tiers (**Latest** and **Daily**), and transactionally synchronizes backups to your personal cloud storage (Google Drive and Microsoft OneDrive).

---

## ✨ Key Features

| Feature | Description |
| :--- | :--- |
| 🔍 **Multi-Platform Discovery** | Automatically locates games via Steam (`.acf`/`.vdf`), GOG Galaxy (32/64-bit Registry), emulators (Dolphin, PCSX2, PPSSPP, RetroArch), and safe heuristic scanners restricted strictly to authentic save directories. |
| 📖 **Ludusavi Manifest (+53,000 Games)** | Built-in offline mapping with dynamic Win32 path resolution (`<winAppData>`, `<winDocuments>`, `<winSavedGames>`). |
| 🖼️ **High-Resolution Covers (600x900)** | Official Steam vertical artwork resolution for both store titles and generic games via Steam Store Search API and local persistent caching (`steam_covers.json`). |
| 🧹 **Intelligent Anti-Noise Filtering** | Preventative pruning of transient engine caches (`.DDS` files, `cache`, `CrashDumps`, `shadercache`, `.crash`), avoiding false notifications and database bloat. |
| 👁️ **Non-Blocking Win32 Watcher** | Real-time `ReadDirectoryChangesW` monitoring with adaptive debouncing and native resilience against file sharing locks (`ERROR_SHARING_VIOLATION`). |
| 🔐 **BLAKE3 Cryptographic Integrity** | Constant $O(1)$ memory streaming hash computation (64 KB chunks) with zero data duplication. |
| 🛡️ **Dual-Tier Protection: Latest & Daily** | Preserves the most recent snapshot in real-time (**Latest**) and automatically records a safety milestone at the start of each day (**Daily**), allowing you to revert in case of save corruption during today's session. |
| ☁️ **Hybrid Cloud & Disaster Recovery** | Seamless synchronization to Google Drive (`G:\My Drive\checkpoint`) and Microsoft OneDrive with a central disaster recovery manifest (`savevault_manifest.json`) for effortless migration to new PCs. |
| 🖥️ **Native Desktop Interface (Tauri v2)** | Frameless modern UI with custom titlebar, dark/light themes, zero NodeJS runtime overhead, and minimize-to-tray background operation. |
| ⚡ **Aggressive RAM Trimming (15MB - 25MB)** | Active Working Set memory trimming (`SetProcessWorkingSetSize`) across the main process and WebView2 subprocesses on minimize, hide, and idle cycles. |
| 🛡️ **4-Step Atomic Safe Restore** | Mandatory generation of safety backups (`.savevault_safety_*`) prior to touching live files, utilizing atomic Win32 filesystem replacements (`MoveFileExW`). |
| 🚫 **Game Management & Blacklist** | Remove individual games from monitoring directly from their modal and automatically prevent unwanted rediscoveries via SQLite blacklists. |

---

## 📋 System Requirements

* **Operating System:** Windows 10 (Build 19041+) or Windows 11 x64.
* **Rust:** 1.75+ with the MSVC toolchain (`stable-x86_64-pc-windows-msvc`).
* **WebView2 Runtime:** Pre-installed on Windows 10 and 11.

---

## ⚡ Installation & Build

Clone the repository and build using Cargo:

```powershell
git clone https://github.com/pedrojunior01/Savevalt.git checkpoint
cd checkpoint
```

### 1. Run the Desktop Application (Default Mode)
The `desktop` feature is enabled by default:
```powershell
cargo run -p savevault-app
```

### 2. Build the Standalone Optimized Release Binary
```powershell
cargo build -p savevault-app --release
```
The standalone executable will be generated at:
```
target\release\checkpoint.exe
```

### 3. Run in Headless Mode (CLI / Daemon)
For servers, CI pipelines, or headless terminals without GUI dependencies:
```powershell
# Run detection scanner and print a formatted terminal report
cargo run -p savevault-app --bin scan_report

# Run the background daemon continuously in the terminal
cargo run -p savevault-app --no-default-features
```

---

## 🧪 Automated Test Suite

checkpoint features full end-to-end and unit test coverage with over 115 automated tests verifying persistence, concurrency, Win32 path normalization, and synchronization:

```powershell
cargo test --workspace
```

---

## 📁 Workspace Architecture

The codebase is split into 8 modular, decoupled crates:

```
checkpoint/
├── crates/
│   ├── savevault-core/       # Domain models, Windows path canonicalization, autostart (HKCU)
│   ├── savevault-db/         # Bundled SQLite in WAL mode with migrations 001-005
│   ├── savevault-discovery/  # Steam, GOG, Emulators, Ludusavi manifest, and heuristic scanners
│   ├── savevault-monitor/    # Non-blocking watcher, debouncing, and cache filter
│   ├── savevault-backup/     # Streaming BLAKE3 hashing, snapshot manager, and 4-step safe restore
│   ├── savevault-cloud/      # Cloud providers, local sync folder detection (OneDrive/GDrive), OAuth 2.0
│   ├── savevault-sync/       # Async transactional sync engine with exponential backoff
│   └── savevault-app/        # checkpoint.exe binary, Tauri v2 Desktop app, 24 IPC commands, CLI
├── docs/                     # Technical documentation & architecture guides
│   ├── ARCHITECTURE.md       # End-to-end pipeline architecture & concurrency model
│   ├── CRATES_GUIDE.md       # Technical deep-dive into each workspace crate
│   ├── DATABASE_SCHEMA.md    # SQLite tables, migrations, and cascade relationships
│   ├── DEVELOPMENT_GUIDE.md  # Guide for contributors and development practices
│   ├── FRONTEND_AND_IPC.md   # Complete catalog of all 24 IPC commands & design system
│   ├── PROJECT_STATUS.md     # Current milestone status, test metrics, and bugfix history
│   ├── README.md             # Technical documentation index
│   └── SPECIFICATION.md      # Functional specifications and business rules
└── Cargo.toml                # Root Cargo workspace manifest
```

---

## 🤝 Contributing

We welcome contributions! Whether you want to add support for new launchers/emulators, optimize memory performance, enhance the UI, or report issues:

1. Read our [Contributing Guidelines (CONTRIBUTING.md)](CONTRIBUTING.md).
2. Adhere to our [Code of Conduct (CODE_OF_CONDUCT.md)](CODE_OF_CONDUCT.md).
3. Check the open issues or submit a Pull Request following our [PR Template](.github/PULL_REQUEST_TEMPLATE.md).

---

## 📖 In-Depth Documentation

For architectural specifications and developer details, check the [`docs/`](docs/) directory:

* [🏛️ System Architecture](docs/ARCHITECTURE.md)
* [📦 Crates Guide](docs/CRATES_GUIDE.md)
* [🖥️ Frontend & IPC Contract](docs/FRONTEND_AND_IPC.md)
* [🗄️ Database Schema](docs/DATABASE_SCHEMA.md)
* [🛠️ Developer Guide](docs/DEVELOPMENT_GUIDE.md)
* [📊 Project Status & Roadmap](docs/PROJECT_STATUS.md)

---

## 📄 License

Distributed under the **MIT License**. See [LICENSE](LICENSE) for details.
