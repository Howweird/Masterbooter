# Changelog

All notable changes to MasterBooter are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/).

---

## [0.1.1] - 2026-02-18

### Fixed
- **Product key detection speed**: Replaced client-side PowerShell filtering with server-side WQL filter. Detection now completes in under 5 seconds (was 30+ seconds).
- **Script file picker**: "Add Script" dialog now opens to the FirstLogon/ folder instead of the Windows desktop.
- **Preview XML location**: "Preview XML" now saves `autounattend_preview.xml` next to the EXE (e.g., on USB drive) instead of the Windows temp folder.
- **Update check on 404**: Clicking Settings no longer shows an error when no GitHub releases exist yet. HTTP 404 is handled gracefully as "you're on the latest version."

### Changed
- **Removed PE "Include" checkboxes**: The Drivers, Tools, and Network Support checkboxes in WinPE Builder have been removed. These are now always enabled — disabling them produced broken PE images and confused users.

### Added
- **Multi-key management**: `saved_keys.json` now stores multiple product keys (one per machine). The Deploy page has a ComboBox dropdown to select which machine's key to load, plus Load and Delete buttons. Old single-key files are automatically migrated.
- **PE tool hover descriptions**: Hovering over any PE tool checkbox now shows a description of that tool in the bottom bar (e.g., "Desktop shell with taskbar, start menu, and system tray" for WinXShell).
- **Registry fallback for edition**: If WMI fails to detect the Windows edition during key backup, the registry (`ProductName`) is used as a fallback.

---

## [0.1.0] - 2026-02-18

### Added
- Initial release with all four modes:
  - **Backup/Restore**: 5 tool launchers (Fab's AutoBackup, ProfWiz, Transwiz, Disk2VHD, HDD Raw Copy)
  - **Windows Deploy**: Normal + Automated install modes, autounattend.xml generation, 50+ config fields, 27 post-install tweaks, profile system, script management
  - **WinPE Builder**: ADK-based PE building, WiFi injection from ISO, 16 ADK packages, PE fixes, 12 bundled tools, branding wallpaper
  - **System Prep**: SysprepPreparator tool launcher
- Product key detection and backup (OEM + installed key via WMI + registry decode)
- Auto-update from GitHub Releases (check + download + self-replace EXE)
- Dark ocean-blue theme with sidebar navigation
- Software rendering for WinPE compatibility (no GPU required)
- Single portable EXE (~9 MB)
