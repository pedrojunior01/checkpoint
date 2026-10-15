# Contributing to checkpoint 🎮

Thank you for your interest in contributing to **checkpoint**! Open source thrives when communities work together, and we welcome contributions of all kinds: bug reports, documentation improvements, new game detection patterns, cloud providers, feature requests, and code pull requests.

---

## 🧭 Code of Conduct

Please review and adhere to our [Code of Conduct](CODE_OF_CONDUCT.md) in all community interactions, issues, and pull requests. We are committed to providing a welcoming, inclusive, and harassment-free environment for everyone.

---

## 🛠️ Getting Started

### Prerequisites

* **Operating System:** Windows 10 (Build 19041+) or Windows 11 x64.
* **Rust:** 1.75+ with the MSVC toolchain (`stable-x86_64-pc-windows-msvc`).
  * Install via [rustup.rs](https://rustup.rs).
  * Ensure Visual Studio C++ Build Tools (MSVC) are installed.
* **WebView2 Runtime:** Installed by default on modern Windows 10 and 11.
* **Git:** For version control.

### Forking and Cloning

1. Fork the repository on GitHub: `https://github.com/pedrojunior01/Savevalt`
2. Clone your fork locally:
   ```powershell
   git clone https://github.com/<your-username>/checkpoint.git
   cd checkpoint
   ```
3. Add the upstream remote:
   ```powershell
   git remote add upstream https://github.com/pedrojunior01/Savevalt.git
   ```

---

## 📁 Architecture Overview

The repository is organized into 8 modular, decoupled crates:

| Crate | Responsibility |
| :--- | :--- |
| `savevault-core` | Domain types, canonical Windows path handling, system autostart (`HKCU\...\Run`). |
| `savevault-db` | Bundled SQLite in WAL mode, deterministic migrations (001-005), cascade deletions. |
| `savevault-discovery` | Game & save detection: Steam (`.acf`/`.vdf`), GOG Registry, Emulators, Ludusavi manifest (+53k games), and safe heuristic scanners. |
| `savevault-monitor` | Non-blocking Win32 file watcher (`ReadDirectoryChangesW`), debouncer, sharing violation handling (`ERROR_SHARING_VIOLATION`), and cache/noise pruning. |
| `savevault-backup` | Streaming BLAKE3 cryptographic hashing, dual-tier snapshot retention (`latest` & `daily`), and 4-step atomic safe restore with safety backups. |
| `savevault-cloud` | Cloud abstraction, automated local sync folder detection (Microsoft OneDrive & Google Drive), OAuth 2.0 PKCE, disaster recovery manifest (`savevault_manifest.json`). |
| `savevault-sync` | Asynchronous transactional sync engine with exponential backoff retry and database lock decoupling. |
| `savevault-app` | Orchestrator, Tauri v2 native desktop app, system tray, memory trimmer, 24 IPC commands, and headless CLI utilities. |

Detailed architectural documents are available in the [`docs/`](docs/) directory:
* [Architecture & Pipeline](docs/ARCHITECTURE.md)
* [Crates Deep Dive](docs/CRATES_GUIDE.md)
* [Frontend & IPC Specifications](docs/FRONTEND_AND_IPC.md)
* [Database Schema & Migrations](docs/DATABASE_SCHEMA.md)
* [Developer Guide](docs/DEVELOPMENT_GUIDE.md)

---

## 💻 Development Workflow

### Running the Desktop Application

The `desktop` feature is enabled by default:
```powershell
cargo run --package savevault-app
```

### Compiling Release Binaries

```powershell
cargo build --package savevault-app --release
```
The optimized executable will be generated at `target/release/checkpoint.exe`.

### Running in Headless Mode (CLI / Daemon)

For testing without the graphical interface or WebView2 dependencies:
```powershell
cargo run --package savevault-app --no-default-features
```

### Running the Scan Report Utility

To inspect game detection output on your system:
```powershell
cargo run --package savevault-app --bin scan_report
```

---

## 🧪 Testing & Code Quality

Before opening a pull request, make sure all tests pass and code style guidelines are met:

### 1. Run the test suite
```powershell
cargo test --workspace
```
*(Currently, 115+ automated tests must pass with zero failures).*

### 2. Format checking
```powershell
cargo fmt --all -- --check
```

### 3. Clippy linter
```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

---

## 🌿 Branching and Commits

* **Branch naming:** Use descriptive branch names:
  * `feat/your-feature-name`
  * `fix/your-bug-fix`
  * `docs/documentation-update`
* **Commit Messages:** Follow [Conventional Commits](https://www.conventionalcommits.org/):
  * `feat(discovery): add support for Epic Games save manifests`
  * `fix(monitor): handle transient file locks during autosaves`
  * `docs(readme): add internationalization guide`
  * `perf(memory): reduce idle working set`

---

## 📥 Submitting Pull Requests

1. Keep your branch up to date with `upstream/main`:
   ```powershell
   git fetch upstream
   git rebase upstream/main
   ```
2. Push your feature branch to your fork:
   ```powershell
   git push origin feat/your-feature-name
   ```
3. Open a Pull Request against the `main` branch of `pedrojunior01/Savevalt`.
4. Fill out the Pull Request template completely, describing the rationale, changes made, and test results.
5. Participate in code review discussions and address any requested changes.

---

## 💬 Community & Help

If you have questions, ideas, or need guidance on implementation details, feel free to open a [GitHub Discussion](https://github.com/pedrojunior01/Savevalt/discussions) or create an issue.

Thank you for helping make **checkpoint** the best game save companion on Windows! 🚀

