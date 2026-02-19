# Development Guide

This guide helps you set up your development environment and make changes to MasterBooter.
Written for complete beginners - no programming experience assumed.

---

## Table of Contents
1. [What You Need to Install](#what-you-need-to-install)
2. [Setting Up VSCode](#setting-up-vscode)
3. [Understanding the Project](#understanding-the-project)
4. [How to Build the Program](#how-to-build-the-program)
5. [How to Make Changes](#how-to-make-changes)
6. [Common Tasks](#common-tasks)
7. [Troubleshooting](#troubleshooting)

---

## What You Need to Install

### 1. Visual Studio Code (VSCode)
VSCode is a free code editor from Microsoft.

**Download**: https://code.visualstudio.com/

**Installation steps**:
1. Download the installer
2. Run the installer
3. Check "Add to PATH" when asked
4. Check "Add 'Open with Code' to context menu" (useful!)
5. Finish installation

### 2. Rust Toolchain
Rust is the programming language we use. It compiles to native code.

**Download**: https://rustup.rs/

**Installation steps**:
1. Download and run `rustup-init.exe`
2. Choose "Proceed with installation (default)"
3. Wait for installation to complete
4. Restart your terminal/VSCode

**Verify installation**: Open Command Prompt and type:
```
rustc --version
cargo --version
```
You should see version numbers like `rustc 1.XX.X` and `cargo 1.XX.X`

### 3. Visual Studio Build Tools
Rust on Windows needs the MSVC compiler. This is installed automatically with rustup, but if you have issues:

**Download**: https://visualstudio.microsoft.com/visual-cpp-build-tools/

Install "Desktop development with C++" workload.

### 4. Git
Git tracks changes to your code and lets you upload to GitHub.

**Download**: https://git-scm.com/download/win

**Installation steps**:
1. Download the installer
2. Run it
3. Accept defaults for most options
4. For "Adjusting your PATH", select "Git from the command line and also from 3rd-party software"

**Verify installation**: Open Command Prompt and type:
```
git --version
```

### 5. VSCode Extensions
After installing VSCode, add these extensions:

1. Open VSCode
2. Click the Extensions icon (4 squares) on the left sidebar
3. Search for and install:
   - **rust-analyzer** (by rust-lang) - Required for Rust development
   - **Slint** (by Slint) - For Slint UI files
   - **GitLens** (by GitKraken) - Makes Git easier to use
   - **Even Better TOML** (by tamasfe) - For Cargo.toml files

---

## Setting Up VSCode

### Opening the Project
1. Open VSCode
2. Go to File → Open Folder
3. Navigate to `C:\MasterBooter`
4. Click "Select Folder"

### Understanding the VSCode Interface
```
┌─────────────────────────────────────────────────────────┐
│ File  Edit  View  ...                    [Search Bar]  │
├────────┬────────────────────────────────────────────────┤
│        │                                                │
│ Files  │  [Editor Area - where you edit code]          │
│ List   │                                                │
│        │                                                │
│        │                                                │
├────────┴────────────────────────────────────────────────┤
│ Terminal / Problems / Output                            │
└─────────────────────────────────────────────────────────┘
```

**Key areas**:
- **Left sidebar**: File explorer, search, Git, extensions
- **Editor**: Where you view and edit files
- **Bottom panel**: Terminal (command line), error messages

### Useful Keyboard Shortcuts
| Shortcut | What it does |
|----------|--------------|
| Ctrl+S | Save the current file |
| Ctrl+Shift+S | Save all files |
| Ctrl+P | Quick open file by name |
| Ctrl+Shift+F | Search across all files |
| Ctrl+` | Open/close terminal |
| Ctrl+B | Show/hide sidebar |
| F5 | Run the program (debug mode) |

---

## Understanding the Project

### Project Structure
```
C:\MasterBooter\
├── CLAUDE.md           # Instructions for Claude Code AI
├── VISION.md           # Project goals and design
├── REQUIREMENTS.md     # Feature list and tracking
├── DECISIONS.md        # Technical decisions made
├── DEVELOPMENT.md      # This file - your guide
│
├── src/                # Rust source code
│   ├── main.rs         # Application entry point + UI callbacks
│   ├── winpe.rs        # WinPE builder + WiFi extraction
│   ├── deploy.rs       # Windows Deploy (autounattend.xml, profiles)
│   ├── tools.rs        # Tool management (backup + PE)
│   ├── adk_packages.rs # ADK optional components
│   ├── pe_fixes.rs     # PE fixes and wallpaper registry
│   ├── updater.rs      # Auto-update from GitHub releases
│   └── ui/             # Slint UI files
│       └── main.slint  # Main window layout
│
├── assets/             # Files embedded into the EXE at compile time
│   ├── wallpaper.jpg   # Branding wallpaper (917 KB)
│   ├── icon.ico        # EXE icon (multi-size: 16/32/48/256px)
│   ├── icon.png        # Window icon for Slint (256px)
│   └── Masterbooter.svg # Logo for GitHub/README
│
├── Cargo.toml          # Rust dependencies (like package.json)
├── build.rs            # Build configuration
│
├── backup_tools/       # Tools for Backup/Restore (downloaded on first use)
├── pe_tools/           # Tools bundled INTO WinPE (downloaded at build time)
├── drivers/            # Driver packages for PE injection
├── dist/               # Distribution folder (EXE + pe_tools ready to deploy)
├── docs/               # Documentation
└── archive/            # Old C# prototype (reference only)
```

### What Are These File Types?

| Extension | What it is |
|-----------|------------|
| `.rs` | Rust code file - the actual program logic |
| `.slint` | Slint UI file - defines how screens look |
| `.toml` | Configuration file (like Cargo.toml for dependencies) |
| `.md` | Markdown file - documentation (like this file) |

### Key Concepts

**Rust**: A systems programming language that compiles to native code. Example:
```rust
// This is a comment - the compiler ignores it
fn main() {
    // println! prints text to the console
    println!("Hello, World!");
}
```

**Slint**: A UI framework for Rust. Example:
```slint
// This creates a window with a button
export component MainWindow inherits Window {
    Button {
        text: "Click Me";
    }
}
```

**Cargo**: Rust's build tool and package manager. It:
- Downloads dependencies
- Compiles your code
- Runs your program

---

## How to Build the Program

### Important: ARM Development → x64 Deployment
You are developing on an ARM64 laptop but deploying to x64 (Intel/AMD) systems.

- **Local testing**: Use `cargo build` (builds for ARM, runs on your laptop)
- **Distribution**: Use `cargo build --release --target x86_64-pc-windows-msvc`

### First Time Build
1. Open Terminal in VSCode (Ctrl+`)
2. Navigate to the project folder:
   ```
   cd C:\MasterBooter
   ```
3. Build the program:
   ```
   cargo build
   ```

### Running the Program (Local Testing)
In the terminal:
```
cargo run
```

### Building a Release Version (for x64 Users)
First, add the x64 target (one time only):
```
rustup target add x86_64-pc-windows-msvc
```

Then build:
```
cargo build --release --target x86_64-pc-windows-msvc
```

This creates a standalone EXE at:
```
target\x86_64-pc-windows-msvc\release\masterbooter.exe
```

**Note**: The x64 release build will NOT run on your ARM laptop. You need an x64 machine to test it.

### Copying to Another Machine for Testing

**Important**: MasterBooter is designed to run from a **USB flash drive** or removable drive, NOT from `C:\`. It stores all its data (saved keys, profiles, downloaded tools, settings) next to the EXE. Running from USB means your configuration travels between machines.

The EXE needs its tool folders next to it. Copy these to a **USB flash drive**:

```
USB Drive (E:\)/
├── masterbooter.exe          # From target\x86_64-pc-windows-msvc\debug\ (or release\)
├── pe_tools/                 # Entire folder (contains tool.toml manifests)
│   ├── shell/WinXShell/tool.toml
│   ├── shell/Explorer++/tool.toml
│   └── ...
├── backup_tools/             # Entire folder (contains backup tool configs)
│   ├── fabs/
│   └── ...
├── profiles/                 # Created automatically (saved deployment profiles)
├── FirstLogon/               # Created automatically (post-install scripts)
└── saved_keys.json           # Created automatically (saved product keys)
```

**Why USB and not C:\?** MasterBooter is a portable toolkit for IT pros. You back up a key on Machine A, walk to Machine B, and deploy with that key. If the EXE is on C:\, your saved keys and profiles stay on Machine A. On a USB drive, everything travels with you.

**Common mistake**: Running `cargo build` on an ARM laptop produces an ARM EXE that will NOT run on x64 machines. You'll get "This app can't run on your PC." Always use `cargo build --target x86_64-pc-windows-msvc` when building for x64 machines.

**The target machine also needs**:
- Windows ADK + WinPE add-on (for PE building)
- Internet connection (to download PE tools from GitHub at build time)

---

## How to Make Changes

### Example 1: Change Button Text (in Slint)

1. Open the `.slint` file containing the button (e.g., `src/ui/main.slint`)
2. Find the button:
   ```slint
   Button {
       text: "Old Text";
   }
   ```
3. Change the text:
   ```slint
   Button {
       text: "New Text";
   }
   ```
4. Save (Ctrl+S)
5. Run to see the change (`cargo run`)

### Example 2: Change What Happens When Button is Clicked

1. Find the button in the `.slint` file:
   ```slint
   Button {
       text: "Click Me";
       clicked => { /* callback name */ }
   }
   ```
2. Open the corresponding `.rs` file
3. Find the callback handler:
   ```rust
   ui.on_button_clicked(|| {
       // Current code here
   });
   ```
4. Modify the code inside the `{ }` brackets
5. Save and run

### Example 3: Add a New Button

1. Open the `.slint` file
2. Add a new Button element:
   ```slint
   Button {
       text: "New Button";
       clicked => { new-button-clicked() }
   }
   ```
3. Define the callback in the component:
   ```slint
   callback new-button-clicked();
   ```
4. Open the `.rs` file
5. Add the callback handler:
   ```rust
   ui.on_new_button_clicked(|| {
       println!("New button was clicked!");
   });
   ```
6. Save both files and run

---

## Common Tasks

### Adding a New Dependency
1. Open `Cargo.toml`
2. Add the dependency under `[dependencies]`:
   ```toml
   [dependencies]
   slint = "1.x"
   serde = "1.0"  # Add new dependency here
   ```
3. Run `cargo build` to download and compile

### Finding Where Something Is Defined
1. Press Ctrl+Shift+F to search all files
2. Type what you're looking for
3. Click on results to jump to that location

### Undoing Changes
If you made a mistake:
- **Undo in file**: Ctrl+Z
- **Discard all changes to a file**: Right-click file → Git → Discard Changes
- **Go back to last commit**: Ask Claude Code for help

### Saving Your Work with Git
```bash
# See what changed
git status

# Add your changes
git add .

# Save with a message
git commit -m "Describe what you changed"

# Upload to GitHub (after setting up repo)
git push
```

---

## Troubleshooting

### "cargo is not recognized"
- Restart VSCode or your computer
- Reinstall Rust via rustup

### Build Errors
- Read the error message - Rust has very helpful error messages!
- Common issues:
  - Missing semicolon `;` at end of line
  - Mismatched brackets `{ }`
  - Typo in a name
  - Missing `use` statement for imports

### VSCode Not Recognizing Rust
- Make sure rust-analyzer extension is installed
- Restart VSCode
- Check that `Cargo.toml` exists in the project root

### Program Won't Run
- Check the Terminal for error messages
- Make sure you saved all files (Ctrl+Shift+S)
- Try rebuilding: `cargo clean` then `cargo build`

### Slint UI Not Updating
- Make sure you saved the `.slint` file
- Restart the application

---

## Getting Help

### From Claude Code
You can ask Claude Code (this AI) to:
- Explain what code does
- Help fix errors
- Make specific changes
- Show you how to do something

Just describe what you want in plain English!

### Online Resources
- **Rust Book**: https://doc.rust-lang.org/book/
- **Slint Documentation**: https://slint.dev/docs/
- **Stack Overflow**: https://stackoverflow.com/ (search for errors)

---

## Archived C# Prototype

The original C#/WPF prototype is saved in the `archive/` folder. It contains:
- UI layout reference
- Tool management logic (download, launch, update)
- Settings persistence pattern

This code won't run (wrong technology) but serves as reference for porting features to Rust.

---

## WinPE Builder Architecture

### How PE Building Works

The WinPE Builder uses Windows ADK (Assessment and Deployment Kit) to create bootable PE images.

**Build Flow**:
```
 1. Detect ADK installation
 2. Run copype.cmd (creates WinPE base structure)
 3. Mount boot.wim with DISM
 4. Install ADK packages (WMI, PowerShell, NetFX, etc.)
 5. Apply PE fixes (DPI, WallpaperHost, fonts, crash dialogs, long paths)
 6. Extract WiFi files from ISO's install.wim (drivers, DLLs, schemas)
 7. Inject WiFi drivers via DISM + copy for drvload fallback
 8. Inject WLAN service infrastructure (DLLs, registry, L2Schemas)
 9. Inject branding wallpaper (embedded in EXE)
10. Inject PE tools to X:\Tools\
11. Create launcher script + winpeshl.ini + desktop shortcuts
12. Unmount and commit WIM changes
13. Export single WIM image (index 1 only)
14. Run MakeWinPEMedia to create bootable ISO
15. Verify ISO (5-point check: size, ISO9660, El Torito, boot files)
```

### Key Files for PE Building

| File | Purpose | Lines |
|------|---------|-------|
| `src/winpe.rs` | PE building, WiFi extraction from ISO, branding | ~5,600 |
| `src/deploy.rs` | Windows Deploy (XML, diskpart, profiles, scripts, normal/automated) | ~2,900 |
| `src/tools.rs` | Tool management (backup + PE + sysprep tools) | ~1,900 |
| `src/updater.rs` | Auto-update from GitHub releases (check, download, self-replace) | ~500 |
| `src/adk_packages.rs` | ADK optional component management | ~960 |
| `src/pe_fixes.rs` | PE fixes (DPI, fonts, WallpaperHost, wallpaper registry) | ~980 |
| `src/main.rs` | Application entry point, UI callbacks | ~2,400 |
| `src/ui/main.slint` | Full UI layout (sidebar + all pages) | ~3,800 |
| `assets/wallpaper.jpg` | Branding wallpaper (embedded in EXE at compile time) | 917 KB |
| `assets/icon.ico` | EXE icon (multi-size ICO, embedded via winres) | 3.7 KB |
| `assets/icon.png` | Window icon (256px PNG, referenced in Slint) | 1.8 KB |
| `pe_tools/` | PE tools organized by category | 12 tool.toml manifests |

### The Launcher Script Approach

Instead of launching tools directly from `winpeshl.ini`, we use a launcher script (like AMPIPIT does):

**winpeshl.ini**:
```ini
[LaunchApps]
X:\Tools\Launchers\launch.cmd
```

**launch.cmd does**:
1. Runs `wpeinit` (initializes hardware/drivers)
2. Loads drivers from X:\Drivers (baked in) and USB Drivers/ folders (drvload)
3. Adjusts mouse speed for touchpads
4. Creates user profile folders
5. Sets environment variables (USERPROFILE, APPDATA, etc.)
6. Starts network services (Eaphost, dot3svc, wlansvc)
7. Creates desktop shortcuts via PowerShell
8. Launches auto-start tools (PENetwork)
9. Launches the shell (WinXShell)

This approach is critical because many tools expect environment variables and folders to exist.

### PE Tools Folder Structure

```
pe_tools/
├── shell/              # Desktop shells & file managers
│   ├── WinXShell/      # Main shell with Start menu (always default)
│   ├── Explorer++/     # Tabbed file manager
│   └── FileExplorer/   # Dual-pane file explorer (.NET 4.8)
├── network/
│   ├── PENetwork/      # WiFi/network configuration GUI
│   └── WebBrowser/     # Compact web browser (.NET 4.8)
├── disk/
│   ├── CrystalDiskInfo/  # SMART health monitor
│   └── DiskCheck/        # SMART status via smartctl (.NET 4.8)
├── system/
│   └── DISMTool/       # GUI for DISM operations (.NET 4.8)
├── utilities/
│   ├── 7-Zip/          # Portable file archiver
│   ├── Autoruns/       # Sysinternals startup manager
│   ├── EventViewer/    # Windows event log viewer (.NET 4.8)
│   └── InstalledSoftware/  # Software inventory (.NET 4.8)
```

**12 tools total** — 6 are .NET Framework 4.8 apps (from pcassistsoftware.co.uk) that require the `WinPE-NetFx` ADK package (enabled by default).

**Note**: WiFi drivers and WLAN DLLs are extracted from the ISO's install.wim at build time (not from the local machine — the PE could be for a different machine). The branding wallpaper is embedded directly in the EXE. See DECISIONS.md ADR-008.

Each tool folder contains a `tool.toml` that defines:
- Tool name and description
- Executable filename
- Whether it's a shell, whether to auto-launch
- `download_url` — primary download (manufacturer site)
- `fallback_url` — GitHub mirror if primary fails (`Howweird/MasterBooter-Tools` releases)

**Download behavior**: The "Download All" button downloads only tools with checked checkboxes. If the primary download fails, the fallback URL is tried automatically. Archives that extract into a subfolder are automatically flattened.

### Reference Implementation

AMPIPIT uses the same approach successfully:
- Location: `C:\Users\howar\ClaudeSourceFiles\AMPIPIT\`
- Key file: `src/cli/build.rs`
- Function: `configure_autolaunch()` creates the launcher script

---

## Windows Deploy Architecture

### How Windows Deployment Works

The Windows Deploy module offers two modes: **Normal Install** (interactive setup.exe) and **Automated Install** (unattended with autounattend.xml). It works in both Live Windows (preview/test XML) and WinPE (full deployment).

**Mode Selection**: Card-based UI presents Normal vs Automated when user clicks Windows Deploy.

**Normal Install Flow**:
```
 1. Browse for WIM/ESD/ISO image → Parse editions with DISM
 2. Select edition from ComboBox dropdown
 3. Optionally add FirstLogon/SetupComplete scripts
 4. Click Start Normal Install
 5. Find and launch setup.exe interactively (no answer file)
 6. Copy post-install scripts to target drive
 7. Reboot
```

**Automated Install Flow**:
```
 1. Browse for WIM/ESD/ISO image → Parse editions with DISM
 2. Select edition from ComboBox, target disk from ComboBox, boot mode (UEFI/BIOS)
 3. Configure machine name, user account, timezone
 4. Toggle tweaks (privacy, security, performance, UI, bloatware)
 5. Optionally configure domain join
 6. Optionally add FirstLogon/SetupComplete scripts
 7. Preview XML or click Deploy
 8. Format disk with diskpart (UEFI: EFI+MSR+Primary; BIOS: Reserved+Primary)
 9. Apply Win11 bypass registry keys (if enabled)
10. Generate autounattend.xml (3 passes: windowsPE, specialize, oobeSystem)
11. Launch setup.exe /noreboot /unattend:<xml_path>
12. Copy post-install scripts to target drive
13. Reboot into installed Windows
```

**ISO Support**: If user selects an ISO file instead of a WIM, MasterBooter auto-mounts it via PowerShell (`Mount-DiskImage`), finds `sources\install.wim` or `sources\install.esd`, and uses that for DISM operations.

### Key Files for Windows Deploy

| File | Purpose | Lines |
|------|---------|-------|
| `src/deploy.rs` | All deployment logic (XML, diskpart, profiles, scripts, normal/automated) | ~2,400 |
| `src/ui/main.slint` | Deploy page UI (mode selector + Normal page + Automated page with sections) | ~1,000 (within main.slint) |
| `src/main.rs` | Deploy callback wiring (~20 callbacks) | ~600 (within main.rs) |
| `profiles/` | Saved deployment profiles (JSON) | User-created |
| `FirstLogon/` | User-added FirstLogon scripts (copied to target after install) | User-created |
| `SetupComplete/` | User-added SetupComplete scripts (copied to target after install) | User-created |

### Deploy Config Fields (~50 settings)

The `DeployConfig` struct holds all deployment settings, organized into categories:

| Category | Fields | Examples |
|----------|--------|---------|
| Image | wim_path, edition, edition_index | WIM file path, "Windows 11 Pro" |
| Machine | computer_name, timezone, language, boot_mode, disk_id | "DESKTOP-001", "Eastern Standard Time" |
| User | user_name, user_password, display_name, is_admin, autologon | "Admin", true, true |
| OOBE | skip_oobe, skip_eula, skip_network, bypass_win11 | All booleans |
| Privacy (6) | telemetry, location, ads, suggested_apps, bing_search, smartscreen | All disable toggles |
| Security (6) | rdp, uac, defender, firewall, vbs, bitlocker | Mix of enable/disable |
| Performance (3) | fast_startup, high_performance, system_restore | Disable/enable toggles |
| UI (7) | file_extensions, hidden_files, context_menu, search, task_view, widgets, taskbar_align | Show/hide/classic toggles |
| Bloatware (5) | cortana, onedrive, teams, copilot, widgets_service | All disable toggles |
| Domain | join_domain, domain_name, domain_user, domain_pass, workgroup | Enterprise settings (OU removed) |
| Registration | product_key, organization, owner_name | Optional fields |
| Advanced | prevent_device_encryption | BitLocker auto-encrypt prevention |

### Autounattend.xml Structure

The XML is generated from scratch (not template-based) with three passes:

```xml
<?xml version="1.0" encoding="utf-8"?>
<unattend xmlns="urn:schemas-microsoft-com:unattend">
  <!-- Pass 1: windowsPE — disk layout, language, image source -->
  <settings pass="windowsPE">
    <DiskConfiguration>         <!-- UEFI: EFI+MSR+Primary; BIOS: Reserved+Primary -->
    <ImageInstall><OSImage>     <!-- Edition selection by index -->
    <SetupUILanguage>           <!-- Language setting -->
  </settings>

  <!-- Pass 2: specialize — machine identity -->
  <settings pass="specialize">
    <ComputerName>              <!-- Machine name -->
    <TimeZone>                  <!-- Timezone -->
    <RegisteredOrganization>    <!-- Optional -->
  </settings>

  <!-- Pass 3: oobeSystem — user, OOBE skip, all tweaks -->
  <settings pass="oobeSystem">
    <AutoLogon>                 <!-- If enabled -->
    <UserAccounts>              <!-- Local user + optional admin -->
    <OOBE>                      <!-- Skip OOBE, EULA, network -->
    <FirstLogonCommands>        <!-- 27 tweaks as reg/PowerShell commands -->
  </settings>
</unattend>
```

### Profile System

- Profiles saved as JSON in `profiles/` folder next to the EXE
- `DeployConfig` derives `Serialize` + `Deserialize` for direct JSON mapping
- Session-specific fields (wim_path, edition, edition_index) are excluded from profiles
- IT-focused defaults: telemetry disabled, RDP enabled, Defender kept on, bloatware removed
- **ComboBox dropdown**: Saved profiles appear in a dropdown, auto-loads on selection
- **Import button**: Opens file picker to import profiles from any location (copies to profiles/)

### Post-Install Script System

Scripts that run after Windows installation completes:

| Script Type | When It Runs | Target Location |
|-------------|-------------|-----------------|
| **FirstLogon** | After first user login | `C:\Temp\MasterBooter\` (via RunAll.bat) |
| **SetupComplete** | During setup completion (as SYSTEM) | `C:\Windows\Setup\Scripts\` (via SetupComplete.cmd) |

- Scripts managed via Add/Remove buttons in the UI (both Normal and Automated pages)
- File picker supports: `.ps1`, `.bat`, `.cmd`, `.exe`, `.reg`
- Scripts stored in `FirstLogon/` and `SetupComplete/` folders next to the EXE
- `copy_scripts_to_target()` finds the target Windows drive and copies scripts after install
- In Automated mode, RunAll.bat is also added as the final FirstLogonCommand in autounattend.xml

### Reference Implementation

Ported from AMPIPIT's deployment system:
- Location: `C:\AMPIPIT\`
- Key files: `src/cli/automated_install.rs`, `src/services/unattend_template.rs`
- Same Rust + Slint stack, adapted to MasterBooter's simpler architecture (no tokio)

---

## System Prep Architecture

### How System Prep Works

System Prep uses the **tool launcher pattern** (same as Backup/Restore). Instead of reimplementing sysprep functionality, MasterBooter downloads and launches **SysprepPreparator** — a dedicated wizard-based tool that handles the entire sysprep workflow.

**User Flow**:
```
1. Click "System Prep" in sidebar
2. See SysprepPreparator tool card with info box
3. Click the card → tool launcher popup appears
4. Download (if not already downloaded) → extracts ZIP (EXE + DLLs + config + languages)
5. Launch → SysprepPreparator wizard opens
6. Follow wizard: pre-flight checks → cleanup → sysprep execution
```

### Key Files

| File | Purpose |
|------|---------|
| `src/tools.rs` | `SYSPREP_PREPARATOR` constant + `get_tool_by_id()` mapping |
| `src/ui/main.slint` | System Prep page with ToolCard + info box |
| `backup_tools/sysprepprep/` | Downloaded tool folder (SysprepPreparator.exe + DLLs) |

### SysprepPreparator Features (provided by the external tool)

- **6 pre-flight checks**: Setup state, pending operations, drivers, domain, Store apps, Server roles
- **8 cleanup tasks**: Kill processes, delete shadow copies, DISM cleanup, clear update cache, disk cleanup, event logs, recycle bin
- **Sysprep options**: OOBE/Audit mode, Generalize toggle, Shutdown/Reboot/Quit, optional unattend.xml
- **Download**: `https://github.com/CodingWonders/SysprepPreparator/releases`

### Why Tool Launcher Instead of Built-In?

SysprepPreparator already provides a complete, tested sysprep workflow. Reimplementing it would add significant complexity with no additional value. The tool launcher pattern lets us:
- Leverage existing tested software
- Keep MasterBooter's codebase focused
- Get updates automatically when the user re-downloads

See DECISIONS.md ADR-010 for the full rationale.

---

## Auto-Update Architecture

### How Auto-Update Works

MasterBooter checks for updates from GitHub Releases (`Howweird/Masterbooter`) and can self-replace the running EXE.

**Startup Flow** (automatic, background):
```
1. Skip if running in WinPE (no internet assumed)
2. Check masterbooter_version.json — if version changed, refresh PE tool manifests
3. Save current version to masterbooter_version.json
4. Spawn background thread → GET api.github.com/.../releases/latest
5. Compare tag_name (e.g. "v1.2.0") against CARGO_PKG_VERSION
6. If newer: show orange badge in sidebar + green text in status bar
7. If error: silently log to console (don't bother the user)
```

**Manual Check** (via Settings button in sidebar):
- Same as startup but shows errors to the user in status bar

**Download + Replace** (user clicks update badge):
```
1. Download masterbooter.exe asset to temp file (8KB chunks, progress 0-90%)
2. self_replace::self_replace() swaps running EXE (progress 95%)
3. Clean up temp file
4. Show "Restart to update" message (orange text)
```

### Key Files for Auto-Update

| File | Purpose |
|------|---------|
| `src/updater.rs` | GitHub API, version compare, download, self-replace (~500 lines) |
| `src/main.rs` | Startup check, Settings callback, download/dismiss callbacks |
| `src/ui/main.slint` | 9 update properties, 3 callbacks, sidebar badge, status bar |
| `masterbooter_version.json` | Tracks last-run version (next to EXE, gitignored) |

### PE Tool Manifest Refresh

When the EXE is updated to a new version:
1. `check_version_change()` detects version mismatch in `masterbooter_version.json`
2. `refresh_pe_tool_manifests()` calls `create_default_pe_tools()` which overwrites all `tool.toml` files
3. Downloaded tool binaries are NOT deleted — only manifests are refreshed
4. This ensures new PE tools or updated download URLs from the new version take effect

### Dependencies

- `self-replace` crate (v1.5) — handles Windows EXE self-replacement
- `reqwest` blocking client — HTTP requests to GitHub API
- `serde_json` — parse GitHub API response (via `response.text()` + `from_str()`)

---

## Next Steps

The core functionality is complete. Remaining work:
1. ~~**GitHub setup**~~ — **Complete** (Howweird/Masterbooter repo with releases)
2. **Code signing** — sign EXE to avoid SmartScreen warnings
3. **Testing** — test on various hardware configurations
4. **Polish** — error handling improvements, final documentation

Remember: It's normal to make mistakes and get errors. That's how everyone learns!
