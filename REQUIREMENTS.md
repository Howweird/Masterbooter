# MasterBooter Requirements

This file tracks feature requests, requirements, and priorities for the MasterBooter project.

**Last Updated**: 2026-02-18
**Version**: 0.1.1
**Status**: Active Development — WinPE Builder + Windows Deploy + System Prep functional
**Technology**: Rust / Slint (changed 2026-01-17 - see DECISIONS.md ADR-005)

---

## Feature Categories

### 1. WinPE Environment
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| PE-001 | Boot into custom WinPE with GUI | High | AMPIPIT, GhostWin | **Complete** |
| PE-002 | USB drive detection via marker files | High | AMPIPIT | Planned |
| PE-003 | Software rendering (no GPU required) | High | AMPIPIT, GhostWin | **Complete** (Slint renderer-software) |
| PE-004 | Tool/script auto-discovery from folders | Medium | Setup Helper | **Complete** (tool.toml manifests) |
| PE-005 | Network initialization (wpeinit, WLAN) | Medium | AMPIPIT | **Complete** (WiFi injection + wlansvc) |
| PE-006 | Remote access (VNC integration) | Low | Setup Helper, GhostWin | Planned |

### 1a. WinPE Builder - ADK Packages
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| PK-001 | **Windows Management Instrumentation (WMI)** | High | GhostWin, Setup Helper | **Complete** |
| PK-002 | **.NET Framework support** | High | GhostWin, Setup Helper | **Complete** |
| PK-003 | **Windows Script Host** | High | GhostWin, Setup Helper | **Complete** |
| PK-004 | **PowerShell support** | High | Setup Helper | **Complete** |
| PK-005 | **DISM PowerShell cmdlets** | Medium | Setup Helper | **Complete** |
| PK-006 | **Storage WMI (NVMe support)** | High | GhostWin | **Complete** |
| PK-007 | **Enhanced Storage** | Medium | GhostWin | **Complete** |
| PK-008 | **BitLocker support (Secure Startup)** | Medium | Setup Helper | **Complete** |
| PK-009 | **802.1X authentication (Dot3Svc)** | Low | - | **Complete** |
| PK-010 | **HTML Applications (HTA)** | Low | Setup Helper | **Complete** |
| PK-011 | **Dependency resolution for packages** | High | - | **Complete** |
| PK-012 | **UI toggles for package selection** | High | AMPIPIT | **Complete** |

### 1b. WinPE Builder - PE Fixes
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| PF-001 | **DPI scaling fix for high-DPI displays** | High | GhostWin, Setup Helper | **Complete** |
| PF-002 | **WallpaperHost.exe removal** | High | GhostWin | **Complete** |
| PF-003 | **Segoe UI italic font fix** | Medium | Setup Helper | **Complete** |
| PF-004 | ~~User profile folders creation~~ | Medium | Setup Helper | **Removed** (handled by launcher script) |
| PF-005 | ~~TEMP/TMP folder configuration~~ | Medium | - | **Removed** (handled by launcher script) |
| PF-006 | ~~File associations (.txt, .log, .xml)~~ | Low | Setup Helper | **Removed** (was broken — .reg file was never imported) |
| PF-007 | **Disable crash/debug dialogs** | Low | GhostWin | **Complete** |
| PF-008 | **Enable long path support** | Low | - | **Complete** |
| PF-009 | **UI toggles for fix selection** | High | AMPIPIT | **Complete** |

### 2. Windows Installation
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| WI-001 | Generate autounattend.xml programmatically | High | Unattend Generator | **Complete** |
| WI-002 | Normal install mode (standard setup) | High | Setup Helper, AMPIPIT | **Complete** (mode selector + interactive setup.exe) |
| WI-003 | Automated install mode (unattended) | High | Setup Helper, AMPIPIT | **Complete** (autounattend.xml + all tweaks) |
| WI-009 | ISO image support (mount + detect WIM) | High | AMPIPIT | **Complete** (auto-mount ISO, find install.wim/esd) |
| WI-010 | Normal/Automated mode selector | Medium | AMPIPIT | **Complete** (card-based UI selection) |
| WI-004 | WIM edition selection | High | AMPIPIT | **Complete** |
| WI-005 | Disk partitioning configuration | Medium | Unattend Generator | **Complete** |
| WI-006 | User account creation | Medium | Unattend Generator | **Complete** |
| WI-007 | Regional/locale settings | Medium | Unattend Generator | **Complete** |
| WI-008 | Product key handling | Low | Unattend Generator | **Complete** |
| WI-011 | Product key backup (detect + save/copy) | Medium | — | **Complete** |
| WI-012 | Multi-key management (store/select/delete by hostname) | Medium | — | **Complete** |

### 3. Driver Management
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| DM-001 | Driver injection into WIM | High | AMPIPIT, GhostWin | **Complete** (DISM /Add-Driver + drvload fallback) |
| DM-002 | Intel VMD/RST driver support | High | GhostWin | Planned |
| DM-003 | NVMe driver injection | High | GhostWin | **Complete** (via DISM /Add-Driver /Recurse) |
| DM-004 | Driver export from current system | Medium | SysprepPreparator, DISM++ | Planned |
| DM-005 | Driver backup before sysprep | Medium | SysprepPreparator | Planned |
| DM-006 | **WiFi drivers (6 vendors)** | High | PhoenixPE pattern | **Complete** (extracted from ISO's install.wim via 7-Zip) |
| DM-007 | **WLAN service injection for WinPE** | High | Community PE builders | **Complete** (inject_wifi_support() from ISO source) |
| DM-008 | **Custom branding wallpaper** | Medium | PhoenixPE | **Complete** (embedded in EXE, injected into WIM + registry) |

### 4. Post-Installation Scripts
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| PS-001 | ~~SetupComplete phase scripts~~ | High | AMPIPIT | **Removed** (FirstLogonCommands only — SetupComplete unreliable in WinPE) |
| PS-002 | FirstLogon phase scripts | High | AMPIPIT, Setup Helper | **Complete** (via autounattend.xml FirstLogonCommands + RunAll.bat) |
| PS-003 | Script tagging system ([ONLINE], [ADMIN], etc.) | Medium | AMPIPIT | Planned |
| PS-004 | Script execution ordering | Medium | Setup Helper | **Complete** (scripts run in alphabetical order via RunAll.bat/SetupComplete.cmd) |
| PS-005 | Multiple script types (.ps1, .bat, .exe, .reg) | Medium | Setup Helper | **Complete** (file picker supports .ps1, .bat, .cmd, .exe, .reg) |
| PS-006 | Script add/remove UI | Medium | AMPIPIT | **Complete** (Add/Remove buttons in both Normal and Automated pages) |

### 5. System Preparation (Sysprep)
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| SP-001 | Pre-sysprep compatibility checks | High | SysprepPreparator | **Complete** (via SysprepPreparator tool) |
| SP-002 | System cleanup before capture | High | SysprepPreparator, DISM++ | **Complete** (via SysprepPreparator tool) |
| SP-003 | Pending operations detection | Medium | SysprepPreparator | **Complete** (via SysprepPreparator tool) |
| SP-004 | Third-party app detection | Medium | SysprepPreparator | **Complete** (via SysprepPreparator tool) |
| SP-005 | Sysprep execution with options | Medium | SysprepPreparator | **Complete** (via SysprepPreparator tool) |
| SP-006 | SysprepPreparator download & launch | High | SysprepPreparator | **Complete** (tool launcher pattern) |

**Approach**: System Prep uses the **tool launcher pattern** (like Backup/Restore). SysprepPreparator is downloaded from GitHub and launched — it provides a complete wizard-based sysprep workflow including pre-flight checks, system cleanup, and sysprep execution. See DECISIONS.md ADR-010.

### 5a. System Prep Tools
| ID | Requirement | Priority | Status |
|----|-------------|----------|--------|
| ST-001 | **SysprepPreparator integration** | High | **Complete** |
| ST-002 | **Download from GitHub releases** | High | **Complete** |
| ST-003 | **Full ZIP extraction (EXE + DLLs + config)** | High | **Complete** |
| ST-004 | **Tool launcher popup (download/launch/folder)** | High | **Complete** |

**Download Source:**
- SysprepPreparator: `https://github.com/CodingWonders/SysprepPreparator/releases` (ZIP with EXE + DLLs)

### 6. Image Management
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| IM-001 | WIM mount/unmount operations | High | DISM++ | Planned |
| IM-002 | WIM capture and apply | High | DISM++ | Planned |
| IM-003 | ISO building with oscdimg | Medium | AMPIPIT, GhostWin | Planned |
| IM-004 | ESD to WIM conversion | Low | DISM++ | Planned |
| IM-005 | **VHD creation from physical disk** | Low | Disk2vhd | **Complete** |

### 7. Backup & Migration
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| BM-001 | **User profile backup** | High | Fab's AutoBackup | **Complete** (via tool) |
| BM-002 | **Application settings backup** | Medium | Fab's AutoBackup | **Complete** (via tool) |
| BM-003 | **Profile restore to new system** | High | Fab's AutoBackup | **Complete** (via tool) |
| BM-004 | **Full disk imaging (VHD/VHDX)** | High | Disk2vhd | **Complete** (via tool) |
| BM-005 | **Raw HDD copy (sector-by-sector)** | High | HDD Raw Copy Tool | **Complete** (via tool) |
| BM-006 | Portable programs backup | Medium | - | Planned |
| BM-007 | **Browser data migration** | Low | Fab's AutoBackup | **Complete** (via tool) |
| BM-008 | **Email/Outlook migration** | Low | Fab's AutoBackup | **Complete** (via tool) |

### 7a. Bundled Tools System (Backup/Restore — live Windows)
| ID | Requirement | Priority | Status |
|----|-------------|----------|--------|
| BT-001 | **Download tools from official sources** | High | **Complete** |
| BT-002 | **Fab's AutoBackup integration** | High | **Complete** |
| BT-003 | **ProfWiz (User Profile Wizard) integration** | High | **Complete** |
| BT-004 | **Transwiz integration** | High | **Complete** |
| BT-005 | **Disk2VHD integration** | High | **Complete** |
| BT-006 | **HDD Raw Copy Tool integration** | High | **Complete** |
| BT-007 | **Preserve config/license on update** | High | **Complete** |
| BT-008 | **Tool launcher dialog** | Medium | **Complete** |
| BT-009 | **Open tool folder for license files** | Medium | **Complete** |

**Download Sources:**
- Fab's AutoBackup: `https://download.fpnet.fr/trial/AutoBackup7Pro.exe` (self-extracting)
- ProfWiz: `https://www.forensit.com/Downloads/Profwiz.msi`
- Transwiz: `https://www.forensit.com/Downloads/Transwiz.msi`
- Disk2VHD: `https://download.sysinternals.com/files/Disk2vhd.zip`
- HDD Raw Copy Tool: `https://hddguru.com/software/HDD-Raw-Copy-Tool/HDDRawCopy1.20Portable.exe`

### 7b. PE Tools System (bundled INTO WinPE image)
| ID | Requirement | Priority | Status |
|----|-------------|----------|--------|
| PT-001 | **tool.toml manifest discovery** | High | **Complete** |
| PT-002 | **Download from manufacturer + GitHub fallback** | High | **Complete** |
| PT-003 | **Download All button (only checked tools)** | High | **Complete** |
| PT-004 | **Green/orange status dots per tool** | Medium | **Complete** |
| PT-005 | **Detect button animation** | Medium | **Complete** |
| PT-006 | **Build feedback (injected/skipped counts)** | Medium | **Complete** |
| PT-007 | **Archive flattening (nested subfolder fix)** | Medium | **Complete** |
| PT-008 | **GitHub staging copy for release uploads** | Medium | **Complete** |
| PT-009 | **PE tool hover descriptions in bottom bar** | Low | **Complete** |

**12 PE Tools (all enabled by default, manifests auto-refreshed on EXE update):**

| Tool | Category | Source | Fallback |
|------|----------|--------|----------|
| WinXShell | shell | GitHub (primary) | — |
| Explorer++ | shell | derceg/GitHub | GitHub mirror |
| File Explorer | shell | pcassistsoftware | GitHub mirror |
| PENetwork | network | GitHub (primary) | — |
| Web Browser | network | pcassistsoftware | GitHub mirror |
| CrystalDiskInfo | disk | SourceForge | GitHub mirror |
| Disk Check | disk | pcassistsoftware | GitHub mirror |
| DISM Tool | system | pcassistsoftware | GitHub mirror |
| 7-Zip | utilities | GitHub (primary) | — |
| Autoruns | utilities | Microsoft Sysinternals | GitHub mirror |
| Event Viewer | utilities | pcassistsoftware | GitHub mirror |
| Installed Software | utilities | pcassistsoftware | GitHub mirror |

**GitHub fallback mirror**: `Howweird/MasterBooter-Tools` releases (v1.0)
**.NET 4.8 tools** (6 from pcassistsoftware): require `WinPE-NetFx` ADK package (enabled by default)

### 8. Configuration & Profiles
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| CP-001 | Save/load deployment profiles | High | AMPIPIT | **Complete** (JSON profiles in profiles/ folder) |
| CP-002 | Profile export/import (portable) | Medium | AMPIPIT | **Complete** (JSON files, Import button with file picker, auto-copies to profiles/) |
| CP-003 | TOML-based configuration | Medium | AMPIPIT, GhostWin | Deferred (using JSON for deploy profiles) |
| CP-004 | Default profile templates | Low | - | **Complete** (DeployConfig::default() with IT-focused defaults) |
| CP-005 | **Persistent settings across sessions** | High | - | **Complete** |
| CP-006 | **Window positions remembered** | Medium | - | **Complete** |
| CP-007 | **Tool preferences saved (bundled vs local)** | Medium | - | **Complete** |

### 9. User Interface
| ID | Requirement | Priority | Source Tool | Status |
|----|-------------|----------|-------------|--------|
| UI-001 | WinPE-compatible GUI | High | AMPIPIT, GhostWin | **Complete** (Slint software rendering) |
| UI-002 | Dark theme (ocean blue) | Medium | AMPIPIT | **Complete** |
| UI-003 | Sidebar tool/script selection | Medium | AMPIPIT | **Complete** |
| UI-004 | Progress/status display | Medium | All | **Complete** (PE build progress bar) |
| UI-005 | CLI interface for automation | Medium | All | Planned |
| UI-006 | App icon (EXE + window) | Medium | — | **Complete** (winres ICO + Slint PNG) |

### 10. Auto-Update
| ID | Requirement | Priority | Status |
|----|-------------|----------|--------|
| AU-001 | **Check GitHub releases on startup** | High | **Complete** (background thread, non-blocking) |
| AU-002 | **Manual update check via Settings** | High | **Complete** (button in sidebar) |
| AU-003 | **Download and self-replace EXE** | High | **Complete** (self_replace crate, progress bar) |
| AU-004 | **Sidebar update badge** | Medium | **Complete** (orange badge with version + size) |
| AU-005 | **Status bar update indicator** | Medium | **Complete** (green/orange update text) |
| AU-006 | **PE tool manifest refresh on update** | Medium | **Complete** (auto-refresh tool.toml files after version change) |
| AU-007 | **Version tracking file** | Medium | **Complete** (masterbooter_version.json) |
| AU-008 | **Skip update check in WinPE** | Low | **Complete** (no internet assumed) |

---

## User Stories

### US-001: Technician deploys Windows to new PC
**As a** technician
**I want to** boot a PC from USB and run automated Windows installation
**So that** I can deploy a standardized Windows image with minimal interaction

**Acceptance Criteria:**
- [x] Boot into WinPE from USB
- [x] Select Windows edition from available WIMs
- [x] Configure basic settings (username, computer name, timezone)
- [x] Installation runs unattended
- [x] Post-install scripts execute automatically (via FirstLogonCommands)

### US-002: Prepare system for imaging
**As a** technician
**I want to** prepare a reference PC for image capture
**So that** I can create a clean, generalized system image

**Acceptance Criteria:**
- [x] Run pre-sysprep compatibility checks (via SysprepPreparator tool)
- [x] Clean up temporary files and caches (via SysprepPreparator tool)
- [ ] Export current drivers
- [x] Run sysprep with OOBE/generalize (via SysprepPreparator tool)

### US-003: Migrate user data to new PC
**As a** technician
**I want to** backup and restore user profiles between PCs
**So that** users don't lose their data during hardware refresh

**Acceptance Criteria:**
- [ ] Select user profiles to backup
- [ ] Capture documents, settings, browser data
- [ ] Restore to new PC (same or different username)

---

## Open Questions

| ID | Question | Context | Resolution |
|----|----------|---------|------------|
| Q-001 | Should we support both Rust and AutoIt, or Rust-only? | AMPIPIT is Rust, Setup Helper is AutoIt | **Resolved**: Rust-only |
| Q-002 | How to handle licensing for Fab's AutoBackup features? | Commercial tool | TBD - may need to reimplement |
| Q-003 | Include VPN integration (Tailscale/NetBird)? | AMPIPIT has this | TBD |
| Q-004 | Support for ARM64 Windows? | Growing market | TBD |
| Q-005 | Single executable or modular components? | Deployment simplicity vs flexibility | **Resolved**: Single portable EXE (~12 MB) |
| Q-006 | How to handle updates? | Auto-update vs manual download | **Resolved**: Auto-update from GitHub releases (self_replace crate) |

---

## Priority Legend

- **High**: Core functionality, must have for MVP
- **Medium**: Important features, include if time permits
- **Low**: Nice to have, future versions

## Status Legend

- **Planned**: Identified but not started
- **In Progress**: Currently being implemented
- **Blocked**: Waiting on decision or dependency
- **Complete**: Implemented and tested
- **Deferred**: Moved to future version

---

## Change Log

| Date | Change | Author |
|------|--------|--------|
| 2026-01-15 | Initial requirements document created | Claude Code |
| 2026-01-17 | Added settings persistence requirements (CP-005, CP-006, CP-007) - Complete | Claude Code |
| 2026-01-17 | Added Bundled Tools System section (BT-001 to BT-008) - All Complete | Claude Code |
| 2026-01-17 | Disk2VHD integration complete (IM-005) | Claude Code |
| 2026-01-17 | HDD Raw Copy Tool integration complete (BM-005, BT-006) | Claude Code |
| 2026-01-17 | **Technology switch: C#/WPF → Rust/Slint** (WPF incompatible with WinPE) | Claude Code |
| 2026-01-17 | C# prototype archived, starting fresh with Rust | Claude Code |
| 2026-01-17 | **Backup/Restore section complete** - All 5 tools working (Fab's, ProfWiz, Transwiz, Disk2VHD, HDD Raw Copy) | Claude Code |
| 2026-01-17 | Added 7-Zip support for MSI/CAB/installer extraction | Claude Code |
| 2026-01-17 | Fixed download progress display in UI popup | Claude Code |
| 2026-01-19 | **WinPE Builder - ADK Packages module complete** (PK-001 to PK-012) | Claude Code |
| 2026-01-19 | **WinPE Builder - PE Fixes module complete** (PF-001 to PF-009) | Claude Code |
| 2026-01-19 | Added adk_packages.rs module with 16 optional components | Claude Code |
| 2026-01-19 | Added pe_fixes.rs module with 8 fixes for WinPE quirks | Claude Code |
| 2026-01-19 | Created Advanced Options UI section (AMPIPIT-style toggles) | Claude Code |
| 2026-02-15 | **WiFi drivers** - Extract from local Windows INF (PhoenixPE pattern, replaces 93 MB bundled pack) | Claude Code |
| 2026-02-15 | **WiFi/WLAN injection** - Manual WLAN service injection for WinPE (WinPE-WiFi-Package doesn't exist as ADK .cab) | Claude Code |
| 2026-02-15 | **WIM mount safety** - WimMountGuard RAII pattern, force-unmount stale mounts | Claude Code |
| 2026-02-15 | **ISO verification** - 5-point post-build check (size, ISO9660, El Torito, critical files) | Claude Code |
| 2026-02-15 | **PE build robustness** - Pre-flight validation, BCD fallback, boot file fallback, error cleanup | Claude Code |
| 2026-02-15 | **Driver injection** - DISM /Add-Driver + drvload fallback at boot, copy to PE filesystem | Claude Code |
| 2026-02-15 | **PE tested** - Built ISO boots successfully, tools load, shortcuts work | Claude Code |
| 2026-02-15 | **Project audit** - Full review of all reference programs, documentation, Claude Code features | Claude Code |
| 2026-01-19 | Connected UI toggles to Rust callbacks for build config | Claude Code |
| 2026-02-17 | **PE Tools overhaul** — Removed ChkDsk GUI + Windows Login Unlocker (no legal source) | Claude Code |
| 2026-02-17 | **Added 6 PE tools from pcassistsoftware.co.uk** (DISM Tool, Disk Check, Web Browser, Event Viewer, Installed Software, File Explorer) | Claude Code |
| 2026-02-17 | **Switched Explorer++ and Autoruns to manufacturer download URLs** (derceg/GitHub and Sysinternals) | Claude Code |
| 2026-02-17 | **GitHub fallback URLs** — All tools with manufacturer URLs get automatic fallback to Howweird/MasterBooter-Tools | Claude Code |
| 2026-02-17 | **Fixed Download All button** — Now reads UI checkbox states, not pe_tools_config.json | Claude Code |
| 2026-02-17 | **Detect button animation** — Shows "Detecting..." and greys out while running | Claude Code |
| 2026-02-17 | **Status dots** — Green (downloaded) / Orange (missing) dot per tool checkbox | Claude Code |
| 2026-02-17 | **Build feedback** — Progress bar shows injection counts and download failures | Claude Code |
| 2026-02-17 | **Removed shell selection buttons** — WinXShell is now always the default shell | Claude Code |
| 2026-02-17 | **Archive flattening** — Handles .7z archives that extract into a subfolder | Claude Code |
| 2026-02-17 | **8MB stack size** — Linker flag prevents stack overflow from deeply nested Slint UI | Claude Code |
| 2026-02-17 | **WiFi from ISO** — WiFi drivers+DLLs now extracted from ISO's install.wim (not local machine) | Claude Code |
| 2026-02-17 | **ADK package defaults fixed** — Disabled Rejuv, SRT (no .cab in ADK), DismCmdlets (0x800f081e) | Claude Code |
| 2026-02-17 | **PE fix IDs fixed** — crash_dialogs/long_paths now match pe_fixes.rs (were silently skipped) | Claude Code |
| 2026-02-17 | **WallpaperHost fix** — takeown/icacls before deletion (TrustedInstaller ownership) | Claude Code |
| 2026-02-17 | **Driver count reporting** — Shows both DISM-injected and drvload-fallback counts | Claude Code |
| 2026-02-17 | **Branding wallpaper** — Embedded in EXE (917 KB), injected into WIM, registry keys for WinXShell | Claude Code |
| 2026-02-17 | **Windows Deploy module complete** (WI-001 to WI-008) — autounattend.xml generation, DISM WIM parsing, diskpart formatting, Win11 bypass | Claude Code |
| 2026-02-17 | **Deploy profiles** (CP-001, CP-002, CP-004) — Save/load JSON profiles with 50+ config fields, IT-focused defaults | Claude Code |
| 2026-02-17 | **Deploy tweaks** — 27 post-install tweaks: privacy(6), security(6), performance(3), UI(7), bloatware(5) via FirstLogonCommands | Claude Code |
| 2026-02-17 | **Domain join support** — Domain name, credentials, workgroup configuration in autounattend.xml | Claude Code |
| 2026-02-17 | **Deploy UI** — 11 collapsible sections, edition/disk selection, text inputs, preview XML, progress bar | Claude Code |
| 2026-02-18 | **Removed OU path** from domain join (not needed) | Claude Code |
| 2026-02-18 | **Profile system redesign** — ComboBox dropdown for saved profiles (auto-loads on select), Import button with file picker | Claude Code |
| 2026-02-18 | **ISO support** — Auto-mount ISO via PowerShell, find install.wim/esd inside, dismount on completion | Claude Code |
| 2026-02-18 | **ComboBox dropdowns** — Replaced text-based edition and disk selection with ComboBox widgets | Claude Code |
| 2026-02-18 | **Normal/Automated install modes** — Card-based mode selector (like AMPIPIT), separate pages for each mode | Claude Code |
| 2026-02-18 | **Post-install script management** — Add/Remove FirstLogon and SetupComplete scripts, copied to target after install | Claude Code |
| 2026-02-18 | **Script infrastructure** — RunAll.bat for FirstLogon, SetupComplete.cmd wrapper, file picker for .ps1/.bat/.cmd/.exe/.reg | Claude Code |
| 2026-02-18 | **Automated execute() Step 7** — Now calls copy_scripts_to_target() instead of TODO placeholder | Claude Code |
| 2026-02-18 | **System Prep module** — Tool launcher for SysprepPreparator (download from GitHub + launch) | Claude Code |
| 2026-02-18 | **ZIP extraction upgrade** — Detects complete apps (EXE + DLLs) and extracts everything, not just EXEs | Claude Code |
| 2026-02-18 | **Documentation update** — All docs brought up to date with current project state | Claude Code |
| 2026-02-18 | **Product key backup** (WI-011) — Detect Windows product key via WMI, save to file, copy to clipboard | Claude Code |
| 2026-02-18 | **Auto-update system** (AU-001 to AU-008) — GitHub releases check, self-replace EXE, sidebar badge, PE manifest refresh | Claude Code |
| 2026-02-18 | **App icon** (UI-006) — Custom MasterBooter icon embedded in EXE (winres) and window (Slint) | Claude Code |
| 2026-02-18 | **Removed SetupComplete** (PS-001) — FirstLogonCommands only (SetupComplete unreliable in WinPE) | Claude Code |
| 2026-02-18 | **GitHub repo setup** — Howweird/Masterbooter with README, acknowledgments, SVG logo | Claude Code |
| 2026-02-18 | **v0.1.1 release** — 7 fixes: fast key detection (WQL filter), script picker directory, XML preview path, removed PE Include checkboxes, multi-key management, PE tool hover descriptions, 404 handling in update check | Claude Code |
| 2026-02-19 | **WiFi driver signature fix** — Disable driver signature enforcement in BIOS + UEFI BCD stores (PhoenixPE approach) to fix "cannot verify digital signature" error for WiFi protocol drivers | Claude Code |
| 2026-02-19 | **Removed 4 redundant/broken PE fixes** — Profile folders, TEMP config, file associations (redundant with launcher script or broken), set resolution (broken). Kept 5 useful offline registry fixes. | Claude Code |
| 2026-02-19 | **Fixed 48% build hang** — Disabled Windows console Quick Edit mode at build thread start (clicking console paused the process) | Claude Code |
