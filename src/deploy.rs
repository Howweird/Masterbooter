// ============================================
// deploy.rs — Windows Deployment Module
// ============================================
//
// This module handles Windows deployment:
// 1. WIM/ESD edition parsing (via DISM)
// 2. Disk detection (PowerShell + diskpart fallback)
// 3. Complete autounattend.xml generation from scratch
// 4. Win11 hardware bypass registry keys
// 5. Disk pre-formatting with diskpart
// 6. Windows Setup launch with /noreboot /unattend
// 7. Deployment profile save/load (JSON files)
//
// Ported from AMPIPIT's automated_install.rs, adapted for
// MasterBooter's simpler architecture (no tokio, std::thread).
// ============================================

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ============================================
// ENUMS
// ============================================

/// Boot mode for Windows installation.
/// UEFI = modern boot (GPT partitions, EFI system partition).
/// BIOS = legacy boot (MBR partitions, active boot partition).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum BootMode {
    UEFI,
    BIOS,
}

impl Default for BootMode {
    fn default() -> Self {
        // Auto-detect from firmware_type environment variable (set by WinPE)
        // In WinPE, this tells us what the machine actually booted with
        if std::env::var("firmware_type")
            .map(|v| v.eq_ignore_ascii_case("UEFI"))
            .unwrap_or(false)
        {
            return BootMode::UEFI;
        }

        // Alternative detection: check for EFI folder (present on UEFI systems)
        if Path::new("X:\\EFI").exists() || Path::new("\\EFI").exists() {
            return BootMode::UEFI;
        }

        // Default to UEFI — most modern machines use it
        BootMode::UEFI
    }
}

impl std::fmt::Display for BootMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootMode::UEFI => write!(f, "UEFI"),
            BootMode::BIOS => write!(f, "BIOS"),
        }
    }
}

// ============================================
// GENERIC PRODUCT KEYS (Edition Selectors)
// ============================================
// These are NOT activation keys — they tell Windows Setup which edition to install.
// The machine activates later with a real key or digital license.
// These work with ALL key types (OEM, retail, volume) and are the standard approach
// used by deployment tools (Unattend Generator, MDT, etc.).
// Source: Microsoft + Unattend Generator (WindowsEdition.json)

/// Maps lowercase edition names to their generic product keys.
/// Used when the user doesn't provide a product key — ensures the correct
/// Windows edition installs without requiring activation at install time.
const GENERIC_KEYS: &[(&str, &str)] = &[
    ("home",                    "YTMG3-N6DKC-DKB77-7M9GH-8HVX7"),
    ("home n",                  "4CPRK-NM3K3-X6XXQ-RXX86-WXCHW"),
    ("home single language",    "BT79Q-G7N6G-PGBYW-4YWX6-6F4BT"),
    ("pro",                     "VK7JG-NPHTM-C97JM-9MPGT-3V66T"),
    ("pro n",                   "2B87N-8KFHP-DKV6R-Y2C8J-PKCKT"),
    ("pro education",           "8PTT6-RNW4C-6V7J2-C2D3X-MHBPB"),
    ("pro education n",         "GJTYN-HDMQY-FRR76-HVGC7-QPF8P"),
    ("pro for workstations",    "DXG7C-N36C4-C4HTG-X4T3X-2YV77"),
    ("pro n for workstations",  "WYPNQ-8C467-V2W6J-TX4WX-WT2RQ"),
    ("education",               "YNMGQ-8RYV3-4PGQ3-C8XTP-7CFBY"),
    ("education n",             "84NGF-MHBT6-FXBX8-QWJK7-DRR8H"),
    ("enterprise",              "XGVPP-NMH47-7TTHJ-W3FW7-8HV2C"),
    ("enterprise n",            "WGGHN-J84D6-QYCPR-T7PJ7-X766F"),
];

/// Look up the generic product key for a Windows edition.
/// The edition name comes from DISM output (e.g., "Windows 11 Pro", "Windows 10 Home").
/// We strip the "Windows 10/11 " prefix and match case-insensitively.
///
/// # Arguments
/// * `edition_name` — Full edition name from WIM (e.g., "Windows 11 Pro")
///
/// # Returns
/// * `Some("XXXXX-...")` — matching generic key
/// * `None` — no match found (unusual edition or empty string)
pub fn get_generic_key(edition_name: &str) -> Option<&'static str> {
    // Strip "Windows XX " prefix to get just the edition part
    // DISM returns names like "Windows 11 Pro", "Windows 10 Home N", etc.
    let lower = edition_name.to_lowercase();
    let edition = lower
        .strip_prefix("windows 11 ")
        .or_else(|| lower.strip_prefix("windows 10 "))
        .or_else(|| lower.strip_prefix("windows "))
        .unwrap_or(&lower);

    // Match against our generic keys table
    for (name, key) in GENERIC_KEYS {
        if edition == *name {
            return Some(key);
        }
    }
    None
}

// ============================================
// DATA STRUCTURES
// ============================================

/// Information about a Windows edition found in a WIM/ESD file.
/// Populated by parsing DISM /Get-WimInfo output.
#[derive(Debug, Clone)]
pub struct WimEdition {
    /// WIM image index (1-based, used to select the edition)
    pub index: u32,
    /// Edition name (e.g., "Windows 11 Pro")
    pub name: String,
    /// Uncompressed size in bytes
    pub size_bytes: u64,
}

impl WimEdition {
    /// Returns a human-readable size string (e.g., "4.5 GB")
    pub fn size_display(&self) -> String {
        let gb = self.size_bytes as f64 / 1_073_741_824.0;
        if gb >= 1.0 {
            format!("{:.1} GB", gb)
        } else {
            let mb = self.size_bytes as f64 / 1_048_576.0;
            format!("{:.0} MB", mb)
        }
    }
}

/// Information about a detected physical disk.
/// Populated by PowerShell Get-Disk or diskpart fallback.
#[derive(Debug, Clone)]
pub struct DiskInfo {
    /// Disk number (0-based, used with diskpart "select disk N")
    pub number: u32,
    /// Friendly name (e.g., "Samsung SSD 960 EVO")
    pub friendly_name: String,
    /// Total size in bytes
    pub size_bytes: u64,
    /// Partition style ("GPT" or "MBR")
    pub partition_style: String,
    /// Whether this is the system disk (disk containing C: or disk 0)
    pub is_system_disk: bool,
}

impl DiskInfo {
    /// Returns a human-readable size string (e.g., "500 GB" or "1.5 TB")
    pub fn size_display(&self) -> String {
        let gb = self.size_bytes as f64 / 1_073_741_824.0;
        if gb >= 1024.0 {
            let tb = gb / 1024.0;
            format!("{:.1} TB", tb)
        } else {
            format!("{:.0} GB", gb)
        }
    }

    /// Returns a full display string for the UI (e.g., "Disk 0: Samsung SSD (500 GB, GPT)")
    pub fn display_string(&self) -> String {
        let system_tag = if self.is_system_disk { " [SYSTEM]" } else { "" };
        format!(
            "Disk {}: {} ({}, {}){}",
            self.number,
            self.friendly_name,
            self.size_display(),
            self.partition_style,
            system_tag
        )
    }
}

/// Main configuration struct — holds ALL deployment settings.
/// Maps 1:1 to UI properties for easy reading/writing.
/// Derives Serialize/Deserialize for profile save/load as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployConfig {
    // ============================================
    // Image Selection
    // ============================================
    /// Path to install.wim or install.esd (or ISO to mount)
    #[serde(default)]
    pub wim_path: PathBuf,
    /// Selected edition name (e.g., "Windows 11 Pro")
    #[serde(default)]
    pub edition: String,
    /// Selected edition index in the WIM (1-based)
    #[serde(default)]
    pub edition_index: u32,

    // ============================================
    // Machine Identity
    // ============================================
    /// Computer name (max 15 chars, or "*" for auto-generate)
    pub computer_name: String,
    /// Windows timezone identifier (e.g., "Eastern Standard Time")
    pub timezone: String,
    /// Language/locale code (e.g., "en-US")
    pub language: String,

    // ============================================
    // Boot & Disk
    // ============================================
    /// Boot mode: UEFI (GPT) or BIOS (MBR)
    pub boot_mode: BootMode,
    /// Target disk number (-1 = let Windows choose/prompt)
    pub disk_id: i32,
    /// Enable Windows 11 hardware requirements bypass
    pub bypass_win11: bool,

    // ============================================
    // User Account (creates one new local account)
    // ============================================
    /// Username for the new account (e.g., "Admin")
    pub user_name: String,
    /// Password for the new account (empty = no password)
    pub user_password: String,
    /// Display name shown on login screen
    pub user_display_name: String,
    /// Whether the user should be an Administrator (true) or standard User (false)
    pub user_is_admin: bool,
    /// Automatically log in as this user after setup
    pub enable_autologon: bool,

    // ============================================
    // OOBE (Out-of-Box Experience) Control
    // ============================================
    /// Skip the entire Out-of-Box Experience
    pub skip_oobe: bool,
    /// Skip the EULA acceptance screen
    pub skip_eula: bool,
    /// Skip network configuration (offline install)
    pub skip_network: bool,

    // ============================================
    // Optional Registration Info
    // ============================================
    /// Windows product key (leave empty to skip)
    #[serde(default)]
    pub product_key: String,
    /// Organization name for Windows registration
    #[serde(default)]
    pub organization: String,
    /// Owner name for Windows registration
    #[serde(default)]
    pub owner_name: String,

    // ============================================
    // Privacy & Telemetry (6 toggles)
    // ============================================
    /// Disable telemetry and diagnostic data collection
    pub disable_telemetry: bool,
    /// Disable location history tracking
    pub disable_location: bool,
    /// Disable personalized advertising ID
    pub disable_ads: bool,
    /// Disable suggested apps and Content Delivery Manager
    pub disable_suggested_apps: bool,
    /// Disable Bing search results in Start menu
    pub disable_bing_search: bool,
    /// Disable Windows SmartScreen filter
    pub disable_smartscreen: bool,

    // ============================================
    // System Security (6 toggles)
    // ============================================
    /// Enable Remote Desktop (RDP) for remote access
    pub enable_rdp: bool,
    /// Disable User Account Control prompts (not recommended)
    pub disable_uac: bool,
    /// Disable Windows Defender antivirus (not recommended)
    pub disable_defender: bool,
    /// Disable Windows Firewall (not recommended)
    pub disable_firewall: bool,
    /// Disable Core Isolation / Virtualization-Based Security
    pub disable_vbs: bool,
    /// Disable automatic BitLocker device encryption
    pub disable_bitlocker: bool,

    // ============================================
    // Performance & Power (3 toggles)
    // ============================================
    /// Disable Fast Startup (forces clean cold boot every time)
    pub disable_fast_startup: bool,
    /// Enable High Performance power plan
    pub high_performance: bool,
    /// Disable System Restore (saves disk space)
    pub disable_system_restore: bool,

    // ============================================
    // UI Customization (7 toggles)
    // ============================================
    /// Show file extensions in Explorer (e.g., "document.docx" instead of "document")
    pub show_file_extensions: bool,
    /// Show hidden files and folders in Explorer
    pub show_hidden_files: bool,
    /// Use classic right-click context menu on Windows 11
    pub classic_context_menu: bool,
    /// Taskbar search display mode: 0=show box, 1=icon only, 2=hidden
    pub taskbar_search_mode: u8,
    /// Hide the Task View button on the taskbar
    pub hide_task_view: bool,
    /// Hide the Widgets button on the taskbar (Windows 11)
    pub hide_widgets: bool,
    /// Left-align the taskbar instead of centered (Windows 11)
    pub taskbar_left_align: bool,

    // ============================================
    // Bloatware Removal (5 named apps)
    // ============================================
    /// Disable Cortana
    pub disable_cortana: bool,
    /// Disable OneDrive auto-install
    pub disable_onedrive: bool,
    /// Disable Teams chat icon
    pub disable_teams: bool,
    /// Disable Copilot AI assistant
    pub disable_copilot: bool,
    /// Disable Widgets service
    pub disable_widgets_service: bool,

    // ============================================
    // Domain Join (enterprise)
    // ============================================
    /// Join a domain instead of workgroup
    pub join_domain: bool,
    /// Domain name to join (e.g., "contoso.com")
    #[serde(default)]
    pub domain_name: String,
    /// Domain join username (e.g., "DOMAIN\\Administrator")
    #[serde(default)]
    pub domain_username: String,
    /// Domain join password
    #[serde(default)]
    pub domain_password: String,
    /// Workgroup name (used if not joining domain)
    pub workgroup: String,

    // ============================================
    // Advanced
    // ============================================
    /// Prevent automatic device encryption during setup
    pub prevent_device_encryption: bool,
}

impl Default for DeployConfig {
    /// Creates a DeployConfig with sensible IT-deployment defaults.
    /// Privacy stuff disabled, RDP enabled, bloatware removed, etc.
    fn default() -> Self {
        Self {
            // Image selection — empty until user browses
            wim_path: PathBuf::new(),
            edition: String::new(),
            edition_index: 0,

            // Machine identity
            computer_name: "*".to_string(), // "*" means auto-generate
            timezone: "Eastern Standard Time".to_string(),
            language: "en-US".to_string(),

            // Boot & Disk
            boot_mode: BootMode::default(),
            disk_id: -1, // -1 = let Windows choose
            bypass_win11: true,

            // User account — create "Admin" with admin rights
            user_name: "Admin".to_string(),
            user_password: String::new(),
            user_display_name: "Administrator".to_string(),
            user_is_admin: true,
            enable_autologon: true,

            // OOBE — skip everything for clean automated install
            skip_oobe: true,
            skip_eula: true,
            skip_network: false,

            // Optional registration — empty by default
            product_key: String::new(),
            organization: String::new(),
            owner_name: String::new(),

            // Privacy — disable tracking/telemetry for IT deployment
            disable_telemetry: true,
            disable_location: true,
            disable_ads: true,
            disable_suggested_apps: true,
            disable_bing_search: true,
            disable_smartscreen: false, // Keep SmartScreen — it's useful for security

            // Security — keep protections on, enable RDP for remote management
            enable_rdp: true,
            disable_uac: false,       // Keep UAC — it's important
            disable_defender: false,   // Keep Defender — it's important
            disable_firewall: false,   // Keep Firewall — it's important
            disable_vbs: false,        // Keep VBS — it's a security feature
            disable_bitlocker: true,   // Disable auto-encryption (IT controls this)

            // Performance — high performance for workstations
            disable_fast_startup: true,    // Clean boots are more reliable
            high_performance: true,        // No sleep/throttling
            disable_system_restore: false, // Keep System Restore

            // UI — show extensions, classic menu, clean taskbar
            show_file_extensions: true,
            show_hidden_files: false,
            classic_context_menu: true,  // Classic right-click on Win11
            taskbar_search_mode: 2,      // Hide search box
            hide_task_view: true,
            hide_widgets: true,
            taskbar_left_align: false,   // Keep Win11 centered (default)

            // Bloatware — remove common annoyances
            disable_cortana: true,
            disable_onedrive: false,      // Keep OneDrive (many users need it)
            disable_teams: true,
            disable_copilot: true,
            disable_widgets_service: true,

            // Domain — workgroup by default
            join_domain: false,
            domain_name: String::new(),
            domain_username: String::new(),
            domain_password: String::new(),
            workgroup: "WORKGROUP".to_string(),

            // Advanced
            prevent_device_encryption: true,
        }
    }
}

/// Result of the deployment execution pipeline.
/// Returned by execute() after all steps complete or fail.
#[derive(Debug, Clone)]
pub struct DeployResult {
    /// Whether the deployment completed successfully
    pub success: bool,
    /// Human-readable status message (success info or error details)
    pub message: String,
}

// ============================================
// WIM EDITION PARSING
// ============================================

/// If the given path is an ISO file, mount it and find the install.wim or install.esd inside.
/// Returns the path to the actual WIM/ESD file (and the mount drive letter to dismount later).
///
/// If it's already a WIM/ESD file, returns the path unchanged.
///
/// # Arguments
/// * `image_path` — Path to an ISO, WIM, or ESD file
///
/// # Returns
/// * `Ok((PathBuf, Option<String>))` — (wim_path, mounted_drive_letter)
///   - mounted_drive_letter is Some("E:") if we mounted an ISO (needs dismounting after)
///   - mounted_drive_letter is None if path was already a WIM/ESD
/// * `Err(String)` — error message
pub fn resolve_image_to_wim(image_path: &Path) -> Result<(PathBuf, Option<String>), String> {
    let ext = image_path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // If it's already a WIM or ESD file, return as-is
    if ext == "wim" || ext == "esd" {
        return Ok((image_path.to_path_buf(), None));
    }

    // If it's an ISO, mount it using PowerShell and find the WIM inside
    if ext == "iso" {
        println!("[Deploy] ISO detected — mounting to find install.wim...");

        // Mount the ISO using PowerShell (built into Windows 8+)
        // Returns the drive letter of the mounted ISO
        let mount_output = Command::new("powershell")
            .args([
                "-NoProfile", "-Command",
                &format!(
                    "(Mount-DiskImage -ImagePath '{}' -PassThru | Get-Volume).DriveLetter",
                    image_path.display()
                )
            ])
            .output()
            .map_err(|e| format!("Failed to mount ISO: {}", e))?;

        if !mount_output.status.success() {
            let stderr = String::from_utf8_lossy(&mount_output.stderr);
            return Err(format!("Failed to mount ISO: {}", stderr.trim()));
        }

        let drive_letter = String::from_utf8_lossy(&mount_output.stdout).trim().to_string();
        if drive_letter.is_empty() {
            return Err("ISO mounted but no drive letter assigned".to_string());
        }

        let drive = format!("{}:", drive_letter);
        println!("[Deploy] ISO mounted at drive {}", drive);

        // Look for install.wim or install.esd in the sources folder
        let wim_path = PathBuf::from(format!("{}\\sources\\install.wim", drive));
        let esd_path = PathBuf::from(format!("{}\\sources\\install.esd", drive));

        if wim_path.exists() {
            println!("[Deploy] Found install.wim at: {}", wim_path.display());
            return Ok((wim_path, Some(drive)));
        } else if esd_path.exists() {
            println!("[Deploy] Found install.esd at: {}", esd_path.display());
            return Ok((esd_path, Some(drive)));
        } else {
            // Dismount since we can't find anything useful
            let _ = dismount_iso(image_path);
            return Err(format!(
                "No install.wim or install.esd found in ISO at {}\\sources\\",
                drive
            ));
        }
    }

    // Unknown file type
    Err(format!(
        "Unsupported file type: .{}. Please select a .wim, .esd, or .iso file.",
        ext
    ))
}

/// Dismount a previously mounted ISO image.
///
/// # Arguments
/// * `iso_path` — Path to the original ISO file (same path used to mount)
pub fn dismount_iso(iso_path: &Path) -> Result<(), String> {
    println!("[Deploy] Dismounting ISO: {}", iso_path.display());
    let output = Command::new("powershell")
        .args([
            "-NoProfile", "-Command",
            &format!("Dismount-DiskImage -ImagePath '{}'", iso_path.display())
        ])
        .output()
        .map_err(|e| format!("Failed to dismount ISO: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to dismount ISO: {}", stderr.trim()));
    }
    println!("[Deploy] ISO dismounted successfully");
    Ok(())
}

/// Parse available Windows editions from a WIM or ESD file using DISM.
/// Runs: dism.exe /Get-WimInfo /WimFile:<path>
/// Returns a list of editions with index, name, description, and size.
///
/// If the path is an ISO, it will be mounted automatically to find the WIM inside.
/// The resolved WIM path is also returned so the caller can update the UI.
///
/// BLOCKING — call from a worker thread, not the UI thread.
///
/// # Arguments
/// * `image_path` — Path to install.wim, install.esd, or a .iso file
///
/// # Returns
/// * `Ok((Vec<WimEdition>, PathBuf))` — (editions list, resolved WIM path)
/// * `Err(String)` — error message if DISM fails
pub fn parse_wim_editions(image_path: &Path) -> Result<(Vec<WimEdition>, PathBuf), String> {
    println!("[Deploy] Parsing WIM editions from: {}", image_path.display());

    // Check the file exists
    if !image_path.exists() {
        return Err(format!("Image file not found: {}", image_path.display()));
    }

    // If it's an ISO, mount it and find the WIM inside
    let (wim_path, _mounted_drive) = resolve_image_to_wim(image_path)?;

    // Run DISM to get WIM info
    // dism.exe /Get-WimInfo /WimFile:"C:\path\to\install.wim"
    let output = Command::new("dism.exe")
        .args(["/Get-WimInfo", &format!("/WimFile:{}", wim_path.display())])
        .output()
        .map_err(|e| format!("Failed to run DISM: {}. Is DISM installed?", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Check for errors
    if !output.status.success() {
        return Err(format!(
            "DISM failed (exit code {}): {}{}",
            output.status.code().unwrap_or(-1),
            stdout,
            stderr
        ));
    }

    // Parse the DISM output line by line.
    // Each edition block looks like:
    //   Index : 1
    //   Name : Windows 11 Pro
    //   Description : Windows 11 Pro
    //   Size : 4,123,456,789 bytes
    let mut editions: Vec<WimEdition> = Vec::new();
    let mut current_index: Option<u32> = None;
    let mut current_name = String::new();
    let mut current_size: u64 = 0;

    for line in stdout.lines() {
        let line = line.trim();

        if line.starts_with("Index :") || line.starts_with("Index:") {
            // If we have a previous edition, save it first
            if let Some(idx) = current_index {
                editions.push(WimEdition {
                    index: idx,
                    name: current_name.clone(),
                    size_bytes: current_size,
                });
            }
            // Start a new edition
            let val = line.split(':').nth(1).unwrap_or("").trim();
            current_index = val.parse::<u32>().ok();
            current_name = String::new();
            current_size = 0;
        } else if line.starts_with("Name :") || line.starts_with("Name:") {
            current_name = line.split(':').nth(1).unwrap_or("").trim().to_string();
        } else if line.starts_with("Size :") || line.starts_with("Size:") {
            // Size line looks like: "Size : 4,123,456,789 bytes"
            // Remove commas, spaces, and "bytes" to get the number
            let size_str = line.split(':').nth(1).unwrap_or("").trim();
            let digits_only: String = size_str
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect();
            current_size = digits_only.parse::<u64>().unwrap_or(0);
        }
    }

    // Don't forget the last edition in the output
    if let Some(idx) = current_index {
        editions.push(WimEdition {
            index: idx,
            name: current_name,
            size_bytes: current_size,
        });
    }

    if editions.is_empty() {
        return Err("No Windows editions found in the image. Is this a valid install.wim or install.esd?".to_string());
    }

    println!("[Deploy] Found {} edition(s):", editions.len());
    for e in &editions {
        println!("  Index {}: {} ({})", e.index, e.name, e.size_display());
    }

    // Return both the editions and the resolved WIM path
    // (important when an ISO was mounted — caller needs the WIM path for setup.exe)
    Ok((editions, wim_path))
}

// ============================================
// DISK DETECTION
// ============================================

/// Detect available physical disks on the system.
/// Tries PowerShell first (full info), falls back to diskpart (WinPE compatible).
/// Filters out USB drives. Marks the system disk.
///
/// BLOCKING — call from a worker thread, not the UI thread.
///
/// # Returns
/// * `Ok(Vec<DiskInfo>)` — list of detected disks
/// * `Err(String)` — error message if both detection methods fail
pub fn detect_disks() -> Result<Vec<DiskInfo>, String> {
    println!("[Deploy] Detecting available disks...");

    // First try to detect the system disk number (the disk containing C:)
    let system_disk = get_system_disk_number();

    // Try PowerShell first — gives us friendly names and partition style
    match detect_disks_powershell(system_disk) {
        Ok(disks) if !disks.is_empty() => {
            println!("[Deploy] Detected {} disk(s) via PowerShell", disks.len());
            return Ok(disks);
        }
        Ok(_) => {
            println!("[Deploy] PowerShell found 0 disks, trying diskpart...");
        }
        Err(e) => {
            println!("[Deploy] PowerShell detection failed: {}, trying diskpart...", e);
        }
    }

    // Fall back to diskpart — works in WinPE where PowerShell may be limited
    match detect_disks_diskpart(system_disk) {
        Ok(disks) if !disks.is_empty() => {
            println!("[Deploy] Detected {} disk(s) via diskpart", disks.len());
            Ok(disks)
        }
        Ok(_) => Err("No disks detected. Are any physical disks connected?".to_string()),
        Err(e) => Err(format!("Disk detection failed: {}", e)),
    }
}

/// Detect disks using PowerShell Get-Disk command.
/// Filters out USB bus type to avoid listing USB flash drives.
/// Output format: "Number|FriendlyName|Size|PartitionStyle" per line.
fn detect_disks_powershell(system_disk: Option<u32>) -> Result<Vec<DiskInfo>, String> {
    // PowerShell command to list non-USB disks
    // Outputs: "0|Samsung SSD 960 EVO|500107862016|GPT"
    let ps_script = r#"Get-Disk | Where-Object { $_.BusType -ne 'USB' } | ForEach-Object { "$($_.Number)|$($_.FriendlyName.Trim())|$($_.Size)|$($_.PartitionStyle)" }"#;

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
        .output()
        .map_err(|e| format!("Failed to run PowerShell: {}", e))?;

    if !output.status.success() {
        return Err("PowerShell Get-Disk command failed".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut disks: Vec<DiskInfo> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse: "0|Samsung SSD 960 EVO|500107862016|GPT"
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 4 {
            let number = parts[0].trim().parse::<u32>().unwrap_or(0);
            let friendly_name = parts[1].trim().to_string();
            let size_bytes = parts[2].trim().parse::<u64>().unwrap_or(0);
            let partition_style = parts[3].trim().to_string();

            // Check if this is the system disk
            let is_system = system_disk.map_or(number == 0, |sd| number == sd);

            disks.push(DiskInfo {
                number,
                friendly_name,
                size_bytes,
                partition_style,
                is_system_disk: is_system,
            });
        }
    }

    Ok(disks)
}

/// Detect disks using diskpart "list disk" command.
/// This is the fallback for WinPE where PowerShell's Get-Disk may not work.
/// Parses output like: "  Disk 0    Online       238 GB  1024 KB  *"
fn detect_disks_diskpart(system_disk: Option<u32>) -> Result<Vec<DiskInfo>, String> {
    // Create a temporary diskpart script that just lists disks
    let temp_dir = std::env::temp_dir();
    let script_path = temp_dir.join("mb_list_disks.txt");
    fs::write(&script_path, "list disk\n")
        .map_err(|e| format!("Failed to write diskpart script: {}", e))?;

    // Run diskpart with the script
    let output = Command::new("diskpart")
        .args(["/s", &script_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("Failed to run diskpart: {}", e))?;

    // Clean up the temp script
    let _ = fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut disks: Vec<DiskInfo> = Vec::new();

    // Parse diskpart output. Each disk line looks like:
    //   Disk 0    Online       238 GB  1024 KB  *
    //   Disk 1    Online       931 GB  0 B
    // The "*" at the end indicates GPT partition style
    for line in stdout.lines() {
        let line = line.trim();

        // Look for lines starting with "Disk" followed by a number
        if !line.starts_with("Disk ") {
            continue;
        }

        // Skip the header line "Disk ###  Status  Size  Free  Dyn  Gpt"
        if line.contains("###") || line.contains("Status") {
            continue;
        }

        // Parse the disk number (first number after "Disk ")
        let rest = line.strip_prefix("Disk ").unwrap_or("");
        let number_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let number = match number_str.parse::<u32>() {
            Ok(n) => n,
            Err(_) => continue,
        };

        // Parse size: look for "XXX GB" or "XXX TB" or "XXX MB"
        let mut size_bytes: u64 = 0;
        let words: Vec<&str> = line.split_whitespace().collect();
        for i in 0..words.len() {
            if let Ok(num) = words[i].parse::<u64>() {
                if i + 1 < words.len() {
                    match words[i + 1] {
                        "TB" => size_bytes = num * 1_099_511_627_776,
                        "GB" => size_bytes = num * 1_073_741_824,
                        "MB" => size_bytes = num * 1_048_576,
                        _ => {}
                    }
                }
            }
        }

        // Check for GPT indicator (asterisk at the end)
        let partition_style = if line.ends_with('*') {
            "GPT".to_string()
        } else {
            "MBR".to_string()
        };

        let is_system = system_disk.map_or(number == 0, |sd| number == sd);

        disks.push(DiskInfo {
            number,
            friendly_name: format!("Disk {}", number), // diskpart doesn't give friendly names
            size_bytes,
            partition_style,
            is_system_disk: is_system,
        });
    }

    Ok(disks)
}

/// Detect which physical disk contains the C: drive.
/// Used to mark the system disk in the UI (so the user doesn't format it by accident).
fn get_system_disk_number() -> Option<u32> {
    // Use PowerShell to find which disk number contains the C: partition
    let ps_script = r#"(Get-Partition -DriveLetter C -ErrorAction SilentlyContinue).DiskNumber"#;

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim().parse::<u32>().ok()
    } else {
        None
    }
}

// ============================================
// AUTOUNATTEND.XML GENERATION
// ============================================

/// Generate a complete autounattend.xml from the DeployConfig.
/// Builds the XML from scratch — no template file needed.
///
/// The XML has three passes:
/// 1. windowsPE — disk configuration, language, install source
/// 2. specialize — computer name, timezone, registration
/// 3. oobeSystem — user accounts, OOBE settings, FirstLogonCommands (tweaks)
///
/// # Arguments
/// * `config` — The deployment configuration with all settings
///
/// # Returns
/// Complete XML string ready to write to a file
pub fn generate_autounattend(config: &DeployConfig) -> String {
    println!("[Deploy] Generating autounattend.xml...");

    let mut xml = String::new();

    // XML header
    xml.push_str(r#"<?xml version="1.0" encoding="utf-8"?>"#);
    xml.push('\n');
    xml.push_str(r#"<unattend xmlns="urn:schemas-microsoft-com:unattend">"#);
    xml.push('\n');

    // ============================================
    // PASS 1: windowsPE — Setup configuration
    // ============================================
    xml.push_str(r#"    <settings pass="windowsPE">"#);
    xml.push('\n');

    // Microsoft-Windows-International-Core-WinPE — Language settings
    xml.push_str(r#"        <component name="Microsoft-Windows-International-Core-WinPE" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#);
    xml.push('\n');
    xml.push_str(&format!("            <SetupUILanguage>\n                <UILanguage>{}</UILanguage>\n            </SetupUILanguage>\n", escape_xml(&config.language)));
    xml.push_str(&format!("            <InputLocale>{}</InputLocale>\n", escape_xml(&config.language)));
    xml.push_str(&format!("            <SystemLocale>{}</SystemLocale>\n", escape_xml(&config.language)));
    xml.push_str(&format!("            <UILanguage>{}</UILanguage>\n", escape_xml(&config.language)));
    xml.push_str(&format!("            <UserLocale>{}</UserLocale>\n", escape_xml(&config.language)));
    xml.push_str("        </component>\n");

    // Microsoft-Windows-Setup — Disk config + image selection
    xml.push_str(r#"        <component name="Microsoft-Windows-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#);
    xml.push('\n');

    // Disk configuration (only if user selected a specific disk)
    if config.disk_id >= 0 {
        xml.push_str("            <DiskConfiguration>\n");
        xml.push_str("                <WillShowUI>OnError</WillShowUI>\n");
        xml.push_str(&format!("                <Disk wcm:action=\"add\">\n                    <DiskID>{}</DiskID>\n                    <WillWipeDisk>true</WillWipeDisk>\n", config.disk_id));

        match config.boot_mode {
            BootMode::UEFI => {
                // UEFI: EFI partition (100MB) + MSR (16MB) + OS partition (rest)
                xml.push_str("                    <CreatePartitions>\n");
                // Partition 1: EFI System Partition
                xml.push_str("                        <CreatePartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>1</Order>\n");
                xml.push_str("                            <Size>100</Size>\n");
                xml.push_str("                            <Type>EFI</Type>\n");
                xml.push_str("                        </CreatePartition>\n");
                // Partition 2: Microsoft Reserved
                xml.push_str("                        <CreatePartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>2</Order>\n");
                xml.push_str("                            <Size>16</Size>\n");
                xml.push_str("                            <Type>MSR</Type>\n");
                xml.push_str("                        </CreatePartition>\n");
                // Partition 3: Windows OS (rest of disk)
                xml.push_str("                        <CreatePartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>3</Order>\n");
                xml.push_str("                            <Extend>true</Extend>\n");
                xml.push_str("                            <Type>Primary</Type>\n");
                xml.push_str("                        </CreatePartition>\n");
                xml.push_str("                    </CreatePartitions>\n");
                // Format partitions
                xml.push_str("                    <ModifyPartitions>\n");
                // Format EFI as FAT32
                xml.push_str("                        <ModifyPartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>1</Order>\n");
                xml.push_str("                            <PartitionID>1</PartitionID>\n");
                xml.push_str("                            <Format>FAT32</Format>\n");
                xml.push_str("                            <Label>System</Label>\n");
                xml.push_str("                        </ModifyPartition>\n");
                // MSR doesn't need formatting (partition 2)
                xml.push_str("                        <ModifyPartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>2</Order>\n");
                xml.push_str("                            <PartitionID>2</PartitionID>\n");
                xml.push_str("                        </ModifyPartition>\n");
                // Format OS partition as NTFS
                xml.push_str("                        <ModifyPartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>3</Order>\n");
                xml.push_str("                            <PartitionID>3</PartitionID>\n");
                xml.push_str("                            <Format>NTFS</Format>\n");
                xml.push_str("                            <Label>Windows</Label>\n");
                xml.push_str("                            <Letter>C</Letter>\n");
                xml.push_str("                        </ModifyPartition>\n");
                xml.push_str("                    </ModifyPartitions>\n");
            }
            BootMode::BIOS => {
                // BIOS: System Reserved (100MB, active) + OS partition (rest)
                xml.push_str("                    <CreatePartitions>\n");
                // Partition 1: System Reserved (boot)
                xml.push_str("                        <CreatePartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>1</Order>\n");
                xml.push_str("                            <Size>100</Size>\n");
                xml.push_str("                            <Type>Primary</Type>\n");
                xml.push_str("                        </CreatePartition>\n");
                // Partition 2: Windows OS (rest of disk)
                xml.push_str("                        <CreatePartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>2</Order>\n");
                xml.push_str("                            <Extend>true</Extend>\n");
                xml.push_str("                            <Type>Primary</Type>\n");
                xml.push_str("                        </CreatePartition>\n");
                xml.push_str("                    </CreatePartitions>\n");
                // Format partitions
                xml.push_str("                    <ModifyPartitions>\n");
                // System Reserved: NTFS, active
                xml.push_str("                        <ModifyPartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>1</Order>\n");
                xml.push_str("                            <PartitionID>1</PartitionID>\n");
                xml.push_str("                            <Active>true</Active>\n");
                xml.push_str("                            <Format>NTFS</Format>\n");
                xml.push_str("                            <Label>System Reserved</Label>\n");
                xml.push_str("                        </ModifyPartition>\n");
                // OS partition: NTFS
                xml.push_str("                        <ModifyPartition wcm:action=\"add\">\n");
                xml.push_str("                            <Order>2</Order>\n");
                xml.push_str("                            <PartitionID>2</PartitionID>\n");
                xml.push_str("                            <Format>NTFS</Format>\n");
                xml.push_str("                            <Label>Windows</Label>\n");
                xml.push_str("                            <Letter>C</Letter>\n");
                xml.push_str("                        </ModifyPartition>\n");
                xml.push_str("                    </ModifyPartitions>\n");
            }
        }
        xml.push_str("                </Disk>\n");
        xml.push_str("            </DiskConfiguration>\n");
    }

    // Image Install — which edition to install
    if !config.edition.is_empty() {
        // Tell Setup where to install Windows (which partition)
        let install_partition = if config.disk_id >= 0 {
            match config.boot_mode {
                BootMode::UEFI => "3", // Partition 3 (after EFI and MSR)
                BootMode::BIOS => "2", // Partition 2 (after System Reserved)
            }
        } else {
            "" // Let Windows choose
        };

        xml.push_str("            <ImageInstall>\n");
        xml.push_str("                <OSImage>\n");
        if config.disk_id >= 0 {
            xml.push_str(&format!("                    <InstallTo>\n                        <DiskID>{}</DiskID>\n                        <PartitionID>{}</PartitionID>\n                    </InstallTo>\n", config.disk_id, install_partition));
        }
        xml.push_str("                    <InstallFrom>\n");
        xml.push_str("                        <MetaData wcm:action=\"add\">\n");
        xml.push_str("                            <Key>/IMAGE/NAME</Key>\n");
        xml.push_str(&format!("                            <Value>{}</Value>\n", escape_xml(&config.edition)));
        xml.push_str("                        </MetaData>\n");
        xml.push_str("                    </InstallFrom>\n");
        xml.push_str("                </OSImage>\n");
        xml.push_str("            </ImageInstall>\n");
    }

    // Product key in windowsPE pass
    // Priority: 1) User's real key  2) Generic key for selected edition
    // Generic keys just select the edition — they don't activate Windows.
    // This ensures the correct edition installs even without a real key.
    let effective_key = if !config.product_key.is_empty() {
        // User provided a real product key — use it for both edition selection and activation
        println!("[Deploy] Using user-provided product key");
        config.product_key.clone()
    } else if let Some(generic) = get_generic_key(&config.edition) {
        // No user key — auto-fill generic key so the correct edition installs
        println!("[Deploy] No product key provided — using generic key for '{}'", config.edition);
        generic.to_string()
    } else {
        // Unknown edition — no key at all (Windows may prompt during setup)
        println!("[Deploy] No product key and unknown edition '{}' — skipping key", config.edition);
        String::new()
    };

    xml.push_str("            <UserData>\n");
    xml.push_str("                <AcceptEula>true</AcceptEula>\n");
    if !effective_key.is_empty() {
        xml.push_str("                <ProductKey>\n");
        xml.push_str(&format!("                    <Key>{}</Key>\n", escape_xml(&effective_key)));
        xml.push_str("                </ProductKey>\n");
    }
    xml.push_str("            </UserData>\n");

    xml.push_str("        </component>\n");
    xml.push_str("    </settings>\n");

    // ============================================
    // PASS 2: specialize — Machine identity
    // ============================================
    xml.push_str(r#"    <settings pass="specialize">"#);
    xml.push('\n');
    xml.push_str(r#"        <component name="Microsoft-Windows-Shell-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#);
    xml.push('\n');
    xml.push_str(&format!("            <ComputerName>{}</ComputerName>\n", escape_xml(&config.computer_name)));
    xml.push_str(&format!("            <TimeZone>{}</TimeZone>\n", escape_xml(&config.timezone)));

    if !config.organization.is_empty() {
        xml.push_str(&format!("            <RegisteredOrganization>{}</RegisteredOrganization>\n", escape_xml(&config.organization)));
    }
    if !config.owner_name.is_empty() {
        xml.push_str(&format!("            <RegisteredOwner>{}</RegisteredOwner>\n", escape_xml(&config.owner_name)));
    }

    xml.push_str("        </component>\n");

    // Prevent device encryption during specialize pass
    if config.prevent_device_encryption {
        xml.push_str(r#"        <component name="Microsoft-Windows-SecureStartup-FilterDriver" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#);
        xml.push('\n');
        xml.push_str("            <PreventDeviceEncryption>true</PreventDeviceEncryption>\n");
        xml.push_str("        </component>\n");
    }

    xml.push_str("    </settings>\n");

    // ============================================
    // PASS 3: oobeSystem — User, OOBE, Tweaks
    // ============================================
    xml.push_str(r#"    <settings pass="oobeSystem">"#);
    xml.push('\n');
    xml.push_str(r#"        <component name="Microsoft-Windows-Shell-Setup" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#);
    xml.push('\n');

    // Auto-logon configuration (optional)
    if config.enable_autologon && !config.user_name.is_empty() {
        xml.push_str("            <AutoLogon>\n");
        xml.push_str("                <Enabled>true</Enabled>\n");
        xml.push_str("                <LogonCount>1</LogonCount>\n");
        xml.push_str(&format!("                <Username>{}</Username>\n", escape_xml(&config.user_name)));
        if !config.user_password.is_empty() {
            xml.push_str("                <Password>\n");
            xml.push_str(&format!("                    <Value>{}</Value>\n", escape_xml(&config.user_password)));
            xml.push_str("                    <PlainText>true</PlainText>\n");
            xml.push_str("                </Password>\n");
        }
        xml.push_str("            </AutoLogon>\n");
    }

    // User account creation
    if !config.user_name.is_empty() {
        xml.push_str("            <UserAccounts>\n");
        xml.push_str("                <LocalAccounts>\n");
        xml.push_str("                    <LocalAccount wcm:action=\"add\">\n");
        xml.push_str(&format!("                        <Name>{}</Name>\n", escape_xml(&config.user_name)));
        if !config.user_display_name.is_empty() {
            xml.push_str(&format!("                        <DisplayName>{}</DisplayName>\n", escape_xml(&config.user_display_name)));
        }
        // Group: Administrators or Users
        let group = if config.user_is_admin { "Administrators" } else { "Users" };
        xml.push_str(&format!("                        <Group>{}</Group>\n", group));
        if !config.user_password.is_empty() {
            xml.push_str("                        <Password>\n");
            xml.push_str(&format!("                            <Value>{}</Value>\n", escape_xml(&config.user_password)));
            xml.push_str("                            <PlainText>true</PlainText>\n");
            xml.push_str("                        </Password>\n");
        }
        xml.push_str("                    </LocalAccount>\n");
        xml.push_str("                </LocalAccounts>\n");
        xml.push_str("            </UserAccounts>\n");
    }

    // OOBE settings
    xml.push_str("            <OOBE>\n");
    if config.skip_eula {
        xml.push_str("                <HideEULAPage>true</HideEULAPage>\n");
    }
    if config.skip_oobe {
        xml.push_str("                <HideOEMRegistrationScreen>true</HideOEMRegistrationScreen>\n");
        xml.push_str("                <HideOnlineAccountScreens>true</HideOnlineAccountScreens>\n");
        xml.push_str("                <HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE>\n");
        xml.push_str("                <SkipMachineOOBE>true</SkipMachineOOBE>\n");
        xml.push_str("                <SkipUserOOBE>true</SkipUserOOBE>\n");
    }
    if config.skip_network {
        xml.push_str("                <HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE>\n");
    }
    xml.push_str("                <ProtectYourPC>3</ProtectYourPC>\n"); // 3 = Don't change settings
    xml.push_str("                <NetworkLocation>Work</NetworkLocation>\n");
    xml.push_str("            </OOBE>\n");

    // FirstLogonCommands — all the tweaks run here after first login
    let first_logon = build_first_logon_commands(config);
    if !first_logon.is_empty() {
        xml.push_str("            <FirstLogonCommands>\n");
        xml.push_str(&first_logon);
        xml.push_str("            </FirstLogonCommands>\n");
    }

    xml.push_str("        </component>\n");

    // International settings for oobeSystem pass
    xml.push_str(r#"        <component name="Microsoft-Windows-International-Core" processorArchitecture="amd64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#);
    xml.push('\n');
    xml.push_str(&format!("            <InputLocale>{}</InputLocale>\n", escape_xml(&config.language)));
    xml.push_str(&format!("            <SystemLocale>{}</SystemLocale>\n", escape_xml(&config.language)));
    xml.push_str(&format!("            <UILanguage>{}</UILanguage>\n", escape_xml(&config.language)));
    xml.push_str(&format!("            <UserLocale>{}</UserLocale>\n", escape_xml(&config.language)));
    xml.push_str("        </component>\n");

    xml.push_str("    </settings>\n");

    // Close the root element
    xml.push_str("</unattend>\n");

    println!("[Deploy] Generated autounattend.xml ({} bytes)", xml.len());
    xml
}

/// Build the <FirstLogonCommands> section from config tweak toggles.
/// Each enabled tweak adds a <SynchronousCommand> with an incrementing Order number.
/// Commands are organized by category: Privacy → Security → Performance → UI → Bloatware → Domain.
fn build_first_logon_commands(config: &DeployConfig) -> String {
    let mut commands = String::new();
    let mut order: u32 = 1; // Order number for each command (must be unique)

    // ============================================
    // PRIVACY & TELEMETRY
    // ============================================
    if config.disable_telemetry {
        add_reg_command(&mut commands, &mut order,"Disable Telemetry",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
            "AllowTelemetry", "REG_DWORD", "0");
        add_reg_command(&mut commands, &mut order,"Disable Telemetry (user)",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Privacy",
            "TailoredExperiencesWithDiagnosticDataEnabled", "REG_DWORD", "0");
    }

    if config.disable_location {
        add_reg_command(&mut commands, &mut order,"Disable Location Tracking",
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location",
            "Value", "REG_SZ", "Deny");
    }

    if config.disable_ads {
        add_reg_command(&mut commands, &mut order,"Disable Advertising ID",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
            "Enabled", "REG_DWORD", "0");
    }

    if config.disable_suggested_apps {
        add_reg_command(&mut commands, &mut order,"Disable Suggested Apps",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SubscribedContent-338388Enabled", "REG_DWORD", "0");
        add_reg_command(&mut commands, &mut order,"Disable Suggested Apps (2)",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SubscribedContent-338389Enabled", "REG_DWORD", "0");
        add_reg_command(&mut commands, &mut order,"Disable Suggested Apps (3)",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SystemPaneSuggestionsEnabled", "REG_DWORD", "0");
    }

    if config.disable_bing_search {
        add_reg_command(&mut commands, &mut order,"Disable Bing Search in Start",
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\Explorer",
            "DisableSearchBoxSuggestions", "REG_DWORD", "1");
    }

    if config.disable_smartscreen {
        add_reg_command(&mut commands, &mut order,"Disable SmartScreen",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
            "EnableSmartScreen", "REG_DWORD", "0");
    }

    // ============================================
    // SECURITY
    // ============================================
    if config.enable_rdp {
        add_reg_command(&mut commands, &mut order,"Enable RDP",
            r"HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server",
            "fDenyTSConnections", "REG_DWORD", "0");
        add_raw_command(&mut commands, &mut order, "Allow RDP through firewall",
            "netsh advfirewall firewall set rule group=&quot;Remote Desktop&quot; new enable=Yes");
    }

    if config.disable_uac {
        add_reg_command(&mut commands, &mut order,"Disable UAC",
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
            "EnableLUA", "REG_DWORD", "0");
    }

    if config.disable_defender {
        add_reg_command(&mut commands, &mut order,"Disable Windows Defender",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows Defender",
            "DisableAntiSpyware", "REG_DWORD", "1");
        add_reg_command(&mut commands, &mut order,"Disable Real-Time Protection",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows Defender\Real-Time Protection",
            "DisableRealtimeMonitoring", "REG_DWORD", "1");
    }

    if config.disable_firewall {
        add_raw_command(&mut commands, &mut order, "Disable Firewall (Domain)",
            "netsh advfirewall set domainprofile state off");
        add_raw_command(&mut commands, &mut order, "Disable Firewall (Private)",
            "netsh advfirewall set privateprofile state off");
        add_raw_command(&mut commands, &mut order, "Disable Firewall (Public)",
            "netsh advfirewall set publicprofile state off");
    }

    if config.disable_vbs {
        add_reg_command(&mut commands, &mut order,"Disable VBS/Core Isolation",
            r"HKLM\SYSTEM\CurrentControlSet\Control\DeviceGuard",
            "EnableVirtualizationBasedSecurity", "REG_DWORD", "0");
    }

    if config.disable_bitlocker {
        add_reg_command(&mut commands, &mut order,"Disable BitLocker Auto-Encryption",
            r"HKLM\SYSTEM\CurrentControlSet\Control\BitLocker",
            "PreventDeviceEncryption", "REG_DWORD", "1");
    }

    // ============================================
    // PERFORMANCE
    // ============================================
    if config.disable_fast_startup {
        add_reg_command(&mut commands, &mut order,"Disable Fast Startup",
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
            "HiberbootEnabled", "REG_DWORD", "0");
    }

    if config.high_performance {
        add_raw_command(&mut commands, &mut order, "Set High Performance Power Plan",
            "powercfg /setactive 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c");
    }

    if config.disable_system_restore {
        add_reg_command(&mut commands, &mut order,"Disable System Restore",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows NT\SystemRestore",
            "DisableSR", "REG_DWORD", "1");
    }

    // ============================================
    // UI CUSTOMIZATION
    // ============================================
    if config.show_file_extensions {
        add_reg_command(&mut commands, &mut order,"Show File Extensions",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "HideFileExt", "REG_DWORD", "0");
    }

    if config.show_hidden_files {
        add_reg_command(&mut commands, &mut order,"Show Hidden Files",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "Hidden", "REG_DWORD", "1");
    }

    if config.classic_context_menu {
        add_reg_command(&mut commands, &mut order,"Classic Context Menu (Win11)",
            r"HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32",
            "", "REG_SZ", "");
    }

    if config.taskbar_search_mode > 0 {
        // 0=show, 1=icon, 2=hidden
        add_reg_command(&mut commands, &mut order,"Configure Taskbar Search",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
            "SearchboxTaskbarMode", "REG_DWORD", &config.taskbar_search_mode.to_string());
    }

    if config.hide_task_view {
        add_reg_command(&mut commands, &mut order,"Hide Task View Button",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "ShowTaskViewButton", "REG_DWORD", "0");
    }

    if config.hide_widgets {
        add_reg_command(&mut commands, &mut order,"Hide Widgets Button",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarDa", "REG_DWORD", "0");
    }

    if config.taskbar_left_align {
        add_reg_command(&mut commands, &mut order,"Left-align Taskbar (Win11)",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarAl", "REG_DWORD", "0");
    }

    // ============================================
    // BLOATWARE REMOVAL
    // ============================================
    if config.disable_cortana {
        add_reg_command(&mut commands, &mut order,"Disable Cortana",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
            "AllowCortana", "REG_DWORD", "0");
    }

    if config.disable_onedrive {
        add_reg_command(&mut commands, &mut order,"Disable OneDrive",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\OneDrive",
            "DisableFileSyncNGSC", "REG_DWORD", "1");
    }

    if config.disable_teams {
        add_reg_command(&mut commands, &mut order,"Disable Teams Chat",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Chat",
            "ChatIcon", "REG_DWORD", "3");
    }

    if config.disable_copilot {
        add_reg_command(&mut commands, &mut order,"Disable Copilot",
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot",
            "TurnOffWindowsCopilot", "REG_DWORD", "1");
    }

    if config.disable_widgets_service {
        add_reg_command(&mut commands, &mut order,"Disable Widgets Service",
            r"HKLM\SOFTWARE\Policies\Microsoft\Dsh",
            "AllowNewsAndInterests", "REG_DWORD", "0");
    }

    // ============================================
    // DOMAIN JOIN
    // ============================================
    if config.join_domain && !config.domain_name.is_empty() {
        // Build PowerShell command to join the domain
        let mut ps_cmd = format!(
            "Add-Computer -DomainName '{}' -Credential (New-Object PSCredential('{}', (ConvertTo-SecureString '{}' -AsPlainText -Force)))",
            config.domain_name.replace('\'', "''"),
            config.domain_username.replace('\'', "''"),
            config.domain_password.replace('\'', "''")
        );

        // Force restart after domain join
        ps_cmd.push_str(" -Restart -Force");

        add_ps_command(&mut commands, &mut order, "Join Domain", &ps_cmd);
    }

    // ============================================
    // ALWAYS: Set password to never expire
    // ============================================
    add_raw_command(&mut commands, &mut order,
        "Set password to never expire",
        "net accounts /maxpwage:unlimited");

    // ============================================
    // POST-INSTALL SCRIPTS (if any exist)
    // ============================================
    // If the user added FirstLogon scripts, add a final command that
    // runs RunAll.bat (which executes each script in order with logging).
    // The scripts + RunAll.bat are copied to C:\Temp\MasterBooter\ by
    // copy_scripts_to_target(false) during Step 7 of the deployment pipeline.
    // RunAll.bat logs all output to C:\Temp\MasterBooter\RunAll.log.
    let firstlogon_scripts = list_scripts("FirstLogon");
    if !firstlogon_scripts.is_empty() {
        add_raw_command(&mut commands, &mut order,
            "Run MasterBooter post-install scripts",
            r#"cmd /c "C:\Temp\MasterBooter\RunAll.bat""#);
    }

    commands
}

/// Escape special XML characters in a string.
/// Replaces: & < > " '
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Helper: Add a registry command to the FirstLogonCommands XML.
/// Builds: reg add "KEY" /v NAME /t TYPE /d DATA /f
fn add_reg_command(commands: &mut String, order: &mut u32,
    description: &str, key: &str, value: &str, reg_type: &str, data: &str)
{
    // RequiresUserInput=false tells Windows this command doesn't need
    // user interaction, preventing unnecessary delays during setup.
    commands.push_str(&format!(
        "                <SynchronousCommand wcm:action=\"add\">\n\
         \x20                   <Order>{}</Order>\n\
         \x20                   <CommandLine>reg add &quot;{}&quot; /v {} /t {} /d {} /f</CommandLine>\n\
         \x20                   <Description>{}</Description>\n\
         \x20                   <RequiresUserInput>false</RequiresUserInput>\n\
         \x20               </SynchronousCommand>\n",
        order,
        escape_xml(key),
        escape_xml(value),
        escape_xml(reg_type),
        escape_xml(data),
        escape_xml(description)
    ));
    *order += 1;
}

/// Helper: Add a raw command to the FirstLogonCommands XML.
fn add_raw_command(commands: &mut String, order: &mut u32, description: &str, command: &str) {
    commands.push_str(&format!(
        "                <SynchronousCommand wcm:action=\"add\">\n\
         \x20                   <Order>{}</Order>\n\
         \x20                   <CommandLine>{}</CommandLine>\n\
         \x20                   <Description>{}</Description>\n\
         \x20                   <RequiresUserInput>false</RequiresUserInput>\n\
         \x20               </SynchronousCommand>\n",
        order,
        escape_xml(command),
        escape_xml(description)
    ));
    *order += 1;
}

/// Helper: Add a PowerShell command to the FirstLogonCommands XML.
fn add_ps_command(commands: &mut String, order: &mut u32, description: &str, ps_command: &str) {
    // Use -ExecutionPolicy Bypass so PowerShell scripts run regardless of
    // the system's execution policy (default is Restricted = blocks all .ps1)
    commands.push_str(&format!(
        "                <SynchronousCommand wcm:action=\"add\">\n\
         \x20                   <Order>{}</Order>\n\
         \x20                   <CommandLine>powershell -ExecutionPolicy Bypass -NoProfile -Command &quot;{}&quot;</CommandLine>\n\
         \x20                   <Description>{}</Description>\n\
         \x20                   <RequiresUserInput>false</RequiresUserInput>\n\
         \x20               </SynchronousCommand>\n",
        order,
        escape_xml(ps_command),
        escape_xml(description)
    ));
    *order += 1;
}

// ============================================
// WIN11 BYPASS
// ============================================

/// Apply Windows 11 hardware requirements bypass via registry keys.
/// Sets 7 keys that allow installing Win11 on unsupported hardware:
/// - TPM 2.0 check bypass
/// - Secure Boot check bypass
/// - CPU compatibility check bypass
/// - RAM requirement bypass
/// - Storage requirement bypass
/// - Upgrade with unsupported TPM/CPU
/// - OOBE network requirement bypass (BypassNRO)
///
/// # Returns
/// * `Ok(())` — all keys set successfully
/// * `Err(String)` — error message if any key fails
pub fn apply_win11_bypass() -> Result<(), String> {
    println!("[Deploy] Applying Windows 11 bypass registry keys...");

    // List of registry keys to set
    // Format: (key_path, value_name, value_data)
    let bypass_keys: &[(&str, &str, &str)] = &[
        (r"HKLM\SYSTEM\Setup\LabConfig", "BypassSecureBootCheck", "1"),
        (r"HKLM\SYSTEM\Setup\LabConfig", "BypassTPMCheck", "1"),
        (r"HKLM\SYSTEM\Setup\LabConfig", "BypassCPUCheck", "1"),
        (r"HKLM\SYSTEM\Setup\LabConfig", "BypassRAMCheck", "1"),
        (r"HKLM\SYSTEM\Setup\LabConfig", "BypassStorageCheck", "1"),
        (r"HKLM\SYSTEM\Setup\MoSetup", "AllowUpgradesWithUnsupportedTPMOrCPU", "1"),
        (r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE", "BypassNRO", "1"),
    ];

    let mut errors: Vec<String> = Vec::new();

    for (key_path, value_name, value_data) in bypass_keys {
        let output = Command::new("reg")
            .args(["add", key_path, "/v", value_name, "/t", "REG_DWORD", "/d", value_data, "/f"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                println!("  Set {}\\{} = {}", key_path, value_name, value_data);
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                errors.push(format!("{}\\{}: {}", key_path, value_name, err.trim()));
            }
            Err(e) => {
                errors.push(format!("{}\\{}: {}", key_path, value_name, e));
            }
        }
    }

    if errors.is_empty() {
        println!("[Deploy] All 7 bypass keys set successfully");
        Ok(())
    } else {
        Err(format!("Some bypass keys failed: {}", errors.join("; ")))
    }
}

// ============================================
// DISK FORMATTING
// ============================================

/// Pre-format a disk with diskpart before running Windows Setup.
/// This avoids the error 0x80030024 that can happen when Setup tries
/// to format a disk that's in use.
///
/// Creates partition layout based on boot mode:
/// - UEFI: EFI(100MB, FAT32) + MSR(16MB) + Primary(rest, NTFS)
/// - BIOS: System Reserved(100MB, NTFS, active) + Primary(rest, NTFS)
///
/// # Arguments
/// * `disk_id` — Disk number to format (from detect_disks)
/// * `boot_mode` — UEFI or BIOS
///
/// # Returns
/// * `Ok(())` — disk formatted successfully
/// * `Err(String)` — error with details
pub fn format_disk_with_diskpart(disk_id: i32, boot_mode: &BootMode) -> Result<(), String> {
    println!("[Deploy] Formatting Disk {} as {:?}...", disk_id, boot_mode);

    // Build the diskpart script
    let script = match boot_mode {
        BootMode::UEFI => {
            format!(
                "select disk {}\n\
                 clean\n\
                 convert gpt\n\
                 create partition efi size=100\n\
                 format quick fs=fat32 label=\"System\"\n\
                 assign letter=S\n\
                 create partition msr size=16\n\
                 create partition primary\n\
                 format quick fs=ntfs label=\"Windows\"\n\
                 assign letter=C\n\
                 exit\n",
                disk_id
            )
        }
        BootMode::BIOS => {
            format!(
                "select disk {}\n\
                 clean\n\
                 create partition primary size=100\n\
                 format quick fs=ntfs label=\"System Reserved\"\n\
                 active\n\
                 assign letter=S\n\
                 create partition primary\n\
                 format quick fs=ntfs label=\"Windows\"\n\
                 assign letter=C\n\
                 exit\n",
                disk_id
            )
        }
    };

    // Write the script to a temp file
    let temp_dir = std::env::temp_dir();
    let script_path = temp_dir.join("mb_format_disk.txt");
    fs::write(&script_path, &script)
        .map_err(|e| format!("Failed to write diskpart script: {}", e))?;

    // Run diskpart with the script
    let output = Command::new("diskpart")
        .args(["/s", &script_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("Failed to run diskpart: {}", e))?;

    // Clean up the temp script
    let _ = fs::remove_file(&script_path);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        println!("[Deploy] Disk {} formatted successfully", disk_id);
        println!("[Deploy] diskpart output:\n{}", stdout);
        Ok(())
    } else {
        Err(format!(
            "diskpart failed (exit code {}): {}\n{}",
            output.status.code().unwrap_or(-1),
            stdout,
            stderr
        ))
    }
}

// ============================================
// SETUP LAUNCH
// ============================================

/// Find setup.exe on available drives.
/// Scans all drive letters (D: through Z:) for sources\setup.exe.
/// This is the standard location on Windows installation media.
///
/// # Returns
/// * `Ok(PathBuf)` — path to setup.exe
/// * `Err(String)` — not found on any drive
pub fn find_setup_exe() -> Result<PathBuf, String> {
    println!("[Deploy] Searching for Windows Setup (setup.exe)...");

    // Scan drives D: through Z: for sources\setup.exe
    for letter in b'D'..=b'Z' {
        let drive = format!("{}:", letter as char);
        let setup_path = PathBuf::from(&drive).join("sources").join("setup.exe");
        if setup_path.exists() {
            println!("[Deploy] Found setup.exe at: {}", setup_path.display());
            return Ok(setup_path);
        }
    }

    // Also check X: (WinPE RAM drive — setup might be here)
    let x_setup = PathBuf::from(r"X:\sources\setup.exe");
    if x_setup.exists() {
        println!("[Deploy] Found setup.exe at: {}", x_setup.display());
        return Ok(x_setup);
    }

    Err("setup.exe not found on any drive. Make sure the Windows installation media is accessible.".to_string())
}

/// Launch Windows Setup with the generated autounattend.xml.
/// Runs: setup.exe /noreboot /unattend:<xml_path>
///
/// The /noreboot flag prevents automatic reboot so we can copy
/// post-install scripts before the first real boot.
///
/// # Arguments
/// * `xml_path` — Path to the generated autounattend.xml file
///
/// # Returns
/// * `Ok(Child)` — the running setup.exe process
/// * `Err(String)` — error if setup.exe can't be started
pub fn launch_setup(xml_path: &Path) -> Result<std::process::Child, String> {
    let setup_path = find_setup_exe()?;

    println!("[Deploy] Launching Windows Setup...");
    println!("[Deploy]   setup.exe: {}", setup_path.display());
    println!("[Deploy]   unattend: {}", xml_path.display());

    let child = Command::new(&setup_path)
        .args([
            "/noreboot",
            &format!("/unattend:{}", xml_path.display()),
        ])
        .spawn()
        .map_err(|e| format!("Failed to launch setup.exe: {}", e))?;

    println!("[Deploy] setup.exe launched (PID: {})", child.id());
    Ok(child)
}

// ============================================
// EXECUTION PIPELINE
// ============================================

/// Execute the full deployment pipeline with progress callbacks.
///
/// Steps:
/// 1. Validate config (0-5%)
/// 2. Format disk with diskpart (5-15%) — if disk_id >= 0
/// 3. Apply Win11 bypass registry keys (15-20%) — if enabled
/// 4. Generate autounattend.xml (20-30%)
/// 5. Write XML to temp file (30-35%)
/// 6. Launch setup.exe and wait (35-90%)
/// 7. Post-install scripts (90-95%) — future
/// 8. Reboot (95-100%)
///
/// # Arguments
/// * `config` — The complete deployment configuration
/// * `progress_fn` — Called with (percent: i32, status: &str) for UI updates
///
/// # Returns
/// DeployResult with success/failure info
pub fn execute(
    config: &DeployConfig,
    progress_fn: impl Fn(i32, &str) + Send + 'static,
) -> DeployResult {
    // ============================================
    // STEP 1: Validate (0-5%)
    // ============================================
    progress_fn(0, "Validating deployment configuration...");

    // Check that a WIM file is selected
    if config.wim_path.as_os_str().is_empty() {
        return DeployResult {
            success: false,
            message: "No Windows image selected. Please browse for an install.wim or install.esd file.".to_string(),
        };
    }

    // Check edition is selected
    if config.edition.is_empty() {
        return DeployResult {
            success: false,
            message: "No Windows edition selected. Please select an edition from the image.".to_string(),
        };
    }

    // Check DISM is available (needed for setup)
    let dism_check = Command::new("dism.exe").args(["/?"])
        .output();
    if dism_check.is_err() {
        return DeployResult {
            success: false,
            message: "DISM is not available. Are you running from WinPE or a Windows environment?".to_string(),
        };
    }

    progress_fn(5, "Configuration validated");

    // ============================================
    // STEP 2: Format disk (5-15%)
    // ============================================
    if config.disk_id >= 0 {
        progress_fn(5, &format!("Formatting Disk {} ({})...", config.disk_id, config.boot_mode));

        if let Err(e) = format_disk_with_diskpart(config.disk_id, &config.boot_mode) {
            return DeployResult {
                success: false,
                message: format!("Disk formatting failed: {}", e),
            };
        }

        progress_fn(15, "Disk formatted successfully");
    } else {
        progress_fn(15, "Skipping disk format (Windows will choose)");
    }

    // ============================================
    // STEP 3: Win11 Bypass (15-20%)
    // ============================================
    if config.bypass_win11 {
        progress_fn(15, "Applying Windows 11 bypass...");

        if let Err(e) = apply_win11_bypass() {
            // Don't fail the whole deployment — bypass is optional
            println!("[Deploy] Warning: Win11 bypass partially failed: {}", e);
        }

        progress_fn(20, "Windows 11 bypass applied");
    } else {
        progress_fn(20, "Windows 11 bypass not needed");
    }

    // ============================================
    // STEP 4: Generate XML (20-30%)
    // ============================================
    progress_fn(20, "Generating autounattend.xml...");

    let xml = generate_autounattend(config);

    progress_fn(30, "autounattend.xml generated");

    // ============================================
    // STEP 5: Write XML to temp file (30-35%)
    // ============================================
    progress_fn(30, "Writing autounattend.xml...");

    let temp_dir = std::env::temp_dir();
    let xml_path = temp_dir.join("autounattend.xml");

    if let Err(e) = fs::write(&xml_path, &xml) {
        return DeployResult {
            success: false,
            message: format!("Failed to write autounattend.xml: {}", e),
        };
    }

    println!("[Deploy] Wrote autounattend.xml to: {}", xml_path.display());
    progress_fn(35, "autounattend.xml written");

    // ============================================
    // STEP 6: Launch setup.exe (35-90%)
    // ============================================
    progress_fn(35, "Launching Windows Setup...");

    let mut child = match launch_setup(&xml_path) {
        Ok(c) => c,
        Err(e) => {
            return DeployResult {
                success: false,
                message: format!("Failed to launch setup: {}", e),
            };
        }
    };

    progress_fn(40, "Windows Setup is running... This will take a while.");

    // Wait for setup.exe to complete
    // This blocks for a LONG time (15-45 minutes depending on hardware)
    match child.wait() {
        Ok(status) => {
            if status.success() {
                println!("[Deploy] setup.exe completed successfully");
                progress_fn(90, "Windows Setup completed");
            } else {
                let code = status.code().unwrap_or(-1);
                // Exit code 0x80004005 often means the user cancelled
                return DeployResult {
                    success: false,
                    message: format!("Windows Setup exited with code: 0x{:X}", code),
                };
            }
        }
        Err(e) => {
            return DeployResult {
                success: false,
                message: format!("Failed to wait for setup.exe: {}", e),
            };
        }
    }

    // ============================================
    // STEP 7: Post-install scripts (90-95%)
    // ============================================
    // Copy any user-added FirstLogon scripts to the newly installed Windows.
    // In Automated mode, the autounattend.xml already has <FirstLogonCommands>
    // that will trigger RunAll.bat — so we pass is_normal_mode=false.
    progress_fn(90, "Copying post-install scripts to target...");
    match copy_scripts_to_target(false) {
        Ok(()) => {
            progress_fn(93, "Post-install scripts copied successfully");
        }
        Err(e) => {
            // Non-fatal: the Windows installation itself succeeded,
            // scripts were just not copied (they won't auto-run)
            println!("[Deploy] Warning: Script copy failed: {}", e);
            progress_fn(93, &format!("Warning: Script copy issue: {}", e));
        }
    }

    progress_fn(95, "Post-install step complete");

    // ============================================
    // STEP 8: Reboot (95-100%)
    // ============================================
    progress_fn(95, "Preparing to reboot...");

    // Try standard reboot first
    let reboot_result = Command::new("shutdown")
        .args(["/r", "/t", "5", "/f", "/c", "MasterBooter: Windows deployment complete, rebooting..."])
        .output();

    match reboot_result {
        Ok(out) if out.status.success() => {
            progress_fn(100, "Rebooting in 5 seconds...");
        }
        _ => {
            // Try WinPE reboot command as fallback
            let _ = Command::new("wpeutil").args(["reboot"]).output();
            progress_fn(100, "Reboot initiated");
        }
    }

    DeployResult {
        success: true,
        message: "Windows deployment complete! System is rebooting.".to_string(),
    }
}

// ============================================
// PROFILE SAVE/LOAD
// ============================================

/// Get the profiles directory (next to the EXE).
/// Creates the directory if it doesn't exist.
fn get_profiles_dir() -> PathBuf {
    // Find the EXE directory
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let profiles_dir = exe_dir.join("profiles");

    // Create the directory if needed
    if !profiles_dir.exists() {
        let _ = fs::create_dir_all(&profiles_dir);
    }

    profiles_dir
}

/// Save a DeployConfig to a named JSON profile.
/// The profile is stored in profiles/<name>.json next to the EXE.
/// Session-specific fields (wim_path, edition, edition_index) are cleared
/// before saving — they don't make sense to persist.
///
/// # Arguments
/// * `name` — Profile name (used as filename, sanitized)
/// * `config` — The deployment configuration to save
pub fn save_profile(name: &str, config: &DeployConfig) -> Result<(), String> {
    let profiles_dir = get_profiles_dir();

    // Sanitize the filename (remove path separators and other dangerous chars)
    let safe_name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .collect();

    if safe_name.is_empty() {
        return Err("Profile name cannot be empty".to_string());
    }

    // Clone config and clear session-specific fields
    let mut profile_config = config.clone();
    profile_config.wim_path = PathBuf::new();
    profile_config.edition = String::new();
    profile_config.edition_index = 0;

    // Serialize to pretty JSON
    let json = serde_json::to_string_pretty(&profile_config)
        .map_err(|e| format!("Failed to serialize profile: {}", e))?;

    // Write to file
    let file_path = profiles_dir.join(format!("{}.json", safe_name));
    fs::write(&file_path, json)
        .map_err(|e| format!("Failed to write profile: {}", e))?;

    println!("[Deploy] Saved profile '{}' to: {}", safe_name, file_path.display());
    Ok(())
}

/// Load a DeployConfig from a named JSON profile.
/// The wim_path, edition, and edition_index fields will be empty
/// (they are session-specific and not saved in profiles).
///
/// # Arguments
/// * `name` — Profile name to load
///
/// # Returns
/// * `Ok(DeployConfig)` — the loaded configuration
/// * `Err(String)` — error if file not found or invalid JSON
pub fn load_profile(name: &str) -> Result<DeployConfig, String> {
    let profiles_dir = get_profiles_dir();
    let file_path = profiles_dir.join(format!("{}.json", name));

    if !file_path.exists() {
        return Err(format!("Profile '{}' not found", name));
    }

    let json = fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read profile: {}", e))?;

    let config: DeployConfig = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse profile: {}", e))?;

    println!("[Deploy] Loaded profile '{}' from: {}", name, file_path.display());
    Ok(config)
}

/// List all saved profile names.
/// Scans the profiles/ directory for .json files.
///
/// # Returns
/// Vector of profile names (without .json extension), sorted alphabetically
pub fn list_profiles() -> Vec<String> {
    let profiles_dir = get_profiles_dir();

    let mut names: Vec<String> = Vec::new();

    if let Ok(entries) = fs::read_dir(&profiles_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                if let Some(stem) = path.file_stem() {
                    names.push(stem.to_string_lossy().to_string());
                }
            }
        }
    }

    names.sort();
    names
}

/// Delete a named profile.
///
/// # Arguments
/// * `name` — Profile name to delete
///
/// # Returns
/// * `Ok(())` — profile deleted
/// * `Err(String)` — error if file not found
pub fn delete_profile(name: &str) -> Result<(), String> {
    let profiles_dir = get_profiles_dir();
    let file_path = profiles_dir.join(format!("{}.json", name));

    if !file_path.exists() {
        return Err(format!("Profile '{}' not found", name));
    }

    fs::remove_file(&file_path)
        .map_err(|e| format!("Failed to delete profile: {}", e))?;

    println!("[Deploy] Deleted profile '{}'", name);
    Ok(())
}

// ============================================
// FILE DIALOGS
// ============================================

/// Load a deploy profile from an arbitrary file path (for importing).
/// This lets users pick a .json profile file from anywhere on disk.
///
/// # Arguments
/// * `path` — Full path to the .json profile file
///
/// # Returns
/// * `Ok(DeployConfig)` — the loaded configuration
/// * `Err(String)` — error if file not found or invalid JSON
pub fn load_profile_from_path(path: &Path) -> Result<DeployConfig, String> {
    if !path.exists() {
        return Err(format!("Profile file not found: {}", path.display()));
    }

    let json = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read profile: {}", e))?;

    let config: DeployConfig = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse profile: {}", e))?;

    println!("[Deploy] Imported profile from: {}", path.display());
    Ok(config)
}

/// Open a file picker dialog for selecting a Windows image file.
/// Allows selecting .wim, .esd, or .iso files.
///
/// # Returns
/// * `Some(PathBuf)` — the selected file path
/// * `None` — user cancelled the dialog
pub fn pick_image_file() -> Option<PathBuf> {
    let dialog = rfd::FileDialog::new()
        .set_title("Select Windows Image")
        .add_filter("Windows Images", &["wim", "esd", "iso"])
        .add_filter("All Files", &["*"]);

    dialog.pick_file()
}

/// Open a file picker dialog for importing a deploy profile (.json).
///
/// # Returns
/// * `Some(PathBuf)` — the selected profile file path
/// * `None` — user cancelled the dialog
pub fn pick_profile_file() -> Option<PathBuf> {
    // Start in the profiles directory if it exists, otherwise current dir
    let profiles_dir = get_profiles_dir();
    let mut dialog = rfd::FileDialog::new()
        .set_title("Import Deploy Profile")
        .add_filter("JSON Profiles", &["json"])
        .add_filter("All Files", &["*"]);

    if profiles_dir.exists() {
        dialog = dialog.set_directory(&profiles_dir);
    }

    dialog.pick_file()
}

/// Open a file picker dialog for selecting a script file to add.
///
/// # Returns
/// * `Some(PathBuf)` — the selected script file path
/// * `None` — user cancelled the dialog
pub fn pick_script_file() -> Option<PathBuf> {
    let dialog = rfd::FileDialog::new()
        .set_title("Select Script")
        .add_filter("Scripts", &["ps1", "bat", "cmd", "exe", "reg", "vbs"])
        .add_filter("All Files", &["*"]);

    dialog.pick_file()
}

// ============================================
// SCRIPT MANAGEMENT
// ============================================
// Post-install scripts are stored in a FirstLogon/ folder next to the EXE.
// These scripts run after the first user logs into the newly installed Windows.
//
// How scripts are triggered depends on the install mode:
//   - Automated mode: autounattend.xml <FirstLogonCommands> calls RunAll.bat
//   - Normal mode: RunOnce registry key is injected into the offline hive
//
// We do NOT use SetupComplete.cmd because Microsoft disables it for OEM product
// keys (except Enterprise/Server editions). FirstLogonCommands works with ALL
// key types, making it the reliable choice.

/// Get the path to the FirstLogon script folder next to the EXE.
/// Creates the folder if it doesn't exist.
fn get_scripts_dir(script_type: &str) -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let dir = exe_dir.join(script_type);
    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
    }
    dir
}

/// List all script files in the FirstLogon folder.
/// Scans for supported file types (.ps1, .bat, .cmd, .exe, .reg, .vbs).
///
/// # Arguments
/// * `script_type` — Currently only "FirstLogon" (kept generic for future use)
///
/// # Returns
/// Sorted vector of script filenames (just names, not full paths)
pub fn list_scripts(script_type: &str) -> Vec<String> {
    let dir = get_scripts_dir(script_type);
    let mut names: Vec<String> = Vec::new();

    // Supported script file extensions
    let valid_exts = ["ps1", "bat", "cmd", "exe", "reg", "vbs"];

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                // Only include files with valid script extensions
                let ext = path.extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if valid_exts.contains(&ext.as_str()) {
                    if let Some(name) = path.file_name() {
                        names.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    names.sort();
    names
}

/// Add a script to the FirstLogon folder by copying it from a source path.
/// Creates the folder if needed.
///
/// # Arguments
/// * `script_type` — Currently only "FirstLogon"
/// * `source_path` — Full path to the script file to copy
///
/// # Returns
/// * `Ok(())` — script copied successfully
/// * `Err(String)` — error message
pub fn add_script(script_type: &str, source_path: &Path) -> Result<(), String> {
    let dir = get_scripts_dir(script_type);
    let filename = source_path.file_name()
        .ok_or_else(|| "Invalid file path".to_string())?;
    let dest = dir.join(filename);

    fs::copy(source_path, &dest)
        .map_err(|e| format!("Failed to copy script: {}", e))?;

    println!("[Deploy] Added {} script: {}", script_type, dest.display());
    Ok(())
}

/// Remove a script from the FirstLogon folder.
///
/// # Arguments
/// * `script_type` — Currently only "FirstLogon"
/// * `filename` — Just the filename (not full path) to delete
///
/// # Returns
/// * `Ok(())` — script deleted
/// * `Err(String)` — error message
pub fn remove_script(script_type: &str, filename: &str) -> Result<(), String> {
    let dir = get_scripts_dir(script_type);
    let path = dir.join(filename);

    if !path.exists() {
        return Err(format!("Script '{}' not found in {}/", filename, script_type));
    }

    fs::remove_file(&path)
        .map_err(|e| format!("Failed to delete script: {}", e))?;

    println!("[Deploy] Removed {} script: {}", script_type, filename);
    Ok(())
}

// ============================================
// NORMAL (INTERACTIVE) INSTALL
// ============================================

/// Execute the normal (interactive) install pipeline.
/// Simpler than automated: just find and launch setup.exe, then copy scripts.
///
/// # Arguments
/// * `progress_fn` — Callback for progress updates (percentage, message)
///
/// # Returns
/// * `DeployResult` — success or failure with message
pub fn normal_execute(
    progress_fn: impl Fn(i32, &str) + Send + 'static,
) -> DeployResult {
    println!("[Deploy] Starting NORMAL (interactive) installation...");

    // Step 1: Find setup.exe
    progress_fn(5, "Looking for Windows Setup...");
    let setup_path = match find_setup_exe() {
        Ok(path) => {
            println!("[Deploy] Found setup.exe at: {}", path.display());
            path
        }
        Err(e) => {
            return DeployResult {
                success: false,
                message: format!("Cannot find setup.exe: {}. Make sure a Windows ISO is mounted or available.", e),
            };
        }
    };

    // Step 2: Launch setup.exe interactively (NO /unattend:)
    progress_fn(15, "Launching Windows Setup (interactive)...");
    let mut child = match Command::new(&setup_path)
        .arg("/noreboot")
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return DeployResult {
                success: false,
                message: format!("Failed to launch setup.exe: {}", e),
            };
        }
    };

    // Step 3: Wait for setup to complete (this takes 15-45 minutes)
    progress_fn(20, "Windows Setup is running — follow the on-screen prompts...");
    match child.wait() {
        Ok(status) => {
            if !status.success() {
                let code = status.code().unwrap_or(-1);
                return DeployResult {
                    success: false,
                    message: format!("Windows Setup exited with error code: {} (0x{:X})", code, code),
                };
            }
        }
        Err(e) => {
            return DeployResult {
                success: false,
                message: format!("Failed to wait for setup.exe: {}", e),
            };
        }
    }

    // Step 4: Copy scripts to target (if any exist)
    // In Normal mode there's no autounattend.xml, so we pass is_normal_mode=true
    // to inject a RunOnce registry key that triggers RunAll.bat on first logon.
    progress_fn(90, "Copying post-install scripts...");
    if let Err(e) = copy_scripts_to_target(true) {
        println!("[Deploy] Warning: Script copy failed: {}", e);
        // Non-fatal — installation itself succeeded
    }

    // Step 5: Reboot
    progress_fn(95, "Rebooting...");
    let _ = Command::new("shutdown")
        .args(["/r", "/t", "5", "/f", "/c", "MasterBooter: Installation complete, rebooting..."])
        .spawn();
    // Fallback for WinPE
    let _ = Command::new("wpeutil").arg("reboot").spawn();

    progress_fn(100, "Complete!");
    DeployResult {
        success: true,
        message: "Normal installation complete. System will reboot shortly.".to_string(),
    }
}

// ============================================
// POST-INSTALL SCRIPT COPYING
// ============================================

/// Copy FirstLogon scripts to the newly installed Windows.
/// Called after setup.exe completes (both Normal and Automated modes).
///
/// Scripts are copied to C:\Temp\MasterBooter\ on the target drive, and a
/// RunAll.bat is generated that executes each script in order with full logging.
///
/// For **Automated mode**: The autounattend.xml already has a <FirstLogonCommands>
/// entry that calls RunAll.bat — no extra work needed here.
///
/// For **Normal mode**: There's no autounattend.xml, so we inject a RunOnce
/// registry key into the target's offline SOFTWARE hive. This causes Windows
/// to run RunAll.bat the first time any user logs in.
///
/// # Arguments
/// * `is_normal_mode` — true for Normal install, false for Automated install.
///   Normal mode needs the RunOnce registry injection since there's no answer file.
///
/// Finds the target drive by scanning for recently modified Windows installations.
pub fn copy_scripts_to_target(is_normal_mode: bool) -> Result<(), String> {
    let firstlogon_scripts = list_scripts("FirstLogon");

    // Nothing to copy?
    if firstlogon_scripts.is_empty() {
        println!("[Deploy] No FirstLogon scripts to copy — skipping");
        return Ok(());
    }

    // Find the newly installed Windows drive
    // Scan all drives (C: through Z:) for a recent Windows\System32\Config\SYSTEM file
    let target_drive = find_target_windows_drive()
        .ok_or_else(|| "Could not find newly installed Windows. Scripts not copied.".to_string())?;

    println!("[Deploy] Found target Windows at: {}\\", target_drive);

    // ============================================
    // Copy scripts to target drive
    // ============================================
    // Scripts go to <target>\Temp\MasterBooter\ — when the installed OS boots,
    // this becomes C:\Temp\MasterBooter\ (the system drive is always C: in the
    // running OS, even if it had a different letter in WinPE).
    let firstlogon_dir = get_scripts_dir("FirstLogon");
    let target_fl = PathBuf::from(format!("{}\\Temp\\MasterBooter", target_drive));
    let _ = fs::create_dir_all(&target_fl);

    for script_name in &firstlogon_scripts {
        let src = firstlogon_dir.join(script_name);
        let dst = target_fl.join(script_name);
        if let Err(e) = fs::copy(&src, &dst) {
            println!("[Deploy] Warning: Failed to copy script {}: {}", script_name, e);
        } else {
            println!("[Deploy] Copied script: {}", script_name);
        }
    }

    // ============================================
    // Create RunAll.bat with logging
    // ============================================
    // RunAll.bat executes each script in order, with full logging to a .log file
    // so the user can troubleshoot if anything fails. Each script invocation is
    // logged with a timestamp, and errors are captured but don't stop the batch.
    let log_file = r"C:\Temp\MasterBooter\RunAll.log";
    let mut bat_content = String::from("@echo off\r\n");
    bat_content.push_str("REM ============================================\r\n");
    bat_content.push_str("REM MasterBooter Post-Install Scripts\r\n");
    bat_content.push_str("REM This file was generated by MasterBooter.\r\n");
    bat_content.push_str("REM It runs all FirstLogon scripts in order.\r\n");
    bat_content.push_str("REM ============================================\r\n\r\n");

    // Log start time
    bat_content.push_str(&format!(
        "echo ============================================ >> \"{}\"\r\n", log_file));
    bat_content.push_str(&format!(
        "echo MasterBooter Scripts - Started: %DATE% %TIME% >> \"{}\"\r\n", log_file));
    bat_content.push_str(&format!(
        "echo ============================================ >> \"{}\"\r\n\r\n", log_file));

    // Execute each script with logging
    for script_name in &firstlogon_scripts {
        let ext = Path::new(script_name).extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // Log which script is running
        bat_content.push_str(&format!(
            "echo [%TIME%] Running: {} >> \"{}\"\r\n", script_name, log_file));

        match ext.as_str() {
            "ps1" => {
                // PowerShell: use -ExecutionPolicy Bypass so scripts always run
                // (default policy is Restricted which blocks all .ps1 files)
                bat_content.push_str(&format!(
                    "powershell.exe -ExecutionPolicy Bypass -NonInteractive -File \"%~dp0{}\" >> \"{}\" 2>&1\r\n",
                    script_name, log_file
                ));
            }
            "reg" => {
                // Registry files: import silently
                bat_content.push_str(&format!(
                    "reg import \"%~dp0{}\" >> \"{}\" 2>&1\r\n",
                    script_name, log_file
                ));
            }
            _ => {
                // Batch files, executables, VBS, etc: call them
                bat_content.push_str(&format!(
                    "call \"%~dp0{}\" >> \"{}\" 2>&1\r\n",
                    script_name, log_file
                ));
            }
        }

        // Log the result of each script
        bat_content.push_str(&format!(
            "echo [%TIME%] Finished: {} (exit code: %ERRORLEVEL%) >> \"{}\"\r\n",
            script_name, log_file
        ));
        bat_content.push_str(&format!("echo. >> \"{}\"\r\n\r\n", log_file));
    }

    // Log completion
    bat_content.push_str(&format!(
        "echo ============================================ >> \"{}\"\r\n", log_file));
    bat_content.push_str(&format!(
        "echo All scripts finished: %DATE% %TIME% >> \"{}\"\r\n", log_file));
    bat_content.push_str(&format!(
        "echo ============================================ >> \"{}\"\r\n", log_file));

    let runall_path = target_fl.join("RunAll.bat");
    if let Err(e) = fs::write(&runall_path, &bat_content) {
        return Err(format!("Failed to write RunAll.bat: {}", e));
    }
    println!("[Deploy] Created RunAll.bat with {} scripts (logging to {})",
        firstlogon_scripts.len(), log_file);

    // ============================================
    // Normal mode: Inject RunOnce registry key
    // ============================================
    // In Normal mode there's no autounattend.xml, so nothing would trigger
    // RunAll.bat. We fix this by loading the target's offline SOFTWARE registry
    // hive and adding a RunOnce key. Windows will run this command once the
    // first time ANY user logs in, then automatically delete the key.
    //
    // In Automated mode, the autounattend.xml <FirstLogonCommands> handles
    // triggering RunAll.bat, so we skip this step.
    if is_normal_mode {
        println!("[Deploy] Normal mode: injecting RunOnce registry key...");

        // Path to the target's SOFTWARE registry hive (offline)
        let hive_path = format!("{}\\Windows\\System32\\Config\\SOFTWARE", target_drive);
        // Temporary mount point in our current registry
        let temp_key = "HKLM\\TEMP_MASTERBOOTER";

        // Step 1: Load the offline hive into our registry
        let load_result = Command::new("reg")
            .args(["load", temp_key, &hive_path])
            .output();

        match load_result {
            Ok(out) if out.status.success() => {
                println!("[Deploy] Loaded target registry hive");

                // Step 2: Add RunOnce entry — runs cmd /c RunAll.bat on first logon
                // RunOnce keys are automatically deleted after they execute
                let runonce_key = format!(
                    "{}\\Microsoft\\Windows\\CurrentVersion\\RunOnce", temp_key);
                let add_result = Command::new("reg")
                    .args([
                        "add", &runonce_key,
                        "/v", "MasterBooterScripts",
                        "/t", "REG_SZ",
                        "/d", r#"cmd /c "C:\Temp\MasterBooter\RunAll.bat""#,
                        "/f",
                    ])
                    .output();

                match add_result {
                    Ok(out) if out.status.success() => {
                        println!("[Deploy] Added RunOnce key for MasterBooterScripts");
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        println!("[Deploy] Warning: Failed to add RunOnce key: {}", stderr);
                    }
                    Err(e) => {
                        println!("[Deploy] Warning: Failed to run reg add: {}", e);
                    }
                }

                // Step 3: Unload the hive (MUST do this or Windows can't boot!)
                let unload_result = Command::new("reg")
                    .args(["unload", temp_key])
                    .output();

                if let Ok(out) = &unload_result {
                    if out.status.success() {
                        println!("[Deploy] Unloaded target registry hive");
                    } else {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        println!("[Deploy] Warning: Failed to unload hive: {}", stderr);
                        // Try again after a short delay — sometimes the hive is still in use
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        let _ = Command::new("reg")
                            .args(["unload", temp_key])
                            .output();
                    }
                }
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                println!("[Deploy] Warning: Could not load target registry hive: {}", stderr);
                println!("[Deploy] Scripts were copied but won't auto-run. User can run RunAll.bat manually.");
            }
            Err(e) => {
                println!("[Deploy] Warning: reg command failed: {}", e);
                println!("[Deploy] Scripts were copied but won't auto-run. User can run RunAll.bat manually.");
            }
        }
    }

    println!("[Deploy] Script copying complete");
    Ok(())
}

/// Find the drive letter of a newly installed Windows.
/// Scans C: through Z: for Windows\System32\Config\SYSTEM file
/// and returns the drive with the most recently modified one.
fn find_target_windows_drive() -> Option<String> {
    let mut best_drive: Option<String> = None;
    let mut best_time: Option<std::time::SystemTime> = None;

    // Scan all drive letters (C: through Z:)
    for letter in b'C'..=b'Z' {
        let drive = format!("{}:", letter as char);
        let system_file = PathBuf::from(format!("{}\\Windows\\System32\\Config\\SYSTEM", drive));

        if system_file.exists() {
            // Check modification time — recently installed Windows will have a recent timestamp
            if let Ok(metadata) = fs::metadata(&system_file) {
                if let Ok(modified) = metadata.modified() {
                    // Use the most recently modified one
                    if best_time.is_none() || modified > best_time.unwrap() {
                        best_time = Some(modified);
                        best_drive = Some(drive);
                    }
                }
            }
        }
    }

    best_drive
}

// ============================================
// WINDOWS PRODUCT KEY DETECTION & BACKUP
// ============================================
// Detects the current Windows product key (OEM + installed) and saves it
// to a JSON file next to the EXE. This lets users backup their key before
// reinstalling, then load it in the deploy section.
//
// Two keys can exist on one machine:
//   - OEM key: embedded in BIOS/UEFI firmware by the manufacturer
//   - Installed key: the currently active key (may differ if user upgraded)

/// Information about detected Windows product keys.
/// Saved to saved_keys.json next to the EXE for cross-session persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsKeyInfo {
    /// OEM/BIOS key embedded in firmware (may be empty on non-OEM machines)
    pub oem_key: String,
    /// Currently installed/active product key (decoded from registry)
    pub installed_key: String,
    /// Windows edition (e.g., "Windows 11 Pro")
    pub edition: String,
    /// License status (e.g., "Licensed", "Notification", "Grace Period")
    pub status: String,
    /// Computer hostname (for reference — which machine was this backed up from?)
    pub hostname: String,
    /// Date the backup was taken (e.g., "2026-02-18")
    pub date: String,
}

/// Detect Windows product keys using PowerShell.
/// Retrieves both the OEM/BIOS key and the currently installed key.
///
/// The OEM key is read from the firmware (SoftwareLicensingService).
/// The installed key is decoded from the registry's DigitalProductId
/// using the standard 25-character decode algorithm.
///
/// # Returns
/// * `Ok(WindowsKeyInfo)` — detected key information
/// * `Err(String)` — error if PowerShell fails
pub fn detect_windows_keys() -> Result<WindowsKeyInfo, String> {
    println!("[Deploy] Detecting Windows product keys...");

    // PowerShell script that detects both keys and outputs structured data.
    // Each piece of data is on its own line prefixed with a label for easy parsing.
    // The DigitalProductId decode algorithm is the standard well-known method used
    // by every key detection tool (ProduKey, ShowKeyPlus, etc.).
    let ps_script = r#"
# 1. OEM/BIOS key (embedded in firmware by manufacturer)
try {
    $oem = (Get-CimInstance -ClassName SoftwareLicensingService).OA3xOriginalProductKey
    if ($oem) { Write-Output "OEM_KEY:$oem" }
    else { Write-Output "OEM_KEY:" }
} catch { Write-Output "OEM_KEY:" }

# 2. Installed key (decoded from registry DigitalProductId)
try {
    $value = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').DigitalProductId
    if ($value) {
        # Standard decode algorithm for Windows 8+ product keys
        $key = ""
        $chars = "BCDFGHJKMPQRTVWXY2346789"
        $decoded = $value[52..66]

        # The isWin8+ flag is stored in byte 66
        $isWin8 = [math]::Floor($value[66] / 6) -band 1
        $value[66] = ($value[66] -band 0xF7) -bor (($isWin8 -band 2) * 4)
        $decoded = $value[52..66]

        for ($i = 24; $i -ge 0; $i--) {
            $current = 0
            for ($j = 14; $j -ge 0; $j--) {
                $current = $current * 256
                $current = $decoded[$j] + $current
                $decoded[$j] = [math]::Floor([double]($current / 24))
                $current = $current % 24
            }
            $key = $chars[$current] + $key
        }

        # Insert 'N' for Win8+ keys at position determined by last decode
        if ($isWin8 -eq 1) {
            $keypart1 = $key.Substring(1, $current)
            $keypart2 = $key.Substring($current + 1)
            $key = $keypart1 + "N" + $keypart2
        }

        # Format as XXXXX-XXXXX-XXXXX-XXXXX-XXXXX
        $formatted = ""
        for ($i = 0; $i -lt 25; $i++) {
            $formatted += $key[$i]
            if (($i + 1) % 5 -eq 0 -and $i -lt 24) { $formatted += "-" }
        }
        Write-Output "INSTALLED_KEY:$formatted"
    } else {
        Write-Output "INSTALLED_KEY:"
    }
} catch { Write-Output "INSTALLED_KEY:" }

# 3. Edition and license status
try {
    $product = Get-CimInstance -ClassName SoftwareLicensingProduct |
        Where-Object { $_.PartialProductKey -and $_.LicenseStatus -eq 1 } |
        Select-Object -First 1
    if ($product) {
        Write-Output "EDITION:$($product.Name)"
        Write-Output "STATUS:Licensed"
    } else {
        # Fallback: check for any active product even if not fully licensed
        $any = Get-CimInstance -ClassName SoftwareLicensingProduct |
            Where-Object { $_.PartialProductKey } |
            Select-Object -First 1
        if ($any) {
            $statusText = switch ($any.LicenseStatus) {
                0 { "Unlicensed" }
                1 { "Licensed" }
                2 { "Out-of-box Grace" }
                3 { "Out-of-tolerance Grace" }
                4 { "Non-genuine Grace" }
                5 { "Notification" }
                6 { "Extended Grace" }
                default { "Unknown" }
            }
            Write-Output "EDITION:$($any.Name)"
            Write-Output "STATUS:$statusText"
        } else {
            Write-Output "EDITION:"
            Write-Output "STATUS:Not found"
        }
    }
} catch {
    Write-Output "EDITION:"
    Write-Output "STATUS:Error detecting"
}

# 4. Hostname
Write-Output "HOSTNAME:$env:COMPUTERNAME"
"#;

    // Run the PowerShell script
    let output = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-NoProfile", "-Command", ps_script])
        .output()
        .map_err(|e| format!("Failed to run PowerShell: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("PowerShell key detection failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse the labeled output lines
    let mut info = WindowsKeyInfo {
        oem_key: String::new(),
        installed_key: String::new(),
        edition: String::new(),
        status: String::new(),
        hostname: String::new(),
        // Get today's date for the backup timestamp
        date: {
            let now = std::time::SystemTime::now();
            let duration = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let secs = duration.as_secs();
            // Simple date calculation (good enough for a human-readable timestamp)
            let days = secs / 86400;
            let years = 1970 + (days / 365);  // Approximate — close enough for display
            let remaining_days = days % 365;
            let month = remaining_days / 30 + 1;
            let day = remaining_days % 30 + 1;
            format!("{}-{:02}-{:02}", years, month.min(12), day.min(31))
        },
    };

    // Parse each labeled line from PowerShell output
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("OEM_KEY:") {
            info.oem_key = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("INSTALLED_KEY:") {
            info.installed_key = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("EDITION:") {
            info.edition = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("STATUS:") {
            info.status = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("HOSTNAME:") {
            info.hostname = value.trim().to_string();
        }
    }

    println!("[Deploy] Key detection complete:");
    println!("  OEM key: {}", if info.oem_key.is_empty() { "(none)" } else { &info.oem_key });
    println!("  Installed key: {}", if info.installed_key.is_empty() { "(none)" } else { &info.installed_key });
    println!("  Edition: {}", info.edition);
    println!("  Status: {}", info.status);
    println!("  Hostname: {}", info.hostname);

    Ok(info)
}

/// Save detected Windows key info to saved_keys.json next to the EXE.
/// This file persists on the USB drive so keys survive reboots between
/// live Windows (backup) and WinPE (deploy) sessions.
///
/// # Arguments
/// * `info` — Key information to save
///
/// # Returns
/// * `Ok(())` — saved successfully
/// * `Err(String)` — error message
pub fn save_keys_to_file(info: &WindowsKeyInfo) -> Result<(), String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let path = exe_dir.join("saved_keys.json");

    let json = serde_json::to_string_pretty(info)
        .map_err(|e| format!("Failed to serialize key info: {}", e))?;

    fs::write(&path, &json)
        .map_err(|e| format!("Failed to write saved_keys.json: {}", e))?;

    println!("[Deploy] Saved keys to: {}", path.display());
    Ok(())
}

/// Load previously saved Windows key info from saved_keys.json.
/// Returns None if the file doesn't exist (no previous backup).
///
/// # Returns
/// * `Some(WindowsKeyInfo)` — loaded key info
/// * `None` — no saved keys file found
pub fn load_saved_keys() -> Option<WindowsKeyInfo> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let path = exe_dir.join("saved_keys.json");

    if !path.exists() {
        return None;
    }

    let content = fs::read_to_string(&path).ok()?;
    let info: WindowsKeyInfo = serde_json::from_str(&content).ok()?;

    println!("[Deploy] Loaded saved keys from: {}", path.display());
    Some(info)
}
