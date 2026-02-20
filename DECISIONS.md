# Architecture Decision Records

This file tracks important technical decisions for the MasterBooter project.

---

## ADR-001: Programming Language Selection

**Date**: 2026-01-16
**Status**: ~~Accepted~~ **SUPERSEDED by ADR-005**
**Decision**: ~~C# with WPF for GUI~~

### Context
MasterBooter needs to be:
- A single portable executable
- Work in WinPE for some features
- Maintainable by a developer learning to program
- Have a modern, professional GUI

### Options Considered

| Option | Learning Curve | WinPE Support | Windows Integration |
|--------|---------------|---------------|---------------------|
| Rust + Slint | Very Hard | Proven | Good |
| C# + WPF | Easy | Needs AOT | Excellent |
| C# + PowerShell | Easy | Native | Mixed |

### Original Decision (Superseded)
**C# with WPF** was initially chosen for beginner-friendliness.

### Why This Was Changed
See **ADR-005** - WPF does not work in WinPE, which is a core requirement.

---

## ADR-002: GUI Framework Selection

**Date**: 2026-01-16
**Status**: ~~Accepted~~ **SUPERSEDED by ADR-005**
**Decision**: ~~WPF (Windows Presentation Foundation)~~

### Why This Was Changed
See **ADR-005** - WPF requires .NET runtime and desktop composition, neither available in WinPE.

---

## ADR-003: Project Structure

**Date**: 2026-01-16
**Status**: ~~Accepted~~ **SUPERSEDED by ADR-005**

### Why This Was Changed
See **ADR-005** - Switching to Rust project structure.

---

## ADR-004: Build Target Configuration

**Date**: 2026-01-16
**Status**: ~~Accepted~~ **SUPERSEDED by ADR-005**

### Why This Was Changed
See **ADR-005** - Rust uses different build tooling (cargo instead of dotnet).

---

## ADR-005: Switch to Rust + Slint

**Date**: 2026-01-17
**Status**: Accepted
**Decision**: Rust programming language with Slint UI framework

### Context
After building initial prototype in C#/WPF, discovered critical issue:
- **WPF does not work in Windows PE/RE environments**
- WPF requires .NET runtime and desktop composition (DWM)
- Neither is available in WinPE

MasterBooter's Windows Deploy feature **must** work in WinPE to install Windows.

### Why Rust + Slint

| Requirement | Rust + Slint |
|-------------|--------------|
| Single portable EXE | Yes - native binary, zero dependencies |
| Works in WinPE/RE | Yes - proven by AMPIPIT and GhostWin |
| Works in live Windows | Yes |
| Software rendering | Yes - no GPU required |
| Modern GUI | Yes - Slint is modern and customizable |

### Proof of Concept
The reference tools **AMPIPIT** and **GhostWin** already use Rust + Slint and successfully:
- Run in WinPE with software rendering
- Apply WIM images to disk
- Execute Windows setup with autounattend.xml
- Inject drivers into images

### Project Structure (New)
```
MasterBooter/
├── src/
│   ├── main.rs              # Application entry point
│   ├── ui/                  # Slint UI files (.slint)
│   ├── backup/              # Backup/restore module
│   ├── deploy/              # Windows deployment module
│   ├── winpe/               # WinPE builder module
│   └── sysprep/             # System prep module
├── Cargo.toml               # Rust dependencies
├── build.rs                 # Build configuration
├── tools/                   # Bundled third-party tools
└── docs/                    # Documentation
```

### Build Commands
```bash
# Development build (debug, runs on current platform)
cargo build

# Release build (optimized, target x64)
cargo build --release --target x86_64-pc-windows-msvc

# The output is a single EXE:
# target/x86_64-pc-windows-msvc/release/masterbooter.exe
```

### Consequences
- Steeper learning curve than C# (mitigated by Claude Code assistance)
- Single EXE with no runtime dependencies
- Works everywhere: live Windows, WinPE, WinRE
- Smaller binary size than .NET self-contained
- Reference code available in AMPIPIT and GhostWin

### What We Learned from C# Prototype
The C#/WPF prototype was valuable for:
- Defining the UI layout (sidebar + content area)
- Implementing bundled tool management (download, launch, update)
- Settings persistence design
- Understanding the feature requirements

These patterns will be ported to Rust.

---

## ADR-006: WinPE/WinRE Building Approach

**Date**: 2026-01-17
**Updated**: 2026-02-15
**Status**: Accepted
**Decision**: ADK copype + DISM customization with tool.toml manifests

### Context
MasterBooter needs to create customized WinPE/WinRE bootable ISOs. After successfully creating a basic bootable ISO, we discovered it boots to Windows Setup (the default boot.wim content) rather than a customized PE environment.

### What We Learned from Reference Tools

#### **AMPIPIT Approach:**
- **Minimal WIM** - Only launcher scripts go in the WIM, main EXE stays on USB
- Uses DISM to mount/modify boot.wim
- Injects drivers with `DISM /Add-Driver /Recurse`
- Uses `winpeshl.ini` to launch custom shell
- Creates BCD stores with `bcdedit` for ramdisk boot
- Marker file discovery pattern (searches drives for `.marker` file)

#### **Windows Setup Helper Approach:**
- Uses **AutoIt3** as the shell runtime (lightweight interpreter)
- Runs `wpeinit.exe` for network initialization
- Bundles: 7-Zip, CrystalDiskInfo, Disk2vhd, Explorer++, etc.
- Two-tier shell: `winpeshl.ini` → AutoIt3 → GUI
- Dynamic script discovery from folders
- Tree view for selecting tools/scripts to run

#### **RescueMaker Approach:**
- Uses **WinXShell** as the desktop shell (looks like Windows desktop)
- Downloads tools at build time (not pre-bundled)
- Tools: WinXShell, DISM++, Explorer++, ChkDskGUI, CrystalDiskInfo, Windows Login Unlocker
- No network driver handling - focuses on local/offline use
- Creates desktop shortcuts for tools
- Uses local WinRE.wim (no ADK required)

### Key Steps to Customize a PE

1. **Get source WIM**: Either from Windows ISO (`boot.wim`) or local WinRE
2. **Mount WIM**: `DISM /Mount-Wim /WimFile:boot.wim /Index:1 /MountDir:Mount`
3. **Modify the mounted image**:
   - Add/replace shell (winpeshl.ini)
   - Inject drivers (`DISM /Add-Driver`)
   - Copy tools to \Program Files\
   - Create desktop shortcuts
   - Modify registry if needed
4. **Commit changes**: `DISM /Unmount-Wim /MountDir:Mount /Commit`
5. **Build ISO structure**: boot.wim, boot.sdi, BCD, bootmgr, EFI files
6. **Create ISO**: `oscdimg` with BIOS/UEFI boot data

### Shell Options

| Shell | Description | Pros | Cons |
|-------|-------------|------|------|
| **WinXShell** | Desktop replacement, looks like Windows | Familiar UI, file manager built-in | Larger size, more complex |
| **Explorer++** | Lightweight file manager | Small, fast | No Start menu |
| **cmd.exe** | Command prompt | Always available, tiny | Text-only, not user-friendly |
| **Custom Slint app** | Our own MasterBooter GUI | Tailored to our needs | Requires maintaining |

### Build Options to Expose in UI

**Current options (existing):**
- Include Drivers: yes/no
- Include MasterBooter Tools: yes/no
- Include Network Support: yes/no

**Proposed additional options:**
- **Shell Selection**: WinXShell / Explorer++ / Command Prompt / MasterBooter GUI
- **Source Type**: Windows ISO / Local WinRE / ADK WinPE
- **Tool Selection**: Checkboxes for each tool (DISM++, CrystalDiskInfo, etc.)
- **Driver Injection**: Browse to driver folder / Use inbox drivers only

### Decisions Made

1. **MasterBooter EXE is embedded in the PE** (copied to X:\Tools\)
2. **WinXShell is the default shell** (user-selectable, Explorer++ as alternative)
3. **Tools downloaded at build time** (like RescueMaker), cached in pe_tools/
4. **Full WiFi driver injection** + WLAN service injection for wireless support

### Build Pipeline (Implemented)
```
 1. Detect ADK → copype creates WinPE base structure
 2. Mount boot.wim with DISM
 3. Install ADK packages (PowerShell, WMI, .NET, etc.)
 4. Apply PE fixes (DPI, WallpaperHost, fonts, etc.)
 5. Inject WiFi drivers (DISM /Add-Driver) + WLAN service files
 6. Inject WiFi/WLAN service infrastructure from local Windows
 7. Inject PE tools (WinXShell, PENetwork, 7-Zip, etc.)
 8. Configure shell, create launcher script, create shortcuts
 9. Unmount and commit WIM
10. Disable driver signature enforcement in BIOS + UEFI BCD
11. Create ISO with oscdimg (BIOS + UEFI dual-boot)
12. Verify ISO (5-point check)
```

### Consequences
- Requires Windows ADK installed (with WinPE add-on)
- Full WiFi support requires building from a machine with Windows installed (for WLAN DLLs)
- ISO output only (user burns with Rufus/Ventoy)
- PE tools stored separately from EXE (GitHub releases for distribution)

---

## ADR-007: Tool Organization (Backup vs PE)

**Date**: 2026-01-17
**Status**: Accepted
**Decision**: Separate `backup_tools/` and `pe_tools/` folders with different download strategies

### Context
MasterBooter needs to manage two completely different types of tools:

1. **Backup/Restore tools** - Run in LIVE Windows for profile backup, disk imaging, etc.
2. **PE Builder tools** - Get bundled INTO the WinPE image during build

Originally all tools were in a single `tools/` folder, causing confusion about which tools belong where.

### Decision

#### Folder Structure
```
MasterBooter/
├── backup_tools/           # Tools for Backup/Restore page
│   ├── fabs/               # Fab's AutoBackup
│   ├── profwiz/            # User Profile Wizard
│   ├── transwiz/           # Transwiz Profile Transfer
│   ├── disk2vhd/           # Disk2VHD
│   └── hddrawcopy/         # HDD Raw Copy Tool
│
└── pe_tools/               # Tools bundled INTO WinPE images
    ├── shell/              # Desktop shells
    │   ├── winxshell/
    │   └── explorer++/
    ├── network/            # Network tools
    │   └── penetwork/
    ├── disk/               # Disk utilities
    │   ├── crystaldiskinfo/
    │   ├── chkdskgui/
    │   └── dismpp/
    ├── security/           # Security tools
    │   └── wlu/            # Windows Login Unlocker
    └── utilities/          # General utilities
        └── 7zip/
```

#### Download Strategies

| Folder | When Downloaded | Why |
|--------|-----------------|-----|
| `backup_tools/` | On first use | Tools run locally; user may not need all of them |
| `pe_tools/` | At PE build time | Tools get copied into the WIM; download only when building |

#### Tool Discovery

- **Backup tools**: Hard-coded in Rust (known set of tools with specific download handling)
- **PE tools**: Folder-based discovery with `tool.toml` manifests (easy to add new tools)

### Why This Pattern?

Following reference tools:
- **AMPIPIT**: Separates Tools/, PEAutoRun/, SetupComplete/, FirstLogon/
- **RescueMaker**: Downloads tools at build time (not pre-bundled)

### Tool Manifest Format (PE Tools)

Each PE tool has a `tool.toml` file:

```toml
[tool]
name = "WinXShell"
description = "Desktop shell with Start menu and taskbar"
category = "shell"          # shell, network, disk, security, utilities
exe = "WinXShell_x64.exe"   # Main executable
is_shell = true             # Is this a shell replacement?
create_shortcut = false     # Create desktop shortcut?
enabled_by_default = true   # Enabled by default?
auto_launch = false         # Launch at PE startup?
download_url = "https://..."  # Where to download if missing
```

### Consequences

- Clear separation of tool types
- Users can easily add PE tools by creating a folder + tool.toml
- PE tools downloaded only when needed (saves bandwidth/storage)
- Backup tools downloaded on first use (user may never need some)
- Code is organized with clear responsibilities

---

## ADR-008: WiFi/WLAN Support in WinPE

**Date**: 2026-02-15
**Status**: Accepted
**Decision**: Manual WLAN service injection from local Windows

### Context
PENetwork could not see WiFi adapters in our WinPE builds. Investigation revealed:
- `WinPE-WiFi-Package` does NOT exist as a standalone ADK .cab file
- It only exists pre-baked inside WinRE.wim (Microsoft docs confirm this)
- Without the WLAN service, WiFi adapters are invisible even with correct drivers

### Options Considered

| Option | Complexity | Reliability |
|--------|-----------|-------------|
| Use WinRE.wim as base | Low | High (WiFi built-in) |
| Manual WLAN file injection | Medium | Good (tested approach) |
| Require user to provide files | Low | Poor (bad UX) |

### Decision
Two-part WiFi support:

**Part 1: WiFi adapter drivers** via `extract_wifi_drivers_from_local_windows()`:
- Extracts WiFi .inf + .sys files from C:\Windows\INF (like PhoenixPE does from Install.wim)
- Covers 6 manufacturers: Intel, Broadcom, Realtek, Qualcomm, Ralink/MediaTek, Marvell
- ~30 INF files covering most laptop WiFi chipsets
- No external download needed — drivers come from the build machine's Windows
- Injected via DISM /Add-Driver + copied to PE for drvload fallback

**Part 2: WLAN service infrastructure** via `inject_wifi_support()`:
1. Copy ~15 DLLs from C:\Windows\System32 (wlansvc.dll, wlanapi.dll, etc.)
2. Copy NativeWiFi kernel drivers (nwifi.sys, vwififlt.sys, etc.)
3. Copy L2Schema XML files (required for WLAN profile parsing)
4. Add WlanSvc + NativeWifiP registry entries via reg load/add/unload
5. Register WiFi drivers via DISM /Add-Driver
6. Launcher script starts wlansvc at boot

### Previous Approach (Replaced)
Originally bundled WiFi_Drivers.7z (93 MB, extracted from Medicat SDI).
Changed to local extraction because:
- 93 MB was too large for GitHub release hosting
- PhoenixPE/GhostWin/Setup Helper all avoid bundled driver packs
- Local Windows has the same inbox drivers
- Zero download = faster builds

### Driver Signature Enforcement Bypass (BCD)

WiFi protocol/filter drivers (nwifi.sys, vwififlt.sys, wfplwfs.sys) are file-copied from
install.wim, not DISM-injected. WinPE enforces driver signature verification by default,
so these manually-copied drivers are rejected at boot with "cannot verify digital signature."

**Solution**: Disable driver signature enforcement in both BIOS and UEFI BCD stores using
three bcdedit commands (matching PhoenixPE's `700-BCD.script` BypassDriverSigning approach):

1. `loadoptions DDISABLE_INTEGRITY_CHECKS` — traditional WinPE approach
2. `nointegritychecks on` — modern explicit disable
3. `testsigning on` — allows unsigned/test-signed drivers

All three are applied for maximum compatibility across Windows 10/11 PE versions. This is
done automatically in `disable_driver_signature_enforcement()` in winpe.rs, called from both
`create_bcd_store()` (fallback path) and `build_pe_iso()` Step 4.9 (main build path).

### Why Not WinRE?
WinRE would work but limits flexibility. Users with ADK want the standard copype workflow.
WinRE approach is available as a fallback (source auto-detection skips injection for WinRE).

---

## ADR-009: Reference Program Catalog

**Date**: 2026-02-15
**Status**: Accepted
**Decision**: Maintain a catalog of all reference programs with clear module mappings

### Primary References (Rust + Slint — same stack)
| Program | Location | Modules |
|---------|----------|---------|
| AMPIPIT | `C:\AMPIPIT` | Windows Deploy, System Prep, PE GUI |
| GhostWin | `C:\Users\howar\ClaudeSourceFiles\ghostwin-main` | Windows Deploy, PE Builder, Drivers |

### Secondary References (different languages, valuable patterns)
| Program | Location | Language | Modules |
|---------|----------|----------|---------|
| Windows Setup Helper | `C:\ProgramData\CKTech\windows-setup-helper-master` | AutoIt3 | PE Builder, Tool Launcher |
| Unattend Generator | `C:\unattend-generator-master` | C# | Windows Deploy (XML generation) |
| SysprepPreparator | `C:\sysprepartor` | C# | System Prep |
| PhoenixPE | `C:\Users\howar\ClaudeSourceFiles\PhoenixPE-master` | PEBakery | PE Builder (modular approach) |
| AMPIPIT-NWG | `C:\Users\howar\ClaudeSourceFiles\AMPIPIT-NWG` | Rust + NWG | Alternative GUI framework |
| Medicat | `C:\Users\howar\ClaudeSourceFiles\Medicat` | Mixed | PE environment, WiFi patterns |

### Tool/Binary References (no source, patterns only)
| Program | Location | Purpose |
|---------|----------|---------|
| d7x | `C:\Users\howar\ClaudeSourceFiles\d7x` | Config-driven tool integration |
| tools/ collection | `C:\Users\howar\ClaudeSourceFiles\tools` | 55+ PE/diagnostic utilities |

---

## ADR-010: System Prep via Tool Launcher

**Date**: 2026-02-18
**Status**: Accepted
**Decision**: Use tool launcher pattern for System Prep (download + launch SysprepPreparator)

### Context
MasterBooter needs a System Prep module to prepare Windows installations for image capture. Two approaches were considered:

### Options Considered

| Option | Complexity | Maintenance | Quality |
|--------|-----------|-------------|---------|
| **Built-in sysprep module** | High (2000+ lines) | Must maintain all checks/cleanup | Might miss edge cases |
| **Tool launcher for SysprepPreparator** | Low (~50 lines) | External tool maintained by CodingWonders | Complete, tested wizard |

### Decision
Use the **tool launcher pattern** (same as Backup/Restore tools). SysprepPreparator is downloaded from GitHub and launched as an external tool.

### Why
- SysprepPreparator already provides a complete, tested sysprep workflow
- Includes 6 pre-flight checks, 8 cleanup tasks, and configurable sysprep execution
- Maintained by CodingWonders with active development (v0.7.2, async tasks, auto mode)
- Reimplementing would add ~2,000 lines of code with no additional user value
- Tool launcher pattern is proven (already works for 5 backup tools)
- Users get updates by simply re-downloading

### Implementation
- `SYSPREP_PREPARATOR` constant in `tools.rs` (DownloadType::Zip)
- Downloads from: `https://github.com/CodingWonders/SysprepPreparator/releases`
- ZIP extraction upgraded to handle complete apps (EXE + DLLs + config + language files)
- Reuses existing tool popup system (download, launch, open folder)

### Consequences
- Depends on external tool being available on GitHub
- Cannot deeply integrate sysprep into MasterBooter's own UI flow
- Simple, maintainable, and immediately functional

---

## ADR-011: Auto-Update from GitHub Releases

**Date**: 2026-02-18
**Status**: Accepted
**Decision**: Auto-update via GitHub Releases API + self_replace crate

### Context
MasterBooter is distributed as a single portable EXE via GitHub Releases. Users need a way to check for and install updates without manually downloading.

### Options Considered

| Option | Complexity | UX | Reliability |
|--------|-----------|-----|-------------|
| **Manual download only** | None | Poor (user must check GitHub) | N/A |
| **Auto-update from GitHub** | Medium | Good (badge + one-click update) | Good (GitHub is reliable) |
| **Windows Store** | High | Great (automatic) | Good, but limits distribution |

### Decision
Auto-update from GitHub Releases:

1. **Startup check** (background thread): `GET api.github.com/repos/Howweird/Masterbooter/releases/latest`
2. **Manual check** via Settings button in sidebar
3. **Download + self-replace**: Download `masterbooter.exe` asset, then use `self_replace` crate to swap running EXE
4. **PE tool refresh**: On version change, refresh `tool.toml` manifests from embedded defaults
5. **Skip in WinPE**: No update check when running in PE environment

### Why self_replace?
Windows prevents overwriting a running EXE. The `self_replace` crate handles this by:
- Moving the running EXE aside (rename)
- Copying the new EXE into place
- Scheduling cleanup of the old file

### Implementation
- `src/updater.rs` (~500 lines) — GitHub API, version comparison, download with progress, self-replace
- Semver comparison: parse `tag_name` (e.g. "v1.2.0") into (major, minor, patch) tuples
- Progress callback updates UI sidebar badge (0-100%)
- `masterbooter_version.json` tracks last-run version for PE manifest refresh

### Consequences
- Requires internet access for update check (graceful failure if offline)
- GitHub API rate limit: 60 requests/hour unauthenticated (more than sufficient)
- Binary size increase: ~200 KB from self_replace crate
- Users must restart app after update (not automatic)

---

## Future Decisions Needed

| Topic | When to Decide | Notes |
|-------|----------------|-------|
| ~~Shell selection for PE~~ | ~~Now~~ | Resolved: WinXShell default, user-selectable |
| ~~Tool bundling strategy~~ | ~~Now~~ | Resolved: See ADR-007 |
| ~~Autounattend.xml approach~~ | ~~Now~~ | Resolved: Custom generation from scratch in deploy.rs |
| ~~System Prep approach~~ | ~~Now~~ | Resolved: Tool launcher (ADR-010) |
| ~~Update mechanism~~ | ~~Before release~~ | Resolved: Auto-update from GitHub releases (ADR-011) |
| Disk imaging library | Future | Options: libvhd, Windows APIs |
| Code signing | Before release | Self-signed vs purchased certificate (SmartScreen) |
| Logging framework | Phase 2 | println! currently, structured logging later |
