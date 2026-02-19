# MasterBooter

A single portable executable Windows deployment toolkit for IT professionals and MSP/repair shops.

MasterBooter combines four major functions into one tool: Backup/Restore, Windows Deployment, WinPE/RE Building, and System Preparation. It runs in live Windows, WinPE, and WinRE environments.

## Features

### Backup / Restore
- **Profile backup** via Fab's AutoBackup, ProfWiz, and Transwiz
- **Disk imaging** via Disk2VHD and HDD Raw Copy Tool
- **Product key backup** — detect and save your Windows key before reinstalling
- Tools are downloaded on first use and cached locally

### Windows Deploy
- **Normal Install** — interactive setup.exe with optional post-install scripts
- **Automated Install** — full unattended deployment via autounattend.xml
- 50+ configuration fields: user accounts, OOBE, privacy, security, performance, UI tweaks
- Bloatware removal (Cortana, OneDrive, Teams, Copilot, Widgets)
- Post-install script support (PowerShell, batch, registry files)
- Generic product keys auto-fill to select the correct edition
- Save/load deployment profiles for repeatable installs

### WinPE Builder
- Build bootable WinPE ISO from Windows ADK
- WiFi support injection (Intel, Broadcom, Realtek, Qualcomm, Ralink, Marvell)
- 16 optional ADK packages with dependency resolution
- PE fixes: DPI scaling, wallpaper, fonts, user profiles
- Bundled tools: 7-Zip, CrystalDiskInfo, Explorer++, PENetwork, and more
- WinRE-based builds (uses recovery partition — no ADK required)

### System Prep
- Download and launch SysprepPreparator for guided sysprep workflow
- Pre-flight compatibility checks
- System cleanup and pending operation resolution

## Quick Start

1. Download `masterbooter.exe` from [Releases](https://github.com/Howweird/Masterbooter/releases)
2. Run it — no installation needed (single portable EXE, ~9 MB)
3. Choose your mode from the sidebar: Backup, Deploy, WinPE, or System Prep

## Requirements

- **Windows 10 or 11** (x64)
- **Windows ADK** — required only for WinPE Builder (not needed for other modes)
- **Administrator privileges** — needed for WinPE building and deployment

## Building from Source

### Prerequisites
- [Rust](https://rustup.rs/) (stable toolchain)
- Visual Studio Build Tools (C++ workload)
- For cross-compilation from ARM64: `rustup target add x86_64-pc-windows-msvc`

### Build Commands

```bash
# Debug build (runs on your machine)
cargo build

# Release build (x64 target for deployment)
cargo build --release --target x86_64-pc-windows-msvc

# Check for errors without building
cargo check
```

The release binary is at `target/x86_64-pc-windows-msvc/release/masterbooter.exe`.

## Project Structure

```
src/
  main.rs           Entry point and UI callbacks
  deploy.rs         Windows deployment (autounattend.xml generation)
  winpe.rs          WinPE builder + WiFi injection
  tools.rs          Tool management (backup + PE tools)
  adk_packages.rs   ADK optional component management
  pe_fixes.rs       PE fixes (DPI, fonts, wallpaper)
  ui/main.slint     Full UI layout (Slint framework)

backup_tools/       Tools for Backup/Restore (downloaded on first use)
pe_tools/           Tools bundled into WinPE (tool.toml manifests)
assets/             Embedded resources (wallpaper)
docs/               Additional documentation
```

## Technology

- **Language**: Rust
- **UI Framework**: [Slint](https://slint.dev/) with software rendering (no GPU required)
- **Why Rust/Slint?** WPF and other .NET frameworks don't work in WinPE. Slint's software renderer runs everywhere, including minimal Windows environments.

## License

[MIT](LICENSE)
