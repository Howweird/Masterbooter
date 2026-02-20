// ============================================
// MasterBooter - winpe.rs
// ============================================
// This module handles WinPE/WinRE detection and ISO building.
//
// Key concepts:
// - WinRE (Windows Recovery Environment) is built into Windows
// - It contains a WIM file that can be used as a base for WinPE
// - We can customize it by adding drivers, tools, and MasterBooter
// - Windows ISO files contain boot.wim which can be used as PE base
//
// ENHANCED FEATURES (v2):
// - ADK package installation (PowerShell, WMI, Network, etc.)
// - PE fixes (DPI scaling, WallpaperHost removal, profile folders)
// - Driver injection
// - Configurable options UI similar to AMPIPIT
// ============================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::io::Read as IoRead;  // For reading ISO signature bytes
use rfd::FileDialog;

// Import our ADK packages and PE fixes modules
use crate::adk_packages::{self, AdkPackage};
use crate::pe_fixes::{self, PeFix, FixOptions};

// ============================================
// WIM MOUNT GUARD (RAII SAFETY NET)
// ============================================
// This struct auto-unmounts a WIM if the build crashes or returns early.
// Inspired by GhostWin's Drop-trait auto-unmount pattern.
//
// How it works:
// 1. Create a guard with WimMountGuard::new(mount_path)
// 2. After a successful mount, call guard.mark_mounted()
// 3. On normal completion, call guard.commit_and_disarm()
//    - This unmounts with /Commit (saves changes) and disarms the guard
// 4. If the function returns early (error), Drop runs automatically
//    - This unmounts with /Discard (throws away changes)
//
// This ensures WIMs never get stuck mounted after failures.

/// RAII guard that auto-unmounts a WIM on drop.
/// Prevents stuck mounts if the build process crashes or returns early.
pub struct WimMountGuard {
    /// Path where the WIM is mounted
    mount_path: PathBuf,
    /// Whether a WIM is actually mounted (set after successful mount)
    is_mounted: bool,
    /// Whether the guard has been disarmed (normal completion)
    disarmed: bool,
}

impl WimMountGuard {
    /// Create a new guard for the given mount path.
    /// The guard starts in a "not mounted" state - call mark_mounted() after
    /// successfully mounting the WIM.
    pub fn new(mount_path: &Path) -> Self {
        WimMountGuard {
            mount_path: mount_path.to_path_buf(),
            is_mounted: false,
            disarmed: false,
        }
    }

    /// Mark that a WIM has been successfully mounted.
    /// After this, the guard will auto-unmount on drop.
    pub fn mark_mounted(&mut self) {
        self.is_mounted = true;
    }

    /// Normal completion: unmount with /Commit (save changes) and disarm the guard.
    /// Returns Ok(()) if unmount succeeded, Err with message on failure.
    pub fn commit_and_disarm(&mut self) -> Result<(), String> {
        if !self.is_mounted || self.disarmed {
            // Nothing to do - WIM wasn't mounted or already handled
            return Ok(());
        }

        println!("WimMountGuard: Committing and unmounting WIM...");
        let result = unmount_wim(&self.mount_path, true);
        self.disarmed = true;
        self.is_mounted = false;
        result
    }

    /// Get the mount path (for use in operations while mounted)
    #[allow(dead_code)]
    pub fn mount_path(&self) -> &Path {
        &self.mount_path
    }
}

impl Drop for WimMountGuard {
    /// Automatically unmount with /Discard if the guard was not disarmed.
    /// This is the error path - it runs when the function returns early due to an error.
    fn drop(&mut self) {
        if self.is_mounted && !self.disarmed {
            println!("WimMountGuard: ERROR PATH - Unmounting WIM with /Discard (changes lost)");
            // Best-effort unmount - we're in drop, so we can't return errors
            let _ = unmount_wim(&self.mount_path, false);
        }
    }
}

// ============================================
// BCD STORE CREATION (FALLBACK)
// ============================================
// When building without copype (no ADK), the BCD store may not exist.
// This creates a minimal BCD from scratch using bcdedit.
// Based on AMPIPIT's BCD creation approach.

/// Create a BCD (Boot Configuration Data) store from scratch.
///
/// This is used when we're building a PE without copype (e.g., from a WIM file
/// or when the ADK doesn't have proper boot files). We create a minimal BCD
/// that tells the boot manager to load boot.wim.
///
/// # Arguments
/// * `bcd_path` - Where to create the BCD file (e.g., media/boot/BCD)
/// * `boot_wim_path` - Relative path to boot.wim from the BCD (e.g., \sources\boot.wim)
/// * `for_uefi` - If true, create a UEFI BCD; if false, create a BIOS BCD
///
/// # Returns
/// Ok(()) on success, Err with message on failure
pub fn create_bcd_store(bcd_path: &Path, boot_wim_path: &str, for_uefi: bool) -> Result<(), String> {
    let mode_name = if for_uefi { "UEFI" } else { "BIOS" };
    println!("Creating {} BCD store at: {}", mode_name, bcd_path.display());

    // Ensure parent directory exists
    if let Some(parent) = bcd_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create BCD directory: {}", e))?;
    }

    // Remove existing BCD if present (bcdedit can't overwrite)
    if bcd_path.exists() {
        fs::remove_file(bcd_path)
            .map_err(|e| format!("Failed to remove existing BCD: {}", e))?;
    }

    // Step 1: Create empty BCD store
    let output = Command::new("bcdedit")
        .arg("/createstore")
        .arg(bcd_path)
        .output()
        .map_err(|e| format!("Failed to run bcdedit: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "bcdedit /createstore failed: {}",
            String::from_utf8_lossy(&output.stdout)
        ));
    }

    // Step 2: Create the boot manager entry
    let bcd = bcd_path.to_string_lossy().to_string();
    run_bcdedit(&["/store", &bcd, "/create", "{bootmgr}", "/d", "Windows Boot Manager"])?;

    // Step 3: Create a new OS loader entry for WinPE
    // bcdedit /create returns the new GUID - we need to capture it
    let create_output = Command::new("bcdedit")
        .args(["/store", &bcd, "/create", "/d", "MasterBooter WinPE", "/application", "osloader"])
        .output()
        .map_err(|e| format!("Failed to create BCD entry: {}", e))?;

    let stdout_str = String::from_utf8_lossy(&create_output.stdout).to_string();
    let guid = extract_guid_from_bcdedit_output(&stdout_str)
        .ok_or_else(|| format!(
            "Could not extract GUID from bcdedit output: {}",
            stdout_str
        ))?;

    println!("  Created BCD entry with GUID: {}", guid);

    // Step 4: Configure the boot manager to use our entry
    run_bcdedit(&["/store", &bcd, "/set", "{bootmgr}", "default", &guid])?;
    run_bcdedit(&["/store", &bcd, "/set", "{bootmgr}", "displayorder", &guid])?;
    run_bcdedit(&["/store", &bcd, "/set", "{bootmgr}", "timeout", "0"])?;

    // Step 5: Configure the OS loader entry
    run_bcdedit(&["/store", &bcd, "/set", &guid, "device", &format!("ramdisk=[boot]{}", boot_wim_path)])?;
    run_bcdedit(&["/store", &bcd, "/set", &guid, "osdevice", &format!("ramdisk=[boot]{}", boot_wim_path)])?;
    run_bcdedit(&["/store", &bcd, "/set", &guid, "path", "\\windows\\system32\\winload.exe"])?;
    run_bcdedit(&["/store", &bcd, "/set", &guid, "systemroot", "\\windows"])?;
    run_bcdedit(&["/store", &bcd, "/set", &guid, "detecthal", "yes"])?;
    run_bcdedit(&["/store", &bcd, "/set", &guid, "winpe", "yes"])?;

    // Step 6: Create ramdisk options entry
    let ramdisk_output = Command::new("bcdedit")
        .args(["/store", &bcd, "/create", "{ramdiskoptions}"])
        .output();

    // Ramdisk options might already exist, that's OK
    if let Ok(out) = ramdisk_output {
        if out.status.success() || String::from_utf8_lossy(&out.stdout).contains("already exists") {
            let _ = run_bcdedit(&["/store", &bcd, "/set", "{ramdiskoptions}", "ramdisksdidevice", "boot"]);
            let _ = run_bcdedit(&["/store", &bcd, "/set", "{ramdiskoptions}", "ramdisksdipath", "\\boot\\boot.sdi"]);
        }
    }

    // Disable driver signature enforcement so WiFi protocol drivers can load.
    // WiFi drivers (nwifi.sys, vwififlt.sys, etc.) are file-copied from install.wim,
    // not DISM-injected, so WinPE's code integrity checks would reject them at boot.
    disable_driver_signature_enforcement(bcd_path)?;

    println!("  {} BCD store created successfully", mode_name);
    Ok(())
}

/// Disable driver signature enforcement in a BCD store.
///
/// WinPE enforces driver signature verification by default. Drivers that are
/// manually copied (not DISM-injected) into System32\Drivers\ fail this check
/// at boot time with "cannot verify digital signature" errors.
///
/// This is how PhoenixPE solves the same problem — see 700-BCD.script lines
/// 184-199 (BypassDriverSigning section). We use three methods for maximum
/// compatibility across Windows 10/11 PE versions:
///
/// 1. loadoptions DDISABLE_INTEGRITY_CHECKS — traditional WinPE approach
/// 2. nointegritychecks on — modern explicit disable
/// 3. testsigning on — allows test-signed and unsigned drivers
///
/// If any one method fails (e.g., older bcdedit version), the others still work.
///
/// # Arguments
/// * `bcd_path` - Path to the BCD store file to modify
fn disable_driver_signature_enforcement(bcd_path: &Path) -> Result<(), String> {
    let bcd = bcd_path.to_string_lossy().to_string();

    // Method 1: Set loadoptions DDISABLE_INTEGRITY_CHECKS
    // This is the traditional WinPE approach (PhoenixPE uses this)
    // The leading 'D' is intentional — it's a Windows boot option prefix
    let _ = run_bcdedit(&["/store", &bcd, "/set", "{default}",
        "loadoptions", "DDISABLE_INTEGRITY_CHECKS"]);

    // Method 2: Set nointegritychecks on
    // This is the modern approach — explicitly disables kernel code integrity checks
    let _ = run_bcdedit(&["/store", &bcd, "/set", "{default}",
        "nointegritychecks", "on"]);

    // Method 3: Set testsigning on
    // Allows test-signed and unsigned drivers to load
    let _ = run_bcdedit(&["/store", &bcd, "/set", "{default}",
        "testsigning", "on"]);

    Ok(())
}

/// Helper: Run a bcdedit command and return Ok/Err based on success.
fn run_bcdedit(args: &[&str]) -> Result<(), String> {
    let output = Command::new("bcdedit")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run bcdedit: {}", e))?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Some "failures" are OK (like entry already exists)
        if stdout.contains("already exists") {
            return Ok(());
        }
        return Err(format!("bcdedit {:?} failed: {}", args, stdout));
    }
    Ok(())
}

/// Extract a GUID from bcdedit output.
///
/// bcdedit /create outputs something like:
/// "The entry {12345678-1234-1234-1234-123456789abc} was successfully created."
///
/// This function finds and returns the GUID including braces.
pub fn extract_guid_from_bcdedit_output(output: &str) -> Option<String> {
    // Look for pattern: {xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}
    let start = output.find('{')?;
    let end = output[start..].find('}')? + start + 1;
    Some(output[start..end].to_string())
}

// ============================================
// ISO VERIFICATION (POST-BUILD)
// ============================================
// Adapted from GhostWin's 5-point ISO verification.
// Checks that the ISO is valid and bootable after creation.

/// Result of ISO verification
#[derive(Debug)]
#[allow(dead_code)]
pub struct IsoVerification {
    /// Whether the ISO passed all checks
    pub passed: bool,
    /// Individual check results
    pub checks: Vec<(String, bool, String)>, // (check_name, passed, detail)
    /// Summary message
    pub summary: String,
}

/// Verify a WinPE ISO after building.
///
/// Performs 5 checks (adapted from GhostWin):
/// 1. File exists
/// 2. Size is reasonable (>100 MB)
/// 3. ISO 9660 signature at offset 0x8001
/// 4. El Torito boot indicator at expected offset
/// 5. Critical files present (bootmgr, boot.wim) via 7-Zip listing
///
/// # Arguments
/// * `iso_path` - Path to the ISO file to verify
///
/// # Returns
/// IsoVerification with results of all checks
pub fn verify_pe_iso(iso_path: &Path) -> IsoVerification {
    println!("Verifying ISO: {}", iso_path.display());

    let mut checks = Vec::new();

    // Check 1: File exists
    let exists = iso_path.exists() && iso_path.is_file();
    checks.push((
        "File exists".to_string(),
        exists,
        if exists {
            format!("Found at {}", iso_path.display())
        } else {
            format!("NOT FOUND: {}", iso_path.display())
        },
    ));

    if !exists {
        return IsoVerification {
            passed: false,
            checks,
            summary: "ISO file does not exist".to_string(),
        };
    }

    // Check 2: Size is reasonable (>100 MB for a WinPE ISO)
    let file_size = fs::metadata(iso_path).map(|m| m.len()).unwrap_or(0);
    let size_mb = file_size as f64 / (1024.0 * 1024.0);
    let size_ok = file_size > 100 * 1024 * 1024; // >100 MB
    checks.push((
        "Size check".to_string(),
        size_ok,
        format!("{:.1} MB {}", size_mb, if size_ok { "(OK)" } else { "(too small - expected >100 MB)" }),
    ));

    // Check 3: ISO 9660 signature at offset 0x8001
    // The ISO 9660 Primary Volume Descriptor starts at sector 16 (0x8000)
    // Byte at 0x8000 should be 0x01 (PVD type), bytes 0x8001-0x8005 should be "CD001"
    let iso_sig_ok = check_iso_9660_signature(iso_path);
    checks.push((
        "ISO 9660 signature".to_string(),
        iso_sig_ok,
        if iso_sig_ok {
            "Valid CD001 signature found".to_string()
        } else {
            "Missing or invalid ISO 9660 signature".to_string()
        },
    ));

    // Check 4: El Torito boot indicator
    // The Boot Record Volume Descriptor is typically at sector 17 (0x8800)
    // It should contain "EL TORITO SPECIFICATION" or boot catalog pointer
    let el_torito_ok = check_el_torito_boot(iso_path);
    checks.push((
        "El Torito boot record".to_string(),
        el_torito_ok,
        if el_torito_ok {
            "Boot record found - ISO is bootable".to_string()
        } else {
            "No El Torito boot record - ISO may not be bootable".to_string()
        },
    ));

    // Check 5: Critical files present via 7-Zip listing
    let critical_files_ok = check_iso_critical_files(iso_path);
    checks.push((
        "Critical files".to_string(),
        critical_files_ok,
        if critical_files_ok {
            "bootmgr and boot.wim found".to_string()
        } else {
            "Missing bootmgr or boot.wim".to_string()
        },
    ));

    // Build summary
    let passed_count = checks.iter().filter(|(_, ok, _)| *ok).count();
    let total = checks.len();
    let all_passed = passed_count == total;

    for (name, ok, detail) in &checks {
        println!("  [{}] {}: {}", if *ok { "OK" } else { "FAIL" }, name, detail);
    }

    let summary = if all_passed {
        format!("ISO verification passed ({}/{})", passed_count, total)
    } else {
        format!("ISO verification: {}/{} checks passed", passed_count, total)
    };

    println!("{}", summary);

    IsoVerification {
        passed: all_passed,
        checks,
        summary,
    }
}

/// Check for ISO 9660 "CD001" signature at offset 0x8001
fn check_iso_9660_signature(iso_path: &Path) -> bool {
    let mut file = match fs::File::open(iso_path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    // Seek to offset 0x8000 (sector 16, the Primary Volume Descriptor)
    use std::io::Seek;
    if file.seek(std::io::SeekFrom::Start(0x8000)).is_err() {
        return false;
    }

    // Read 6 bytes: type byte (0x01) + "CD001"
    let mut buf = [0u8; 6];
    if file.read_exact(&mut buf).is_err() {
        return false;
    }

    // Check: type=0x01, magic="CD001"
    buf[0] == 0x01 && &buf[1..6] == b"CD001"
}

/// Check for El Torito boot record at sector 17 (offset 0x8800)
fn check_el_torito_boot(iso_path: &Path) -> bool {
    let mut file = match fs::File::open(iso_path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    // The Boot Record Volume Descriptor is at sector 17 (0x8800)
    use std::io::Seek;
    if file.seek(std::io::SeekFrom::Start(0x8800)).is_err() {
        return false;
    }

    // Read the boot record descriptor
    let mut buf = [0u8; 64];
    if file.read_exact(&mut buf).is_err() {
        return false;
    }

    // Type 0x00 = Boot Record, followed by "CD001", then "EL TORITO"
    // Or just check for the CD001 identifier at this sector with type 0
    let has_boot_type = buf[0] == 0x00;
    let has_cd001 = &buf[1..6] == b"CD001";
    let has_el_torito = std::str::from_utf8(&buf[7..39])
        .map(|s| s.contains("EL TORITO"))
        .unwrap_or(false);

    has_boot_type && has_cd001 && has_el_torito
}

/// Check that critical files (bootmgr, boot.wim) exist in the ISO
fn check_iso_critical_files(iso_path: &Path) -> bool {
    let seven_zip = match find_7zip() {
        Some(p) => p,
        None => {
            println!("  Warning: 7-Zip not found, skipping critical file check");
            return true; // Can't check without 7-Zip, assume OK
        }
    };

    let output = match Command::new(&seven_zip).arg("l").arg(iso_path).output() {
        Ok(o) => o,
        Err(_) => return false,
    };

    if !output.status.success() {
        return false;
    }

    let listing = String::from_utf8_lossy(&output.stdout).to_lowercase();

    let has_bootmgr = listing.contains("bootmgr");
    let has_boot_wim = listing.contains("boot.wim");

    if !has_bootmgr {
        println!("  Warning: bootmgr not found in ISO listing");
    }
    if !has_boot_wim {
        println!("  Warning: boot.wim not found in ISO listing");
    }

    has_bootmgr && has_boot_wim
}

// ============================================
// BUILD CONFIG VALIDATION (PRE-FLIGHT)
// ============================================
// Checks everything before the slow build starts.
// Prevents wasting time on a build that will fail halfway through.

/// Result of pre-flight validation
#[derive(Debug)]
pub struct ValidationResult {
    /// Whether all checks passed
    pub valid: bool,
    /// List of errors (must fix before building)
    pub errors: Vec<String>,
    /// List of warnings (build can proceed but may have issues)
    pub warnings: Vec<String>,
}

/// Validate build configuration before starting the build.
///
/// Checks:
/// 1. Source file exists (WIM or ISO)
/// 2. Output directory is writable
/// 3. Enough disk space for build (~5 GB working space)
/// 4. ADK installed if packages are requested
/// 5. 7-Zip available (required for ISO extraction)
/// 6. oscdimg available (required for ISO creation)
///
/// Call this at the top of build_pe_iso() to fail fast.
pub fn validate_build_config(config: &PeBuildConfig) -> ValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    println!("Validating build configuration...");

    // 1. Source file exists
    if !config.source_path.exists() {
        errors.push(format!(
            "Source file not found: {}\n\
            What to do:\n\
            1. Check that the file path is correct\n\
            2. If using an ISO, re-select it with the Browse button\n\
            3. If using Local RE, click Detect to find WinRE",
            config.source_path.display()
        ));
    }

    // 2. Output directory is writable
    if let Some(parent) = config.output_path.parent() {
        if parent.exists() {
            // Try creating a temp file to test writability
            let test_file = parent.join(".masterbooter_write_test");
            match fs::write(&test_file, "test") {
                Ok(_) => {
                    let _ = fs::remove_file(&test_file);
                }
                Err(e) => {
                    errors.push(format!(
                        "Output directory is not writable: {}\n\
                        Error: {}\n\
                        What to do:\n\
                        1. Choose a different output location\n\
                        2. Check folder permissions\n\
                        3. Make sure the drive is not full or read-only",
                        parent.display(), e
                    ));
                }
            }
        } else {
            // Try to create it
            match fs::create_dir_all(parent) {
                Ok(_) => {}
                Err(e) => {
                    errors.push(format!(
                        "Cannot create output directory: {}\n\
                        Error: {}\n\
                        What to do: Choose a different output location",
                        parent.display(), e
                    ));
                }
            }
        }
    }

    // 3. Disk space check (~5 GB needed for working directory)
    if let Ok(free_space) = get_free_disk_space("C:") {
        let free_gb = free_space as f64 / (1024.0 * 1024.0 * 1024.0);
        if free_gb < 5.0 {
            errors.push(format!(
                "Insufficient disk space: {:.1} GB free (need at least 5 GB)\n\
                What to do:\n\
                1. Free up space on the C: drive\n\
                2. Delete temporary files (run Disk Cleanup)\n\
                3. Move large files to another drive",
                free_gb
            ));
        } else if free_gb < 10.0 {
            warnings.push(format!(
                "Low disk space: {:.1} GB free (10+ GB recommended for large PE builds)",
                free_gb
            ));
        }
    }

    // 4. ADK check if packages are requested
    if config.install_packages && !config.enabled_packages.is_empty() {
        let adk_info = detect_adk();
        if !adk_info.found {
            errors.push(
                "ADK packages are selected but Windows ADK is not installed.\n\
                What to do:\n\
                1. Click 'Install Dependencies' to install the ADK, or\n\
                2. Disable 'Install ADK Packages' in the build options, or\n\
                3. Download ADK manually from: https://go.microsoft.com/fwlink/?linkid=2289980\n\
                Note: The ADK installer says 'Windows 10' in its title but supports Windows 11."
                    .to_string()
            );
        }
    }

    // 5. 7-Zip check (needed for ISO extraction)
    let source_ext = config.source_path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if source_ext == "iso" && find_7zip().is_none() {
        errors.push(
            "7-Zip is required to extract boot files from ISO but was not found.\n\
            What to do:\n\
            1. Click 'Install Dependencies' to install 7-Zip, or\n\
            2. Download 7-Zip from: https://www.7-zip.org/"
                .to_string()
        );
    }

    // 6. oscdimg check (needed for ISO creation)
    if find_oscdimg().is_none() {
        // Only an error if we're building an ISO and not using copype
        // (copype + MakeWinPEMedia doesn't need oscdimg separately)
        let adk_info = detect_adk();
        if !adk_info.found {
            errors.push(
                "oscdimg not found (part of Windows ADK).\n\
                What to do:\n\
                1. Install Windows ADK and WinPE Add-on, or\n\
                2. Click 'Install Dependencies'"
                    .to_string()
            );
        }
    }

    // Build result
    let valid = errors.is_empty();
    if valid {
        println!("  Build configuration is valid");
    } else {
        println!("  Build configuration has {} error(s)", errors.len());
    }
    if !warnings.is_empty() {
        println!("  {} warning(s)", warnings.len());
    }

    ValidationResult {
        valid,
        errors,
        warnings,
    }
}

// ============================================
// FORCE UNMOUNT (CLEANUP AT BUILD START)
// ============================================
// Clean up stale mounts from previous failed builds.
// Called at the start of build_pe_iso().

/// Force-unmount any stale WIM mounts from previous failed builds.
///
/// This does two things:
/// 1. Unmount the known mount directory if anything is there
/// 2. Run DISM /Cleanup-Wim to clean up any orphaned mounts
///
/// Based on AMPIPIT's force_unmount() at build start.
fn force_unmount_stale_mounts() {
    println!("Checking for stale WIM mounts...");

    // 1. Try unmounting our known mount directory
    let known_mount = std::env::temp_dir().join("MasterBooter_WIM_Mount");
    if known_mount.exists() && is_wim_mounted(&known_mount) {
        println!("  Found stale mount at {}, unmounting...", known_mount.display());
        let _ = unmount_wim(&known_mount, false); // Discard - stale data
    }

    // Clean up the mount directory itself
    if known_mount.exists() {
        let _ = fs::remove_dir_all(&known_mount);
    }

    // 2. Run DISM /Cleanup-Wim to handle any other orphaned mounts
    let output = Command::new("dism")
        .arg("/Cleanup-Wim")
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("completed successfully") || stdout.contains("cleanup operation") {
            println!("  DISM cleanup completed");
        }
    }
}

// ============================================
// WINRE DETECTION
// ============================================

/// Information about the detected WinRE
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WinreInfo {
    pub found: bool,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub size_display: String,
}

/// Detect the local Windows Recovery Environment (WinRE.wim)
///
/// WinRE is typically located at:
/// - C:\Windows\System32\Recovery\WinRE.wim (most common)
/// - C:\Recovery\WindowsRE\WinRE.wim (alternative location)
///
/// Returns WinreInfo with detection results
pub fn detect_winre() -> WinreInfo {
    println!("Detecting WinRE...");

    // Common locations for WinRE.wim
    let possible_paths = [
        PathBuf::from(r"C:\Windows\System32\Recovery\WinRE.wim"),
        PathBuf::from(r"C:\Recovery\WindowsRE\WinRE.wim"),
        PathBuf::from(r"C:\Windows\System32\Recovery\Winre.wim"), // Case variations
    ];

    for path in &possible_paths {
        if path.exists() {
            if let Ok(metadata) = fs::metadata(path) {
                let size_bytes = metadata.len();
                let size_display = format_file_size(size_bytes);

                println!("Found WinRE at: {}", path.display());
                println!("Size: {}", size_display);

                return WinreInfo {
                    found: true,
                    path: path.clone(),
                    size_bytes,
                    size_display,
                };
            }
        }
    }

    // WinRE not found in standard locations
    // Try using reagentc to find it
    if let Some(path) = detect_winre_via_reagentc() {
        if path.exists() {
            if let Ok(metadata) = fs::metadata(&path) {
                let size_bytes = metadata.len();
                let size_display = format_file_size(size_bytes);

                println!("Found WinRE via reagentc at: {}", path.display());
                println!("Size: {}", size_display);

                return WinreInfo {
                    found: true,
                    path,
                    size_bytes,
                    size_display,
                };
            }
        }
    }

    println!("WinRE not found");
    WinreInfo {
        found: false,
        path: PathBuf::new(),
        size_bytes: 0,
        size_display: String::new(),
    }
}

/// Try to detect WinRE location using reagentc command
fn detect_winre_via_reagentc() -> Option<PathBuf> {
    // reagentc /info shows the WinRE location
    let output = Command::new("reagentc")
        .arg("/info")
        .output();

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse the output to find the WinRE location
        // Look for lines like "Windows RE location: \\?\GLOBALROOT\device\..."
        for line in stdout.lines() {
            if line.contains("Windows RE location") || line.contains("WinRE location") {
                // Extract the path (this is complex due to the device path format)
                // For now, we just check if it's enabled
                if !line.contains("Disabled") && !line.to_lowercase().contains("not") {
                    // WinRE is enabled, try the default path
                    let default_path = PathBuf::from(r"C:\Windows\System32\Recovery\WinRE.wim");
                    if default_path.exists() {
                        return Some(default_path);
                    }
                }
            }
        }
    }

    // If reagentc didn't work, try parsing ReAgent.xml directly
    // This handles WinRE on hidden recovery partitions
    if let Some(path) = detect_winre_via_reagent_xml() {
        return Some(path);
    }

    None
}

/// Parse ReAgent.xml to find WinRE on hidden recovery partitions
///
/// WinRE is often stored on a hidden recovery partition without a drive letter.
/// The ReAgent.xml file contains the partition GUID which we can use to access it
/// via the volume GUID path: \\?\Volume{GUID}\Recovery\WindowsRE\WinRE.wim
fn detect_winre_via_reagent_xml() -> Option<PathBuf> {
    let reagent_xml = PathBuf::from(r"C:\Windows\System32\Recovery\ReAgent.xml");

    if !reagent_xml.exists() {
        println!("ReAgent.xml not found");
        return None;
    }

    // Read and parse the XML file
    let content = match fs::read_to_string(&reagent_xml) {
        Ok(c) => c,
        Err(e) => {
            println!("Failed to read ReAgent.xml: {}", e);
            return None;
        }
    };

    // Look for WinreLocation element with guid attribute
    // Format: <WinreLocation path="\Recovery\WindowsRE" ... guid="{GUID}"/>
    let mut guid = String::new();
    let mut winre_path = String::new();

    for line in content.lines() {
        if line.contains("WinreLocation") {
            // Extract guid attribute
            if let Some(guid_start) = line.find("guid=\"") {
                let guid_value = &line[guid_start + 6..];
                if let Some(guid_end) = guid_value.find('"') {
                    guid = guid_value[..guid_end].to_string();
                }
            }
            // Extract path attribute
            if let Some(path_start) = line.find("path=\"") {
                let path_value = &line[path_start + 6..];
                if let Some(path_end) = path_value.find('"') {
                    winre_path = path_value[..path_end].to_string();
                }
            }
        }
    }

    if guid.is_empty() || winre_path.is_empty() {
        println!("Could not find WinRE location in ReAgent.xml");
        return None;
    }

    // Skip if guid is all zeros (WinRE not configured)
    if guid == "{00000000-0000-0000-0000-000000000000}" {
        println!("WinRE not configured (null GUID)");
        return None;
    }

    println!("Found WinRE partition GUID: {}", guid);
    println!("WinRE path on partition: {}", winre_path);

    // Build the volume GUID path
    // Format: \\?\Volume{GUID}\path\to\WinRE.wim
    let volume_path = format!(r"\\?\Volume{}\{}\WinRE.wim", guid, winre_path.trim_start_matches('\\'));
    let winre_full_path = PathBuf::from(&volume_path);

    println!("Checking volume path: {}", volume_path);

    // Check if we can access this path
    if winre_full_path.exists() {
        println!("Found WinRE via volume GUID path");
        return Some(winre_full_path);
    }

    // If direct volume path doesn't work, try mounting the partition temporarily
    println!("Volume path not accessible, attempting to mount recovery partition...");

    if let Some(mounted_path) = mount_recovery_partition_and_find_winre(&guid, &winre_path) {
        return Some(mounted_path);
    }

    println!("Could not access WinRE on recovery partition");
    None
}

/// Temporarily mount the recovery partition to access WinRE
/// Returns the path to WinRE.wim if successful
fn mount_recovery_partition_and_find_winre(_guid: &str, winre_subpath: &str) -> Option<PathBuf> {
    // Find Recovery partition by type and mount it with drive letter R:
    // The GUID in ReAgent.xml doesn't always match the volume GUID, so we find by partition type

    let mount_script = r#"
        # Find Recovery partition (typically has Type = 'Recovery')
        $recoveryPartition = Get-Partition | Where-Object { $_.Type -eq 'Recovery' } | Select-Object -First 1

        if ($recoveryPartition) {
            if ($recoveryPartition.DriveLetter) {
                # Already has a drive letter
                Write-Output ($recoveryPartition.DriveLetter + ':')
            } else {
                # Try to assign R: drive letter
                try {
                    $recoveryPartition | Add-PartitionAccessPath -AccessPath 'R:' -ErrorAction Stop
                    Write-Output 'R:'
                } catch {
                    # R: might be in use, try S:
                    try {
                        $recoveryPartition | Add-PartitionAccessPath -AccessPath 'S:' -ErrorAction Stop
                        Write-Output 'S:'
                    } catch {
                        Write-Output ''
                    }
                }
            }
        } else {
            Write-Output ''
        }
    "#;

    let output = Command::new("powershell")
        .arg("-Command")
        .arg(mount_script)
        .output();

    if let Ok(output) = output {
        let drive_letter = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if !drive_letter.is_empty() && drive_letter.len() <= 3 {
            println!("Recovery partition accessible at {}", drive_letter);

            // Build the path to WinRE - try both cases
            let subpath = winre_subpath.trim_start_matches('\\');

            // Try WinRE.wim (uppercase)
            let winre_upper = PathBuf::from(format!(r"{}\{}\WinRE.wim", drive_letter, subpath));
            if winre_upper.exists() {
                println!("Found WinRE at: {}", winre_upper.display());
                return Some(winre_upper);
            }

            // Try winre.wim (lowercase) - common on newer Windows
            let winre_lower = PathBuf::from(format!(r"{}\{}\winre.wim", drive_letter, subpath));
            if winre_lower.exists() {
                println!("Found WinRE at: {}", winre_lower.display());
                return Some(winre_lower);
            }

            // Try Winre.wim (mixed case)
            let winre_mixed = PathBuf::from(format!(r"{}\{}\Winre.wim", drive_letter, subpath));
            if winre_mixed.exists() {
                println!("Found WinRE at: {}", winre_mixed.display());
                return Some(winre_mixed);
            }

            println!("WinRE.wim not found in {}", format!(r"{}\{}\", drive_letter, subpath));
        }
    }

    None
}

// ============================================
// ADK DETECTION
// ============================================

/// Information about the detected Windows ADK
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AdkInfo {
    pub found: bool,
    pub version: String,
    pub path: PathBuf,
    pub winpe_path: PathBuf,
}

/// Detect if Windows Assessment and Deployment Kit (ADK) is installed
///
/// ADK is typically installed at:
/// - C:\Program Files (x86)\Windows Kits\10\
///
/// We specifically need the WinPE add-on which provides:
/// - WinPE base images
/// - Optional packages (WMI, PowerShell, .NET, etc.)
pub fn detect_adk() -> AdkInfo {
    println!("Detecting Windows ADK...");

    // Common ADK installation paths
    let adk_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10"),
    ];

    for base_path in &adk_paths {
        // Check for the deployment tools
        let deployment_tools = base_path.join("Assessment and Deployment Kit").join("Deployment Tools");
        let winpe_path = base_path.join("Assessment and Deployment Kit").join("Windows Preinstallation Environment");

        // Alternative structure (newer ADK versions)
        let alt_winpe = base_path.join("ADK").join("Windows Preinstallation Environment");

        if deployment_tools.exists() || winpe_path.exists() {
            // Try to get version from registry or folder structure
            let version = detect_adk_version(base_path);

            let actual_winpe = if winpe_path.exists() {
                winpe_path
            } else if alt_winpe.exists() {
                alt_winpe
            } else {
                base_path.clone()
            };

            println!("Found ADK at: {}", base_path.display());
            println!("Version: {}", version);

            return AdkInfo {
                found: true,
                version,
                path: base_path.clone(),
                winpe_path: actual_winpe,
            };
        }
    }

    println!("Windows ADK not found");
    AdkInfo {
        found: false,
        version: String::new(),
        path: PathBuf::new(),
        winpe_path: PathBuf::new(),
    }
}

/// Try to detect ADK version from folder structure or registry
fn detect_adk_version(base_path: &Path) -> String {
    // Try to read version from a known file or folder name
    // ADK folders often include version numbers

    // Check for version folders in Assessment and Deployment Kit
    let adk_folder = base_path.join("Assessment and Deployment Kit");
    if adk_folder.exists() {
        if let Ok(entries) = fs::read_dir(&adk_folder) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Look for version numbers in folder names
                if name.starts_with("10.") || name.contains("2004") || name.contains("2104")
                   || name.contains("2204") || name.contains("2304") {
                    return format!("Windows 10 ADK ({})", name);
                }
            }
        }
    }

    // Default version string
    "Windows 10 ADK".to_string()
}

// ============================================
// COMPREHENSIVE DEPENDENCY CHECK
// ============================================

/// Result of checking all PE build dependencies
#[derive(Debug, Clone)]
pub struct DependencyCheckResult {
    pub all_satisfied: bool,
    pub adk_installed: bool,
    pub adk_path: String,
    pub winpe_addon_installed: bool,
    pub winpe_addon_path: String,
    pub oscdimg_available: bool,
    pub oscdimg_path: String,
    pub seven_zip_available: bool,
    pub seven_zip_path: String,
    pub disk_space_ok: bool,
    pub disk_space_gb: f64,
    pub dism_available: bool,
    pub powershell_available: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Check all dependencies required for PE building
/// Returns detailed status of each requirement
pub fn check_pe_build_dependencies() -> DependencyCheckResult {
    let mut result = DependencyCheckResult {
        all_satisfied: true,
        adk_installed: false,
        adk_path: String::new(),
        winpe_addon_installed: false,
        winpe_addon_path: String::new(),
        oscdimg_available: false,
        oscdimg_path: String::new(),
        seven_zip_available: false,
        seven_zip_path: String::new(),
        disk_space_ok: false,
        disk_space_gb: 0.0,
        dism_available: false,
        powershell_available: false,
        errors: Vec::new(),
        warnings: Vec::new(),
    };

    println!("\n========================================");
    println!("Checking PE Build Dependencies");
    println!("========================================\n");

    // 1. Check ADK installation
    let adk_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit"),
    ];

    for adk_path in &adk_paths {
        if adk_path.exists() {
            result.adk_installed = true;
            result.adk_path = adk_path.to_string_lossy().to_string();
            println!("[OK] ADK installed: {}", result.adk_path);
            break;
        }
    }

    if !result.adk_installed {
        result.all_satisfied = false;
        result.errors.push("Windows ADK not installed. Download from: https://go.microsoft.com/fwlink/?linkid=2289980 (supports Windows 11)".to_string());
        println!("[ERROR] ADK not installed");
    }

    // 2. Check WinPE Add-on
    let winpe_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Windows Preinstallation Environment"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Windows Preinstallation Environment"),
    ];

    for winpe_path in &winpe_paths {
        if winpe_path.exists() {
            // Verify it has the amd64 folder with actual content
            let amd64_path = winpe_path.join("amd64").join("WinPE_OCs");
            if amd64_path.exists() {
                result.winpe_addon_installed = true;
                result.winpe_addon_path = winpe_path.to_string_lossy().to_string();
                println!("[OK] WinPE Add-on installed: {}", result.winpe_addon_path);
                break;
            }
        }
    }

    if !result.winpe_addon_installed {
        result.all_satisfied = false;
        result.errors.push("WinPE Add-on not installed. Download from: https://go.microsoft.com/fwlink/?linkid=2243391".to_string());
        println!("[ERROR] WinPE Add-on not installed");
    }

    // 3. Check oscdimg
    let oscdimg_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\oscdimg.exe"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\oscdimg.exe"),
    ];

    for oscdimg_path in &oscdimg_paths {
        if oscdimg_path.exists() {
            result.oscdimg_available = true;
            result.oscdimg_path = oscdimg_path.to_string_lossy().to_string();
            println!("[OK] oscdimg available: {}", result.oscdimg_path);
            break;
        }
    }

    if !result.oscdimg_available {
        result.all_satisfied = false;
        result.errors.push("oscdimg not found - part of ADK Deployment Tools".to_string());
        println!("[ERROR] oscdimg not available");
    }

    // 4. Check 7-Zip
    if let Some(seven_zip) = find_7zip() {
        result.seven_zip_available = true;
        result.seven_zip_path = seven_zip.to_string_lossy().to_string();
        println!("[OK] 7-Zip available: {}", result.seven_zip_path);
    } else {
        result.all_satisfied = false;
        result.errors.push("7-Zip not installed. Download from: https://www.7-zip.org/".to_string());
        println!("[ERROR] 7-Zip not installed");
    }

    // 5. Check DISM (should be built into Windows)
    let dism_check = Command::new("where")
        .arg("dism")
        .output();

    if let Ok(output) = dism_check {
        if output.status.success() {
            result.dism_available = true;
            println!("[OK] DISM available (Windows built-in)");
        }
    }

    if !result.dism_available {
        result.warnings.push("DISM not found in PATH - should be built into Windows".to_string());
        println!("[WARN] DISM not found in PATH");
    }

    // 6. Check PowerShell
    let ps_check = Command::new("where")
        .arg("powershell")
        .output();

    if let Ok(output) = ps_check {
        if output.status.success() {
            result.powershell_available = true;
            println!("[OK] PowerShell available");
        }
    }

    if !result.powershell_available {
        result.warnings.push("PowerShell not found - some features may not work".to_string());
        println!("[WARN] PowerShell not found");
    }

    // 7. Check disk space (need at least 10GB)
    if let Ok(free_space) = get_free_disk_space("C:") {
        result.disk_space_gb = free_space as f64 / (1024.0 * 1024.0 * 1024.0);
        if result.disk_space_gb >= 10.0 {
            result.disk_space_ok = true;
            println!("[OK] Disk space: {:.1} GB free", result.disk_space_gb);
        } else {
            result.all_satisfied = false;
            result.errors.push(format!("Insufficient disk space: {:.1} GB free (need 10+ GB)", result.disk_space_gb));
            println!("[ERROR] Insufficient disk space: {:.1} GB", result.disk_space_gb);
        }
    } else {
        result.warnings.push("Could not check disk space".to_string());
        println!("[WARN] Could not check disk space");
    }

    println!("\n========================================");
    if result.all_satisfied {
        println!("All dependencies satisfied - ready to build PE");
    } else {
        println!("Missing dependencies - see errors above");
    }
    println!("========================================\n");

    result
}

/// Get free disk space on specified drive
fn get_free_disk_space(drive: &str) -> Result<u64, String> {
    let output = Command::new("powershell")
        .arg("-Command")
        .arg(format!("(Get-PSDrive {}).Free", drive.trim_end_matches(':')))
        .output()
        .map_err(|e| format!("Failed to check disk space: {}", e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim()
            .parse::<u64>()
            .map_err(|e| format!("Failed to parse disk space: {}", e))
    } else {
        Err("PowerShell command failed".to_string())
    }
}

// ============================================================================
// DEPENDENCY INSTALLATION SYSTEM
// ============================================================================
// Handles downloading and installing all dependencies needed for PE building:
// - Windows ADK (Assessment and Deployment Kit)
// - WinPE Add-on (Windows Preinstallation Environment)
// - 7-Zip (for archive extraction)
// ============================================================================

/// Download URLs from official sources
pub const ADK_DOWNLOAD_URL: &str = "https://go.microsoft.com/fwlink/?linkid=2289980";
pub const ADK_WINPE_ADDON_URL: &str = "https://go.microsoft.com/fwlink/?linkid=2289981";
pub const SEVEN_ZIP_DOWNLOAD_URL: &str = "https://www.7-zip.org/download.html";

/// Winget package IDs
pub const WINGET_ADK_ID: &str = "Microsoft.WindowsADK";
pub const WINGET_WINPE_ADDON_ID: &str = "Microsoft.ADKPEAddon";
pub const WINGET_7ZIP_ID: &str = "7zip.7zip";

/// Result of a single dependency installation attempt
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct InstallResult {
    pub success: bool,
    pub method: String,  // "winget", "manual", "skipped", "already_installed"
    pub message: String,
}

/// Result of installing all dependencies
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DependencyInstallResult {
    pub all_success: bool,
    pub adk_result: InstallResult,
    pub winpe_addon_result: InstallResult,
    pub seven_zip_result: InstallResult,
    pub summary: String,
    pub next_steps: Vec<String>,
}

/// Check if winget is available
pub fn is_winget_available() -> bool {
    Command::new("winget")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Install a package via winget
/// Returns (success, stdout, stderr)
fn install_via_winget(package_id: &str) -> (bool, String, String) {
    println!("Installing {} via winget...", package_id);

    let output = Command::new("winget")
        .args(["install", "-e", "--id", package_id,
               "--silent", "--accept-package-agreements", "--accept-source-agreements"])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            println!("winget exit code: {:?}", out.status.code());

            // Check for "already installed" message
            if stdout.contains("already installed") || stderr.contains("already installed") {
                return (true, "Already installed".to_string(), String::new());
            }

            (out.status.success(), stdout, stderr)
        }
        Err(e) => (false, String::new(), e.to_string())
    }
}

/// Open a URL in the default browser
fn open_url(url: &str) -> Result<(), String> {
    println!("Opening URL: {}", url);
    Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn()
        .map_err(|e| format!("Failed to open browser: {}", e))?;
    Ok(())
}

// ============================================================================
// INDIVIDUAL DEPENDENCY INSTALLERS
// ============================================================================

/// Check if ADK is installed
fn is_adk_installed() -> bool {
    let adk_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit"),
    ];

    for path in &adk_paths {
        if path.exists() {
            // Verify it has Deployment Tools (key component)
            let deployment_tools = path.join("Deployment Tools");
            if deployment_tools.exists() {
                return true;
            }
        }
    }
    false
}

/// Install Windows ADK by downloading and running the installer directly
pub fn install_adk() -> InstallResult {
    println!("\n--- Installing Windows ADK ---");

    // Check if already installed
    if is_adk_installed() {
        println!("ADK already installed");
        return InstallResult {
            success: true,
            method: "already_installed".to_string(),
            message: "ADK already installed".to_string(),
        };
    }

    // Method 1: Direct download and install (most reliable)
    // NOTE: The ADK installer's window title says "Windows 10" but the latest ADK
    // (10.1.26100.2454) fully supports Windows 11 25H2/24H2. The "10" is the kit version.
    println!("Downloading Windows ADK installer from Microsoft...");
    println!("(The installer says 'Windows 10' in its title but supports Windows 11)");
    println!("URL: {}", ADK_DOWNLOAD_URL);

    let temp_dir = std::env::temp_dir();
    let installer_path = temp_dir.join("adksetup.exe");

    match download_file(ADK_DOWNLOAD_URL, &installer_path) {
        Ok(_) => {
            println!("Running Windows ADK installer silently...");
            println!("Command: {} /quiet /features + /ceip off", installer_path.display());
            println!("This may take several minutes. Please wait...");

            // Run installer silently
            let install_result = Command::new(&installer_path)
                .args(["/quiet", "/features", "+", "/ceip", "off"])
                .output();

            match install_result {
                Ok(out) => {
                    let exit_code = out.status.code().unwrap_or(-1);
                    println!("Installer exit code: {}", exit_code);

                    // Wait for installation to complete
                    println!("Waiting for ADK installation to complete...");
                    for i in 1..=48 {  // Wait up to 4 minutes (ADK is larger)
                        std::thread::sleep(std::time::Duration::from_secs(5));

                        if is_adk_installed() {
                            println!("ADK installation verified after ~{}s", i * 5);
                            let _ = std::fs::remove_file(&installer_path);
                            return InstallResult {
                                success: true,
                                method: "direct_install".to_string(),
                                message: "Windows ADK installed successfully".to_string(),
                            };
                        }

                        if i % 6 == 0 {
                            println!("Still waiting... ({}s elapsed)", i * 5);
                        }
                    }

                    println!("Timeout waiting for ADK installation");
                }
                Err(e) => {
                    println!("Failed to run installer: {}", e);
                }
            }

            let _ = std::fs::remove_file(&installer_path);
        }
        Err(e) => {
            println!("Direct download failed: {}", e);
        }
    }

    // Method 2: Try winget as fallback
    println!("\nDirect install didn't work, trying winget as fallback...");
    if is_winget_available() {
        let (success, _stdout, stderr) = install_via_winget(WINGET_ADK_ID);

        if success {
            println!("Winget reported success, waiting for ADK...");
            for i in 1..=24 {
                std::thread::sleep(std::time::Duration::from_secs(5));
                if is_adk_installed() {
                    return InstallResult {
                        success: true,
                        method: "winget".to_string(),
                        message: "Windows ADK installed via winget".to_string(),
                    };
                }
                if i % 6 == 0 {
                    println!("Waiting for ADK... ({}s)", i * 5);
                }
            }
        }
        println!("Winget failed: {}", stderr);
    }

    // Method 3: Final fallback - open browser
    println!("\nAutomatic installation failed. Opening browser for manual download...");
    let _ = open_url(ADK_DOWNLOAD_URL);
    InstallResult {
        success: false,
        method: "manual".to_string(),
        message: "Auto-install failed. Browser opened for manual download.".to_string(),
    }
}

/// Check if WinPE Add-on is installed
fn is_winpe_addon_installed() -> bool {
    let winpe_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Windows Preinstallation Environment"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Windows Preinstallation Environment"),
    ];

    for path in &winpe_paths {
        if path.exists() {
            // Verify it has actual content (WinPE optional components)
            let amd64_path = path.join("amd64").join("WinPE_OCs");
            if amd64_path.exists() {
                return true;
            }
        }
    }
    false
}

/// Download a file - tries curl first (built into Windows 10 1803+), then PowerShell
fn download_file(url: &str, dest_path: &Path) -> Result<(), String> {
    println!("Downloading from: {}", url);
    println!("Saving to: {}", dest_path.display());

    // Method 1: Try curl.exe (built into Windows 10 1803+, no script policy issues)
    // curl follows redirects by default with -L
    println!("Trying curl.exe...");
    let curl_result = Command::new("curl.exe")
        .args(["-L", "-o", &dest_path.to_string_lossy(), url])
        .output();

    if let Ok(output) = curl_result {
        if output.status.success() && dest_path.exists() {
            let size = std::fs::metadata(dest_path).map(|m| m.len()).unwrap_or(0);
            if size > 0 {
                println!("Download complete via curl ({} bytes)", size);
                return Ok(());
            }
        }
        println!("curl failed or incomplete, trying PowerShell...");
    } else {
        println!("curl.exe not available, trying PowerShell...");
    }

    // Method 2: PowerShell Invoke-WebRequest (works on all Windows 10/11)
    // Using -Command with inline script - execution policy doesn't apply to inline commands
    let ps_script = format!(
        r#"$ProgressPreference = 'SilentlyContinue'; Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing"#,
        url,
        dest_path.to_string_lossy().replace("'", "''")
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps_script])
        .output()
        .map_err(|e| format!("Failed to run PowerShell: {}", e))?;

    if output.status.success() && dest_path.exists() {
        let size = std::fs::metadata(dest_path).map(|m| m.len()).unwrap_or(0);
        if size > 0 {
            println!("Download complete via PowerShell ({} bytes)", size);
            return Ok(());
        }
    }

    // Method 3: bitsadmin (legacy, works on older Windows)
    println!("PowerShell failed, trying bitsadmin...");
    let bits_result = Command::new("bitsadmin")
        .args(["/transfer", "MasterBooterDownload", "/download", "/priority", "high",
               url, &dest_path.to_string_lossy()])
        .output();

    if let Ok(output) = bits_result {
        if output.status.success() && dest_path.exists() {
            let size = std::fs::metadata(dest_path).map(|m| m.len()).unwrap_or(0);
            if size > 0 {
                println!("Download complete via bitsadmin ({} bytes)", size);
                return Ok(());
            }
        }
    }

    Err("All download methods failed".to_string())
}

/// Install WinPE Add-on by downloading and running the installer directly
/// This is more reliable than winget which often fails with dependency errors
pub fn install_winpe_addon() -> InstallResult {
    println!("\n--- Installing WinPE Add-on ---");

    // Check if already installed
    if is_winpe_addon_installed() {
        println!("WinPE Add-on already installed");
        return InstallResult {
            success: true,
            method: "already_installed".to_string(),
            message: "WinPE Add-on already installed".to_string(),
        };
    }

    // WinPE Add-on requires ADK to be installed first
    if !is_adk_installed() {
        println!("ERROR: ADK must be installed before WinPE Add-on");
        let _ = open_url(ADK_WINPE_ADDON_URL);
        return InstallResult {
            success: false,
            method: "manual".to_string(),
            message: "ADK not installed. Install ADK first, then WinPE Add-on.".to_string(),
        };
    }

    // Method 1: Direct download and install (most reliable)
    println!("Downloading WinPE Add-on installer directly from Microsoft...");
    println!("URL: {}", ADK_WINPE_ADDON_URL);

    let temp_dir = std::env::temp_dir();
    let installer_path = temp_dir.join("adkwinpesetup.exe");

    match download_file(ADK_WINPE_ADDON_URL, &installer_path) {
        Ok(_) => {
            println!("Running WinPE Add-on installer silently...");
            println!("Command: {} /quiet /features + /ceip off", installer_path.display());
            println!("This may take several minutes. Please wait...");

            // Run installer silently with all features
            // /quiet = silent mode
            // /features + = install all features
            // /ceip off = disable telemetry
            let install_result = Command::new(&installer_path)
                .args(["/quiet", "/features", "+", "/ceip", "off"])
                .output();

            match install_result {
                Ok(out) => {
                    let exit_code = out.status.code().unwrap_or(-1);
                    println!("Installer exit code: {}", exit_code);

                    if !out.stdout.is_empty() {
                        println!("Installer stdout: {}", String::from_utf8_lossy(&out.stdout));
                    }
                    if !out.stderr.is_empty() {
                        println!("Installer stderr: {}", String::from_utf8_lossy(&out.stderr));
                    }

                    // The installer may spawn child processes and return immediately
                    // Wait for installation to complete
                    println!("Waiting for installation to complete...");
                    for i in 1..=36 {  // Wait up to 3 minutes
                        std::thread::sleep(std::time::Duration::from_secs(5));

                        if is_winpe_addon_installed() {
                            println!("WinPE Add-on installation verified after ~{}s", i * 5);
                            let _ = std::fs::remove_file(&installer_path);
                            return InstallResult {
                                success: true,
                                method: "direct_install".to_string(),
                                message: "WinPE Add-on installed successfully".to_string(),
                            };
                        }

                        if i % 6 == 0 {  // Every 30 seconds
                            println!("Still waiting... ({}s elapsed)", i * 5);
                        }
                    }

                    println!("Timeout waiting for installation to complete");
                }
                Err(e) => {
                    println!("Failed to run installer: {}", e);
                }
            }

            // Clean up installer
            let _ = std::fs::remove_file(&installer_path);
        }
        Err(e) => {
            println!("Direct download failed: {}", e);
        }
    }

    // Method 2: Try winget as fallback
    println!("\nDirect install didn't work, trying winget as fallback...");
    if is_winget_available() {
        let (success, _stdout, stderr) = install_via_winget(WINGET_WINPE_ADDON_ID);

        if success {
            println!("Winget reported success, verifying...");
            std::thread::sleep(std::time::Duration::from_secs(10));

            if is_winpe_addon_installed() {
                return InstallResult {
                    success: true,
                    method: "winget".to_string(),
                    message: "WinPE Add-on installed via winget".to_string(),
                };
            }
        } else {
            println!("Winget failed: {}", stderr);
        }
    }

    // Method 3: Final fallback - open browser
    println!("\nAutomatic installation failed. Opening browser for manual download...");
    let _ = open_url(ADK_WINPE_ADDON_URL);
    InstallResult {
        success: false,
        method: "manual".to_string(),
        message: "Auto-install failed. Browser opened for manual download.".to_string(),
    }
}

/// Install 7-Zip
pub fn install_7zip() -> InstallResult {
    println!("\n--- Installing 7-Zip ---");

    // Check if already installed
    if find_7zip().is_some() {
        println!("7-Zip already installed");
        return InstallResult {
            success: true,
            method: "already_installed".to_string(),
            message: "7-Zip already installed".to_string(),
        };
    }

    // Try winget
    if is_winget_available() {
        let (success, _stdout, stderr) = install_via_winget(WINGET_7ZIP_ID);
        if success {
            return InstallResult {
                success: true,
                method: "winget".to_string(),
                message: "7-Zip installed successfully via winget".to_string(),
            };
        }
        println!("winget failed: {}", stderr);
    }

    // Fallback to browser
    let _ = open_url(SEVEN_ZIP_DOWNLOAD_URL);
    InstallResult {
        success: false,
        method: "manual".to_string(),
        message: "Browser opened with 7-Zip download page. Please install manually.".to_string(),
    }
}

// ============================================================================
// MASTER INSTALLER - INSTALL ALL MISSING DEPENDENCIES
// ============================================================================

/// Install all missing dependencies required for PE building
/// This is the main function to call when user clicks "Install Dependencies"
#[allow(dead_code)]
pub fn install_all_dependencies() -> DependencyInstallResult {
    println!("\n========================================");
    println!("Installing All Missing Dependencies");
    println!("========================================");
    println!("This will install: ADK, WinPE Add-on, 7-Zip");
    println!("");

    let mut all_success = true;
    let mut next_steps = Vec::new();

    // Check for winget availability first
    let has_winget = is_winget_available();
    if has_winget {
        println!("[INFO] winget is available - will use for automatic installation");
    } else {
        println!("[INFO] winget not available - will open browser for manual downloads");
    }

    // Install 7-Zip first (needed for other operations)
    let seven_zip_result = install_7zip();
    if !seven_zip_result.success && seven_zip_result.method == "manual" {
        all_success = false;
        next_steps.push("Install 7-Zip from the download page that opened".to_string());
    }

    // Install ADK
    let adk_result = install_adk();
    if !adk_result.success && adk_result.method == "manual" {
        all_success = false;
        next_steps.push("Install Windows ADK from the download page that opened".to_string());
    }

    // Install WinPE Add-on (must be after ADK)
    let winpe_addon_result = install_winpe_addon();
    if !winpe_addon_result.success && winpe_addon_result.method == "manual" {
        all_success = false;
        next_steps.push("Install WinPE Add-on from the download page that opened".to_string());
    }

    // Build summary
    let summary = if all_success {
        "All dependencies installed successfully!".to_string()
    } else if next_steps.is_empty() {
        "Some dependencies may need manual installation.".to_string()
    } else {
        format!("{} manual installation(s) required. Click Detect after installing.", next_steps.len())
    };

    if !next_steps.is_empty() {
        next_steps.push("Click 'Detect' again after installing to verify".to_string());
    }

    println!("\n========================================");
    println!("Installation Summary:");
    println!("  7-Zip: {} ({})",
        if seven_zip_result.success { "OK" } else { "MANUAL" },
        seven_zip_result.method);
    println!("  ADK: {} ({})",
        if adk_result.success { "OK" } else { "MANUAL" },
        adk_result.method);
    println!("  WinPE Add-on: {} ({})",
        if winpe_addon_result.success { "OK" } else { "MANUAL" },
        winpe_addon_result.method);
    println!("========================================\n");

    DependencyInstallResult {
        all_success,
        adk_result,
        winpe_addon_result,
        seven_zip_result,
        summary,
        next_steps,
    }
}

/// Open download pages for all missing dependencies (manual installation)
#[allow(dead_code)]
pub fn open_all_download_pages(deps: &DependencyCheckResult) {
    println!("Opening download pages for missing dependencies...");

    if !deps.seven_zip_available {
        let _ = open_url(SEVEN_ZIP_DOWNLOAD_URL);
    }

    if !deps.adk_installed {
        let _ = open_url(ADK_DOWNLOAD_URL);
    }

    if !deps.winpe_addon_installed {
        let _ = open_url(ADK_WINPE_ADDON_URL);
    }
}

/// Run ADK's copype.cmd to create a WinPE base working directory
///
/// copype creates a properly configured WinPE with:
/// - boot.wim that uses winpeshl.ini (not Windows Setup)
/// - Correct media folder structure
/// - Boot files (bootmgr, BCD, EFI files)
///
/// # Arguments
/// * `architecture` - Target architecture: amd64, x86, or arm64
/// * `work_dir` - Directory where WinPE will be created
/// * `progress` - Progress callback function
///
/// # Returns
/// Ok(()) on success, Err with message on failure
pub fn run_copype(
    architecture: &str,
    work_dir: &Path,
    progress: impl Fn(i32, &str),
) -> Result<(), String> {
    progress(5, "Detecting ADK installation...");

    // Find ADK Deployment Tools path
    let adk_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools"),
    ];

    let deploy_tools = adk_paths.iter()
        .find(|p| p.exists())
        .ok_or_else(|| "ADK Deployment Tools not found. Please install Windows ADK.".to_string())?;

    let dandisenv_path = deploy_tools.join("DandISetEnv.bat");
    if !dandisenv_path.exists() {
        return Err("DandISetEnv.bat not found in ADK Deployment Tools".to_string());
    }

    println!("Found ADK Deployment Tools at: {}", deploy_tools.display());
    progress(8, "Running copype to create WinPE base...");

    // Clean existing work directory
    if work_dir.exists() {
        println!("Removing existing work directory...");
        fs::remove_dir_all(work_dir)
            .map_err(|e| format!("Failed to remove existing work directory: {}", e))?;
    }

    // Create parent directory if needed
    if let Some(parent) = work_dir.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create parent directory: {}", e))?;
    }

    // Create a batch file to run copype in the correct environment
    // copype must be run from the Deployment Tools environment
    let batch_content = format!(
        r#"@echo off
call "{}"
copype {} "{}"
exit /b %ERRORLEVEL%
"#,
        dandisenv_path.display(),
        architecture,
        work_dir.display()
    );

    let temp_batch = std::env::temp_dir().join("masterbooter_copype.bat");
    fs::write(&temp_batch, &batch_content)
        .map_err(|e| format!("Failed to create copype batch file: {}", e))?;

    println!("Running copype {} to {}...", architecture, work_dir.display());
    progress(10, &format!("Creating WinPE {} base...", architecture));

    // Run the batch file
    let output = Command::new("cmd")
        .args(["/c", &temp_batch.to_string_lossy()])
        .output()
        .map_err(|e| format!("Failed to run copype: {}", e))?;

    // Clean up batch file
    let _ = fs::remove_file(&temp_batch);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "copype failed with exit code {:?}\nStdout: {}\nStderr: {}",
            output.status.code(),
            stdout,
            stderr
        ));
    }

    // Verify boot.wim was created
    let boot_wim = work_dir.join("media").join("sources").join("boot.wim");
    if !boot_wim.exists() {
        return Err(format!(
            "copype completed but boot.wim not found at expected location: {}",
            boot_wim.display()
        ));
    }

    println!("copype completed successfully!");
    println!("  boot.wim: {}", boot_wim.display());
    progress(15, "WinPE base created successfully");

    Ok(())
}

// ============================================
// ISO BUILDING
// ============================================

/// Configuration for building a WinPE ISO
///
/// This enhanced configuration includes all the options from
/// AMPIPIT, GhostWin, and Windows Setup Helper:
/// - ADK package selection
/// - PE fixes (DPI, WallpaperHost, etc.)
/// - Driver injection
/// - Tool injection
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PeBuildConfig {
    // ============================================
    // BASIC OPTIONS
    // ============================================
    pub source_path: PathBuf,       // WinRE.wim or extracted ISO
    pub output_path: PathBuf,       // Output ISO file path
    pub architecture: String,       // amd64, x86, or arm64 (default: amd64)
    pub volume_label: String,       // ISO volume label (default: MASTERBOOTER)

    // ============================================
    // OUTPUT OPTIONS (NEW)
    // ============================================
    pub output_type: String,        // "ISO", "USB", or "VHD"
    pub use_uefi_2023_ca: bool,     // Use UEFI 2023 CA signed boot manager
    pub backup_original: bool,      // Backup original WinRE before modifying (Local RE mode)

    // ============================================
    // SHELL CONFIGURATION (NEW)
    // ============================================
    pub default_shell: String,      // "WinXShell", "Explorer++", or "CMD"

    // ============================================
    // CONTENT OPTIONS
    // ============================================
    pub include_drivers: bool,      // Include system drivers
    pub include_tools: bool,        // Include MasterBooter tools
    pub driver_paths: Vec<PathBuf>, // Paths to driver folders to inject
    pub enable_wifi: bool,          // Inject WLAN service for WiFi support

    // ============================================
    // ADK PACKAGES
    // Toggleable optional components
    // ============================================
    pub install_packages: bool,     // Whether to install ADK packages at all
    pub enabled_packages: Vec<String>,  // List of package IDs to install

    // ============================================
    // PE FIXES
    // Workarounds for WinPE quirks
    // ============================================
    pub apply_fixes: bool,          // Whether to apply PE fixes at all
    pub enabled_fixes: Vec<String>, // List of fix IDs to apply
    pub fix_options: FixOptions,    // Additional options for fixes (e.g., resolution)

    // ============================================
    // DRY RUN MODE
    // ============================================
    pub dry_run: bool,              // If true, validate everything but skip actual operations
}

impl Default for PeBuildConfig {
    /// Create a default configuration with recommended settings
    ///
    /// This enables the most commonly needed packages and fixes
    /// based on what AMPIPIT, GhostWin, and Windows Setup Helper use.
    fn default() -> Self {
        Self {
            source_path: PathBuf::new(),
            output_path: PathBuf::new(),
            architecture: "amd64".to_string(),
            volume_label: "MASTERBOOTER".to_string(),

            // Output options (new)
            output_type: "ISO".to_string(),
            use_uefi_2023_ca: true,
            backup_original: true,

            // Shell configuration (new)
            default_shell: "WinXShell".to_string(),

            include_drivers: true,
            include_tools: true,
            driver_paths: Vec::new(),
            enable_wifi: true,

            // Enable package installation with defaults
            install_packages: true,
            enabled_packages: adk_packages::get_default_enabled_packages(),

            // Enable fixes with defaults
            apply_fixes: true,
            enabled_fixes: pe_fixes::get_default_enabled_fixes(),
            fix_options: FixOptions::default(),

            dry_run: false,
        }
    }
}

#[allow(dead_code)]
impl PeBuildConfig {
    /// Create a minimal configuration (no packages, no fixes)
    ///
    /// Use this for a basic PE build without customization.
    pub fn minimal(source: PathBuf, output: PathBuf) -> Self {
        Self {
            source_path: source,
            output_path: output,
            architecture: "amd64".to_string(),
            volume_label: "MASTERBOOTER".to_string(),

            // Output options
            output_type: "ISO".to_string(),
            use_uefi_2023_ca: true,
            backup_original: true,
            default_shell: "CMD".to_string(),

            include_drivers: false,
            include_tools: true,
            driver_paths: Vec::new(),
            enable_wifi: false,

            install_packages: false,
            enabled_packages: Vec::new(),

            apply_fixes: true,
            enabled_fixes: vec![
                "dpi_scaling".to_string(),
                "wallpaper_host".to_string(),
            ],
            fix_options: FixOptions::default(),

            dry_run: false,
        }
    }

    /// Create a full configuration (all packages, all fixes)
    ///
    /// Use this for the most feature-rich PE build.
    pub fn full(source: PathBuf, output: PathBuf) -> Self {
        Self {
            source_path: source,
            output_path: output,
            architecture: "amd64".to_string(),
            volume_label: "MASTERBOOTER".to_string(),

            // Output options
            output_type: "ISO".to_string(),
            use_uefi_2023_ca: true,
            backup_original: true,
            default_shell: "WinXShell".to_string(),

            include_drivers: true,
            include_tools: true,
            driver_paths: Vec::new(),
            enable_wifi: true,

            install_packages: true,
            enabled_packages: adk_packages::get_all_packages()
                .iter()
                .map(|p| p.id.to_string())
                .collect(),

            apply_fixes: true,
            enabled_fixes: pe_fixes::get_all_fixes()
                .iter()
                .map(|f| f.id.to_string())
                .collect(),
            fix_options: FixOptions::default(),

            dry_run: false,
        }
    }
}

/// Result of the ISO build process
#[derive(Debug)]
#[allow(dead_code)]
pub struct PeBuildResult {
    pub success: bool,
    pub message: String,
    pub output_path: Option<PathBuf>,
}

/// ADK Version Information
/// The WinPE version must match the ADK version for package installation.
/// NOTE: The ADK installer shows "Windows 10" in its title bar and installs to
/// "C:\Program Files (x86)\Windows Kits\10\" — the "10" is the KIT version number,
/// NOT the target OS. The latest ADK fully supports Windows 11.
///
/// Windows ADK 10.1.26100.2454 (December 2024) — LATEST, used by MasterBooter
///   Supports: Windows 11 25H2/24H2 + all earlier Windows 10/11
///   ADK: https://go.microsoft.com/fwlink/?linkid=2289980
///   WinPE Addon: https://go.microsoft.com/fwlink/?linkid=2289981
///
/// Older versions (for reference only):
///   Windows ADK for Windows 11 version 24H2 - 10.1.26100.1
///     ADK: https://go.microsoft.com/fwlink/?linkid=2271337
///   Windows ADK for Windows 11 version 23H2 - 10.1.25398.1
///     ADK: https://go.microsoft.com/fwlink/?linkid=2243390
///   Windows ADK for Windows 11 version 22H2 - 10.1.22621.1
///     ADK: https://go.microsoft.com/fwlink/?linkid=2196127

/// Find oscdimg.exe from the Windows ADK
/// oscdimg is used to create bootable ISO files
fn find_oscdimg() -> Option<PathBuf> {
    let adk_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\oscdimg.exe"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\oscdimg.exe"),
    ];

    for path in adk_paths {
        if path.exists() {
            return Some(path);
        }
    }

    None
}

/// Run MakeWinPEMedia to create a bootable ISO
///
/// MakeWinPEMedia is the proper ADK tool for creating bootable WinPE media.
/// It automatically handles boot files (etfsboot.com, efisys.bin) and creates
/// a properly configured bootable ISO.
///
/// # Arguments
/// * `work_dir` - The copype working directory (contains media, fwfiles, mount folders)
/// * `output_path` - Path for the output ISO file
/// * `use_uefi_2023_ca` - Use UEFI 2023 CA signed boot manager (/bootex flag)
fn run_makewinpemedia(
    work_dir: &Path,
    output_path: &Path,
    use_uefi_2023_ca: bool,
) -> Result<(), String> {
    // Find ADK Deployment Tools path
    let deploy_tools_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools"),
    ];

    let deploy_tools = deploy_tools_paths.iter()
        .find(|p| p.exists())
        .ok_or_else(|| "ADK Deployment Tools not found".to_string())?;

    let dandisenv_path = deploy_tools.join("DandISetEnv.bat");
    if !dandisenv_path.exists() {
        return Err("DandISetEnv.bat not found".to_string());
    }

    println!("Using MakeWinPEMedia to create bootable ISO...");

    // Delete existing output file
    if output_path.exists() {
        println!("Removing existing output file...");
        let _ = fs::remove_file(output_path);
    }

    // Create parent directory if needed
    if let Some(parent) = output_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // IMPORTANT: Ensure output path doesn't exist as a directory
    // This prevents the "folder instead of ISO" bug
    if output_path.is_dir() {
        println!("Warning: Output path exists as a directory, removing it...");
        let _ = fs::remove_dir_all(output_path);
    }

    // Build the batch file to run MakeWinPEMedia in the correct environment
    // Note: We echo output for debugging purposes
    let bootex_flag = if use_uefi_2023_ca { " /bootex" } else { "" };
    let batch_content = format!(
        r#"@echo on
echo MasterBooter: Starting MakeWinPEMedia...
echo Working directory: {}
echo Output path: {}
call "{}"
if errorlevel 1 (
    echo MasterBooter: DandISetEnv.bat failed with error %errorlevel%
    exit /b %errorlevel%
)
echo MasterBooter: Running MakeWinPEMedia...
MakeWinPEMedia /ISO "{}" "{}"{}
set EXITCODE=%errorlevel%
echo MasterBooter: MakeWinPEMedia exit code: %EXITCODE%
exit /b %EXITCODE%"#,
        work_dir.display(),
        output_path.display(),
        dandisenv_path.display(),
        work_dir.display(),
        output_path.display(),
        bootex_flag
    );

    let temp_batch = std::env::temp_dir().join("masterbooter_makewinpemedia.bat");
    fs::write(&temp_batch, &batch_content)
        .map_err(|e| format!("Failed to create batch file: {}", e))?;

    println!("Running: MakeWinPEMedia /ISO \"{}\" \"{}\"{}",
             work_dir.display(), output_path.display(), bootex_flag);

    // Run the batch file and capture output
    let output = Command::new("cmd")
        .args(["/c", &temp_batch.to_string_lossy()])
        .output()
        .map_err(|e| format!("Failed to run MakeWinPEMedia: {}", e))?;

    // Always print stdout for debugging
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    println!("MakeWinPEMedia stdout:\n{}", stdout);
    if !stderr.is_empty() {
        println!("MakeWinPEMedia stderr:\n{}", stderr);
    }

    // Clean up batch file
    let _ = fs::remove_file(&temp_batch);

    if !output.status.success() {
        return Err(format!(
            "MakeWinPEMedia failed with exit code {:?}\nOutput: {}\n{}",
            output.status.code(),
            stdout,
            stderr
        ));
    }

    // Verify ISO was created as a FILE (not a directory)
    if output_path.is_dir() {
        // Something created a directory instead of an ISO file - this is an error
        println!("ERROR: A directory was created instead of an ISO file!");
        let _ = fs::remove_dir_all(output_path); // Clean up the errant directory
        return Err("MakeWinPEMedia created a directory instead of an ISO file. This is a bug.".to_string());
    }

    if !output_path.is_file() {
        // Log the MakeWinPEMedia output for debugging
        return Err("MakeWinPEMedia completed but ISO was not created. Check that ADK is properly installed.".to_string());
    }

    println!("ISO created successfully: {}", output_path.display());
    Ok(())
}

/// Build a WinPE ISO from the given configuration
///
/// This is a complex process that involves:
/// 1. Detecting ADK and using copype for PE creation (preferred)
/// 2. Falling back to ISO extraction if creating RE or ADK not available
/// 3. Customizing the WIM (adding tools, packages, fixes)
/// 4. Building the ISO with oscdimg
///
/// IMPORTANT: For WinPE creation, ADK must be installed. copype creates a
/// properly configured PE that uses winpeshl.ini, unlike boot.wim from a
/// Windows ISO which is designed for Windows Setup.
///
/// Returns a progress callback that can be used to track progress
pub fn build_pe_iso(
    config: &PeBuildConfig,
    progress_callback: impl Fn(i32, &str) + Send + 'static,
) -> PeBuildResult {
    println!("Starting WinPE ISO build...");
    println!("Source: {}", config.source_path.display());
    println!("Output: {}", config.output_path.display());

    // ============================================
    // STEP 0: Pre-flight validation and cleanup
    // ============================================
    progress_callback(0, "Validating build configuration...");

    // Force-unmount any stale WIM mounts from previous failed builds
    // (Based on AMPIPIT's force_unmount() at build start)
    if !config.dry_run {
        force_unmount_stale_mounts();
    }

    // Validate configuration (runs in both normal and dry-run mode)
    let validation = validate_build_config(config);
    if !validation.valid {
        let error_summary = validation.errors.join("\n\n");
        return PeBuildResult {
            success: false,
            message: format!("Build configuration is invalid:\n\n{}", error_summary),
            output_path: None,
        };
    }
    // Log warnings but continue
    for warning in &validation.warnings {
        println!("Warning: {}", warning);
    }

    progress_callback(1, "Initializing build...");

    // ============================================
    // STEP 1: Check ADK and decide build strategy
    // ============================================
    // For WinPE: MUST use ADK's copype (creates proper PE with winpeshl.ini)
    // For WinRE: Can extract from ISO (recovery environment)

    let adk_info = detect_adk();
    let is_re_mode = config.source_path.to_string_lossy().contains("winre")
        || config.source_path.to_string_lossy().to_lowercase().contains("recovery");

    // Determine if source is an ISO or WIM file
    let source_ext = config.source_path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_wim = source_ext == "wim";

    // Use copype for PE creation when ADK is available
    let use_copype = adk_info.found && !is_re_mode && !is_wim;

    println!("ADK found: {}", adk_info.found);
    println!("RE mode: {}", is_re_mode);
    println!("Using copype: {}", use_copype);

    // For PE mode without ADK, we cannot continue
    if !adk_info.found && !is_re_mode && !is_wim {
        return PeBuildResult {
            success: false,
            message: "Windows ADK is required to create WinPE.\n\n\
                What to do:\n\
                1. Download and install Windows ADK from Microsoft\n\
                2. Also install the 'WinPE Add-on for ADK'\n\
                3. Restart MasterBooter and try again\n\n\
                Alternative: Switch to 'Local RE' mode which uses the built-in \
                Recovery Environment and doesn't require ADK".to_string(),
            output_path: None,
        };
    }

    // Check for required tools
    let seven_zip = match find_7zip() {
        Some(path) => path,
        None => {
            return PeBuildResult {
                success: false,
                message: "7-Zip not found.\n\n\
                    What to do:\n\
                    1. Download 7-Zip from https://7-zip.org\n\
                    2. Install to the default location (C:\\Program Files\\7-Zip)\n\
                    3. Restart MasterBooter and try again".to_string(),
                output_path: None,
            };
        }
    };

    let oscdimg = find_oscdimg();
    if oscdimg.is_none() && !is_re_mode {
        return PeBuildResult {
            success: false,
            message: "oscdimg not found - cannot create bootable ISO.\n\n\
                What to do:\n\
                1. Install Windows ADK from Microsoft\n\
                2. During setup, select 'Deployment Tools'\n\
                3. Restart MasterBooter and try again\n\n\
                Alternative: Use Local RE mode which doesn't require oscdimg".to_string(),
            output_path: None,
        };
    }

    // ============================================
    // DRY-RUN: Report what would happen without doing it
    // ============================================
    if config.dry_run {
        progress_callback(50, "Dry run - analyzing build plan...");

        let mut plan = Vec::new();
        plan.push(format!("Source: {}", config.source_path.display()));
        plan.push(format!("Output: {}", config.output_path.display()));
        plan.push(format!("Architecture: {}", config.architecture));
        plan.push(format!("ADK found: {}", adk_info.found));
        plan.push(format!("Build strategy: {}", if use_copype { "copype (ADK)" } else if is_wim { "WIM source" } else { "ISO extraction" }));
        plan.push(format!("7-Zip: {}", seven_zip.display()));
        plan.push(format!("oscdimg: {}", oscdimg.as_ref().map(|p| p.display().to_string()).unwrap_or("not found".to_string())));

        if use_copype {
            plan.push("Would: Run copype to create WinPE base".to_string());
        } else if source_ext == "iso" {
            plan.push("Would: Extract boot.wim from ISO with 7-Zip".to_string());
            plan.push("Would: Extract boot files (bootmgr, EFI) from ISO".to_string());
        } else {
            plan.push("Would: Copy WIM file to working directory".to_string());
        }

        if config.install_packages || config.apply_fixes {
            plan.push(format!("Would: Mount WIM with DISM and customize (packages: {}, fixes: {})",
                config.install_packages, config.apply_fixes));
        } else {
            plan.push("Would: Mount WIM with DISM for basic customization (tools, shell)".to_string());
        }

        if config.include_drivers && !config.driver_paths.is_empty() {
            plan.push(format!("Would: Inject {} driver path(s)", config.driver_paths.len()));
        }

        if use_copype {
            plan.push("Would: Create ISO with MakeWinPEMedia".to_string());
        } else if oscdimg.is_some() {
            plan.push("Would: Create ISO with oscdimg (BIOS/UEFI dual boot)".to_string());
        } else {
            plan.push("Would: Save PE files as folder (no oscdimg available)".to_string());
        }

        plan.push("Would: Verify ISO integrity (5-point check)".to_string());

        progress_callback(100, "Dry run complete!");

        return PeBuildResult {
            success: true,
            message: format!("DRY RUN - Build plan:\n\n{}", plan.join("\n")),
            output_path: None,
        };
    }

    // ============================================
    // STEP 2: Create working directory / Run copype
    // ============================================
    let work_dir = std::env::temp_dir().join("MasterBooter_PE_Build");

    if use_copype {
        // Use ADK's copype to create a proper WinPE base
        progress_callback(5, "Creating WinPE base with ADK...");

        if let Err(e) = run_copype(&config.architecture, &work_dir, |pct, msg| {
            progress_callback(pct, msg);
        }) {
            // Cleanup work directory on failure
            let _ = fs::remove_dir_all(&work_dir);
            return PeBuildResult {
                success: false,
                message: format!("Failed to create WinPE base: {}\n\n\
                    What to do:\n\
                    1. Make sure Windows ADK and WinPE Add-on are fully installed\n\
                    2. Try running MasterBooter as Administrator\n\
                    3. Check that no other DISM operations are running", e),
                output_path: None,
            };
        }

        println!("copype completed - WinPE base created successfully");
    } else {
        // Traditional method: extract from ISO/WIM or modify existing RE
        progress_callback(5, "Creating working directory...");

        if work_dir.exists() {
            println!("Cleaning previous build...");
            let _ = fs::remove_dir_all(&work_dir);
        }
        if let Err(e) = fs::create_dir_all(&work_dir) {
            return PeBuildResult {
                success: false,
                message: format!("Failed to create working directory: {}", e),
                output_path: None,
            };
        }

        // Check if source exists
        progress_callback(8, "Checking source...");
        if !config.source_path.exists() {
            let _ = fs::remove_dir_all(&work_dir);
            return PeBuildResult {
                success: false,
                message: format!("Source file not found: {}\n\n\
                    What to do:\n\
                    1. Verify the source file path is correct\n\
                    2. Make sure the file hasn't been moved or deleted\n\
                    3. For WinRE, ensure Windows Recovery is enabled (reagentc /info)",
                    config.source_path.display()),
                output_path: None,
            };
        }
    }

    // ============================================
    // STEP 3: Set up PE media structure
    // ============================================
    // When using copype, the structure is already created at work_dir/media
    // When extracting from ISO, we need to create it

    let media_dir = work_dir.join("media");
    let boot_dir = media_dir.join("boot");
    let sources_dir = media_dir.join("sources");
    let efi_boot_dir = media_dir.join("EFI").join("Boot");
    let efi_microsoft_dir = media_dir.join("EFI").join("Microsoft").join("Boot");

    // If NOT using copype, create the folder structure
    let is_iso = source_ext == "iso";
    if !use_copype {
        progress_callback(10, "Creating PE folder structure...");

    for dir in [&boot_dir, &sources_dir, &efi_boot_dir, &efi_microsoft_dir] {
        if let Err(e) = fs::create_dir_all(dir) {
            let _ = fs::remove_dir_all(&work_dir);
            return PeBuildResult {
                success: false,
                message: format!("Failed to create directory: {}", e),
                output_path: None,
            };
        }
    }

    if is_iso {
        // Extract from Windows ISO
        progress_callback(15, "Extracting boot.wim from ISO...");
        println!("Extracting boot.wim...");

        // Extract boot.wim
        let output = Command::new(&seven_zip)
            .arg("e")
            .arg("-y")
            .arg(format!("-o{}", sources_dir.display()))
            .arg(&config.source_path)
            .arg("sources/boot.wim")
            .output();

        match output {
            Ok(out) => {
                if !out.status.success() {
                    let _ = fs::remove_dir_all(&work_dir);
                    return PeBuildResult {
                        success: false,
                        message: format!("Failed to extract boot.wim: {}",
                            String::from_utf8_lossy(&out.stderr)),
                        output_path: None,
                    };
                }
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&work_dir);
                return PeBuildResult {
                    success: false,
                    message: format!("Failed to run 7-Zip: {}", e),
                    output_path: None,
                };
            }
        }

        // Verify boot.wim was extracted
        let boot_wim = sources_dir.join("boot.wim");
        if !boot_wim.exists() {
            let _ = fs::remove_dir_all(&work_dir);
            return PeBuildResult {
                success: false,
                message: "boot.wim not found in ISO.\n\n\
                    What to do:\n\
                    1. Verify this is a valid Windows installation ISO\n\
                    2. The ISO must contain sources\\boot.wim\n\
                    3. Try a different Windows ISO (original, not modified)".to_string(),
                output_path: None,
            };
        }
        println!("boot.wim extracted successfully");

        // ============================================
        // CUSTOMIZE WIM - Inject tools and configure shell
        // ============================================
        progress_callback(20, "Customizing WinPE image...");
        println!("\n--- Starting WIM Customization ---\n");

        // Create a wrapper for progress that maps to our range (20-50%)
        let customize_result = customize_wim(&boot_wim, |pct, msg| {
            let mapped_pct = 20 + (pct * 30 / 100);
            progress_callback(mapped_pct, msg);
        });

        match customize_result {
            Ok(()) => {
                println!("WIM customization completed successfully!");
            }
            Err(e) => {
                // If customization fails, we can still continue with an uncustomized PE
                println!("Warning: WIM customization failed: {}", e);
                println!("Continuing with base PE (no custom shell/tools)...");
                // Don't return error - let user have a basic PE at least
            }
        }

        // Extract boot files
        progress_callback(55, "Extracting boot files...");
        println!("Extracting bootmgr and boot folder...");

        // Extract bootmgr
        let _ = Command::new(&seven_zip)
            .arg("e")
            .arg("-y")
            .arg(format!("-o{}", media_dir.display()))
            .arg(&config.source_path)
            .arg("bootmgr")
            .output();

        // Extract boot folder
        let _ = Command::new(&seven_zip)
            .arg("x")
            .arg("-y")
            .arg(format!("-o{}", media_dir.display()))
            .arg(&config.source_path)
            .arg("boot")
            .output();

        progress_callback(60, "Extracting EFI boot files...");
        println!("Extracting EFI folder...");

        // Extract EFI folder
        let _ = Command::new(&seven_zip)
            .arg("x")
            .arg("-y")
            .arg(format!("-o{}", media_dir.display()))
            .arg(&config.source_path)
            .arg("efi")
            .output();

        // Extract bootmgr.efi
        let _ = Command::new(&seven_zip)
            .arg("e")
            .arg("-y")
            .arg(format!("-o{}", media_dir.display()))
            .arg(&config.source_path)
            .arg("bootmgr.efi")
            .output();

        // ============================================
        // BCD FALLBACK (Step 8): Create BCD if not in ISO
        // ============================================
        // Some ISOs may not have a BCD, or extraction may fail.
        // Create one from scratch using bcdedit if needed.
        if !boot_dir.join("BCD").exists() {
            println!("BCD not found after ISO extraction - creating from scratch...");
            progress_callback(62, "Creating BCD store (BIOS)...");
            if let Err(e) = create_bcd_store(
                &boot_dir.join("BCD"),
                "\\sources\\boot.wim",
                false,
            ) {
                println!("Warning: Failed to create BIOS BCD: {}", e);
            }
        }

        // Also check for UEFI BCD
        let efi_bcd_path = efi_microsoft_dir.join("BCD");
        if !efi_bcd_path.exists() && efi_microsoft_dir.exists() {
            println!("UEFI BCD not found - creating from scratch...");
            progress_callback(63, "Creating BCD store (UEFI)...");
            if let Err(e) = create_bcd_store(
                &efi_bcd_path,
                "\\sources\\boot.wim",
                true,
            ) {
                println!("Warning: Failed to create UEFI BCD: {}", e);
            }
        }

        // ============================================
        // BOOT FILE FALLBACK (Step 9): Try ADK Oscdimg dir
        // ============================================
        // If etfsboot.com or efisys.bin not found in ISO, try the ADK
        let fwfiles_dir = std::env::temp_dir().join("MasterBooter_PE_Build").join("fwfiles");
        let etfsboot_check = boot_dir.join("etfsboot.com");
        if !etfsboot_check.exists() && !fwfiles_dir.join("etfsboot.com").exists() {
            // Try copying from ADK Oscdimg directory
            let adk_oscdimg_paths = [
                PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\etfsboot.com"),
                PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\etfsboot.com"),
            ];
            for adk_path in &adk_oscdimg_paths {
                if adk_path.exists() {
                    println!("Found etfsboot.com in ADK, copying...");
                    let _ = fs::create_dir_all(&fwfiles_dir);
                    let _ = fs::copy(adk_path, fwfiles_dir.join("etfsboot.com"));
                    // Also copy to boot dir for fallback
                    let _ = fs::copy(adk_path, &etfsboot_check);
                    break;
                }
            }
        }

        let efisys_check = efi_boot_dir.join("efisys.bin");
        if !efisys_check.exists() && !fwfiles_dir.join("efisys.bin").exists() {
            let adk_efisys_paths = [
                PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\efisys_noprompt.bin"),
                PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\efisys.bin"),
                PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\efisys_noprompt.bin"),
                PathBuf::from(r"C:\Program Files\Windows Kits\10\Assessment and Deployment Kit\Deployment Tools\amd64\Oscdimg\efisys.bin"),
            ];
            for adk_path in &adk_efisys_paths {
                if adk_path.exists() {
                    println!("Found efisys boot file in ADK, copying...");
                    let _ = fs::create_dir_all(&fwfiles_dir);
                    let dest_name = if adk_path.file_name().unwrap().to_str().unwrap().contains("noprompt") {
                        "efisys_noprompt.bin"
                    } else {
                        "efisys.bin"
                    };
                    let _ = fs::copy(adk_path, fwfiles_dir.join(dest_name));
                    break;
                }
            }
        }

    } else {
        // Source is a WIM file - just copy it
        progress_callback(15, "Copying WIM file...");
        let boot_wim = sources_dir.join("boot.wim");
        if let Err(e) = fs::copy(&config.source_path, &boot_wim) {
            let _ = fs::remove_dir_all(&work_dir);
            return PeBuildResult {
                success: false,
                message: format!("Failed to copy WIM file: {}", e),
                output_path: None,
            };
        }

        // We need boot files from somewhere - this won't be bootable without them
        progress_callback(50, "Warning: WIM source - boot files not available");
        println!("Warning: Building from WIM file - boot files may be missing");
    }
    } else {
        // ============================================
        // COPYPE PATH: Customize the WIM that copype created
        // ============================================
        // copype already created the proper PE structure with boot.wim
        // We just need to customize it (add tools, shell, packages)

        let boot_wim = sources_dir.join("boot.wim");
        if !boot_wim.exists() {
            let _ = fs::remove_dir_all(&work_dir);
            return PeBuildResult {
                success: false,
                message: "copype did not create boot.wim.\n\n\
                    What to do:\n\
                    1. The WinPE Add-on for ADK may not be installed\n\
                    2. Reinstall the 'Windows PE Add-on' from Microsoft\n\
                    3. Make sure the ADK version matches your Windows version".to_string(),
                output_path: None,
            };
        }

        progress_callback(20, "Customizing WinPE image...");
        println!("\n--- Starting WIM Customization (copype base) ---\n");

        // Use enhanced customization with config if packages or fixes are enabled
        // This adds ADK packages (PowerShell, WMI, .NET, etc.) and PE fixes (DPI, fonts, etc.)
        if config.install_packages || config.apply_fixes {
            println!("Using enhanced customization (packages: {}, fixes: {})",
                config.install_packages, config.apply_fixes);

            let customize_result = customize_wim_with_config(&boot_wim, config, |pct, msg| {
                let mapped_pct = 20 + (pct * 35 / 100);
                progress_callback(mapped_pct, msg);
            });

            match customize_result {
                Ok(()) => {
                    println!("Enhanced WIM customization completed successfully!");
                }
                Err(e) => {
                    // If enhanced customization fails, try basic customization
                    println!("Warning: Enhanced customization failed: {}", e);
                    println!("Falling back to basic customization...");

                    // Try basic customization
                    let basic_result = customize_wim(&boot_wim, |pct, msg| {
                        let mapped_pct = 20 + (pct * 35 / 100);
                        progress_callback(mapped_pct, msg);
                    });

                    if let Err(e2) = basic_result {
                        println!("Warning: Basic customization also failed: {}", e2);
                        println!("Continuing with unmodified PE...");
                    }
                }
            }
        } else {
            // Basic customization (tools only, no packages/fixes)
            let customize_result = customize_wim(&boot_wim, |pct, msg| {
                let mapped_pct = 20 + (pct * 35 / 100);
                progress_callback(mapped_pct, msg);
            });

            match customize_result {
                Ok(()) => {
                    println!("WIM customization completed successfully!");
                }
                Err(e) => {
                    println!("Warning: WIM customization failed: {}", e);
                    println!("Continuing with base PE (no custom shell/tools)...");
                }
            }
        }
    }

    progress_callback(60, "Verifying boot structure...");

    // Check what files we have
    let has_bootmgr = media_dir.join("bootmgr").exists();
    let has_boot_bcd = boot_dir.join("BCD").exists();
    let has_efi = efi_boot_dir.exists();
    let has_boot_wim = sources_dir.join("boot.wim").exists();

    println!("Boot structure check:");
    println!("  bootmgr: {}", has_bootmgr);
    println!("  boot/BCD: {}", has_boot_bcd);
    println!("  EFI folder: {}", has_efi);
    println!("  sources/boot.wim: {}", has_boot_wim);

    if !has_boot_wim {
        let _ = fs::remove_dir_all(&work_dir);
        return PeBuildResult {
            success: false,
            message: "boot.wim is missing after customization - cannot create bootable PE.\n\n\
                What to do:\n\
                1. The WIM customization may have corrupted the file\n\
                2. Try building again without customization options\n\
                3. Check that enough disk space is available in TEMP folder".to_string(),
            output_path: None,
        };
    }

    // ============================================
    // STEP 4.9: Disable driver signature enforcement in BCD
    // ============================================
    // WiFi protocol drivers (nwifi.sys, vwififlt.sys, wfplwfs.sys) are copied
    // from install.wim into the PE image. Without this BCD setting, Windows
    // rejects them at boot time with "cannot verify digital signature" errors.
    // This matches PhoenixPE's approach (700-BCD.script BypassDriverSigning).
    progress_callback(65, "Configuring boot options for driver compatibility...");

    // Disable signature enforcement in BIOS BCD (media/boot/BCD)
    let bios_bcd = boot_dir.join("BCD");
    if bios_bcd.exists() {
        if let Err(e) = disable_driver_signature_enforcement(&bios_bcd) {
            println!("Warning: Failed to set BIOS BCD driver bypass: {}", e);
        } else {
            println!("  BIOS BCD: driver signature enforcement disabled");
        }
    }

    // Disable signature enforcement in UEFI BCD (media/EFI/Microsoft/Boot/BCD)
    let uefi_bcd = efi_microsoft_dir.join("BCD");
    if uefi_bcd.exists() {
        if let Err(e) = disable_driver_signature_enforcement(&uefi_bcd) {
            println!("Warning: Failed to set UEFI BCD driver bypass: {}", e);
        } else {
            println!("  UEFI BCD: driver signature enforcement disabled");
        }
    }

    // Step 5: Build ISO
    progress_callback(70, "Building bootable ISO...");

    // When using copype, use MakeWinPEMedia (handles boot files automatically)
    // Otherwise fall back to oscdimg
    if use_copype {
        progress_callback(75, "Creating bootable ISO with MakeWinPEMedia...");

        if let Err(e) = run_makewinpemedia(&work_dir, &config.output_path, config.use_uefi_2023_ca) {
            let _ = fs::remove_dir_all(&work_dir);
            return PeBuildResult {
                success: false,
                message: format!("Failed to create ISO with MakeWinPEMedia: {}\n\n\
                    What to do:\n\
                    1. Try running MasterBooter as Administrator\n\
                    2. Check that the output path is writable\n\
                    3. Ensure no other DISM/ISO operations are running", e),
                output_path: None,
            };
        }

        // Verify the ISO we just created (Step 10: post-build verification)
        progress_callback(90, "Verifying ISO integrity...");
        let verification = verify_pe_iso(&config.output_path);
        let checks_passed = verification.checks.iter().filter(|(_, ok, _)| *ok).count();
        if verification.passed {
            println!("ISO verification passed ({}/5 checks)", checks_passed);
        } else {
            println!("ISO verification warnings:");
            for (name, ok, detail) in &verification.checks {
                if !ok {
                    println!("  - {} FAILED: {}", name, detail);
                }
            }
        }

        // Clean up work directory after successful MakeWinPEMedia build
        let _ = fs::remove_dir_all(&work_dir);

        progress_callback(95, "ISO created successfully!");

        // Include verification info in the result message
        let failed_checks: Vec<_> = verification.checks.iter()
            .filter(|(_, ok, _)| !ok)
            .collect();
        let verify_note = if !verification.passed {
            format!("\n\nNote: {} verification warning(s) - ISO may still work",
                failed_checks.len())
        } else {
            String::new()
        };

        return PeBuildResult {
            success: true,
            message: format!("WinPE ISO created successfully{}", verify_note),
            output_path: Some(config.output_path.clone()),
        };
    }

    // Fallback: Use oscdimg directly (for non-copype builds)
    if let Some(oscdimg_path) = oscdimg {
        println!("Using oscdimg to create ISO...");

        // Find etfsboot.com and efisys.bin for BIOS/UEFI boot
        let fwfiles_dir = work_dir.join("fwfiles");

        // Look for etfsboot.com (BIOS boot sector)
        let etfsboot_locations = [
            fwfiles_dir.join("etfsboot.com"),
            boot_dir.join("etfsboot.com"),
            media_dir.join("boot").join("etfsboot.com"),
        ];
        let etfsboot = etfsboot_locations.iter()
            .find(|p| p.exists())
            .cloned()
            .unwrap_or_else(|| boot_dir.join("etfsboot.com"));

        // Look for efisys.bin (UEFI boot sector)
        let efisys_locations = [
            fwfiles_dir.join("efisys.bin"),
            fwfiles_dir.join("efisys_noprompt.bin"),
            efi_boot_dir.join("efisys.bin"),
            efi_microsoft_dir.join("efisys.bin"),
        ];
        let efisys_path = efisys_locations.iter()
            .find(|p| p.exists())
            .cloned()
            .unwrap_or_else(|| efi_boot_dir.join("efisys.bin"));

        println!("Looking for boot files:");
        println!("  etfsboot.com: {} (exists: {})", etfsboot.display(), etfsboot.exists());
        println!("  efisys.bin: {} (exists: {})", efisys_path.display(), efisys_path.exists());

        progress_callback(75, "Creating BIOS/UEFI bootable ISO...");

        // Delete existing output file if it exists
        if config.output_path.exists() {
            println!("Removing existing output file...");
            if let Err(e) = fs::remove_file(&config.output_path) {
                println!("Warning: Could not remove existing file: {}", e);
            }
        }

        // Build oscdimg command
        // Format: oscdimg -bootdata:2#p0,e,b<bios_boot>#pEF,e,b<efi_boot> -m -o -u2 -udfver102 <source> <output>
        let mut cmd = Command::new(&oscdimg_path);

        // Add boot data if boot files exist
        if etfsboot.exists() && efisys_path.exists() {
            // Dual BIOS/UEFI boot
            let bootdata = format!(
                "2#p0,e,b{}#pEF,e,b{}",
                etfsboot.display(),
                efisys_path.display()
            );
            cmd.arg(format!("-bootdata:{}", bootdata));
        } else if etfsboot.exists() {
            // BIOS only
            cmd.arg(format!("-bootdata:1#p0,e,b{}", etfsboot.display()));
        } else if efisys_path.exists() {
            // UEFI only
            cmd.arg(format!("-bootdata:1#pEF,e,b{}", efisys_path.display()));
        } else {
            println!("Warning: No boot files found - ISO may not be bootable");
        }

        cmd.arg("-m");                          // Ignore max size
        cmd.arg("-o");                          // Optimize storage
        cmd.arg("-u2");                         // UDF filesystem
        cmd.arg("-udfver102");                  // UDF version 1.02
        cmd.arg(format!("-l{}", "MASTERBOOTER")); // Volume label (no space)
        cmd.arg(&media_dir);                    // Source folder
        cmd.arg(&config.output_path);           // Output ISO

        progress_callback(80, "Running oscdimg...");
        println!("Running: {:?}", cmd);

        let output = cmd.output();

        match output {
            Ok(out) => {
                progress_callback(95, "Finalizing...");

                if out.status.success() {
                    println!("ISO created successfully!");

                    // Verify the ISO we just created (Step 10: post-build verification)
                    progress_callback(90, "Verifying ISO integrity...");
                    let verification = verify_pe_iso(&config.output_path);
                    let checks_passed = verification.checks.iter().filter(|(_, ok, _)| *ok).count();
                    if verification.passed {
                        println!("ISO verification passed ({}/5 checks)", checks_passed);
                    } else {
                        println!("ISO verification warnings:");
                        for (name, ok, detail) in &verification.checks {
                            if !ok {
                                println!("  - {} FAILED: {}", name, detail);
                            }
                        }
                    }

                    // Get final ISO size
                    let iso_size = if let Ok(meta) = fs::metadata(&config.output_path) {
                        format_file_size(meta.len())
                    } else {
                        "Unknown".to_string()
                    };

                    // Clean up working directory
                    progress_callback(98, "Cleaning up...");
                    let _ = fs::remove_dir_all(&work_dir);

                    progress_callback(100, "Build complete!");

                    // Include verification info in the result message
                    let failed_checks: Vec<_> = verification.checks.iter()
                        .filter(|(_, ok, _)| !ok)
                        .collect();
                    let verify_note = if !verification.passed {
                        format!("\n\nNote: {} verification warning(s) - ISO may still work",
                            failed_checks.len())
                    } else {
                        String::new()
                    };

                    return PeBuildResult {
                        success: true,
                        message: format!("WinPE ISO created successfully!\nSize: {}\nPath: {}{}",
                            iso_size, config.output_path.display(), verify_note),
                        output_path: Some(config.output_path.clone()),
                    };
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    println!("oscdimg failed:");
                    println!("stdout: {}", stdout);
                    println!("stderr: {}", stderr);

                    let _ = fs::remove_dir_all(&work_dir);
                    return PeBuildResult {
                        success: false,
                        message: format!("oscdimg failed: {}\n{}", stdout, stderr),
                        output_path: None,
                    };
                }
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&work_dir);
                return PeBuildResult {
                    success: false,
                    message: format!("Failed to run oscdimg: {}", e),
                    output_path: None,
                };
            }
        }
    } else {
        // No oscdimg - save as folder
        progress_callback(90, "oscdimg not found...");

        // Copy media folder to output location (without .iso extension)
        let output_folder = config.output_path.with_extension("");

        progress_callback(95, "Saving PE files...");

        // Just leave the work folder and inform user
        let final_folder = output_folder.clone();
        if final_folder.exists() {
            let _ = fs::remove_dir_all(&final_folder);
        }

        if let Err(e) = fs::rename(&media_dir, &final_folder) {
            // If rename fails, try copy
            println!("Rename failed, copying files: {}", e);
            // For simplicity, just keep the temp folder
            progress_callback(100, "Build complete (folder only)");

            return PeBuildResult {
                success: true,
                message: format!(
                    "PE files created but ISO not built (oscdimg not found).\n\
                    Files saved to: {}\n\n\
                    To create bootable ISO:\n\
                    1. Install Windows ADK\n\
                    2. Run oscdimg manually, or\n\
                    3. Use Rufus/Ventoy with the boot.wim file",
                    work_dir.join("media").display()
                ),
                output_path: Some(work_dir.join("media")),
            };
        }

        progress_callback(100, "Build complete (folder only)");

        return PeBuildResult {
            success: true,
            message: format!(
                "PE files created but ISO not built (oscdimg not found).\n\
                Files saved to: {}\n\n\
                To create bootable ISO, install Windows ADK.",
                final_folder.display()
            ),
            output_path: Some(final_folder),
        };
    }
}

// ============================================
// HELPER FUNCTIONS
// ============================================

/// Format a file size in bytes to a human-readable string
fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Get the default output path for the ISO
pub fn get_default_output_path() -> PathBuf {
    // Use the user's Documents folder as the default
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        let documents = PathBuf::from(user_profile).join("Documents");
        if documents.exists() {
            return documents.join("MasterBooter_PE.iso");
        }
    }

    // Fallback to current directory
    PathBuf::from("MasterBooter_PE.iso")
}

/// Open a folder in Windows Explorer
pub fn open_folder(path: &Path) -> Result<(), String> {
    let folder = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    if !folder.exists() {
        // Create the folder if it doesn't exist
        let _ = fs::create_dir_all(folder);
    }

    Command::new("explorer")
        .arg(folder)
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    Ok(())
}

// ============================================
// FILE DIALOGS
// ============================================

/// Open a file dialog to select a Windows ISO file
/// Returns the selected path or None if cancelled
pub fn pick_iso_file() -> Option<PathBuf> {
    FileDialog::new()
        .set_title("Select Windows ISO")
        .add_filter("ISO Files", &["iso"])
        .add_filter("All Files", &["*"])
        .pick_file()
}

/// Open a save file dialog to select output ISO path
/// Returns the selected path or None if cancelled
pub fn pick_output_path() -> Option<PathBuf> {
    FileDialog::new()
        .set_title("Save WinPE ISO As")
        .add_filter("ISO Files", &["iso"])
        .set_file_name("MasterBooter_PE.iso")
        .save_file()
}

// ============================================
// 7-ZIP INTEGRATION
// ============================================

/// Find 7-Zip executable on the system
/// Checks common installation paths
pub fn find_7zip() -> Option<PathBuf> {
    let paths = [
        PathBuf::from(r"C:\Program Files\7-Zip\7z.exe"),
        PathBuf::from(r"C:\Program Files (x86)\7-Zip\7z.exe"),
    ];

    for path in paths {
        if path.exists() {
            return Some(path);
        }
    }

    // Check if 7z is in PATH
    if let Ok(output) = Command::new("where").arg("7z.exe").output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = path_str.lines().next() {
                let path = PathBuf::from(first_line.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }

    None
}

// ============================================
// ISO EXTRACTION
// ============================================

/// Information about a Windows ISO
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IsoInfo {
    pub path: PathBuf,
    pub has_boot_wim: bool,
    pub has_install_wim: bool,
    pub size_display: String,
}

/// Analyze a Windows ISO to see what it contains
/// Uses 7-Zip to list the contents
pub fn analyze_iso(iso_path: &Path) -> Result<IsoInfo, String> {
    let seven_zip = find_7zip().ok_or("7-Zip not found. Please install 7-Zip.")?;

    // List contents of ISO
    let output = Command::new(&seven_zip)
        .arg("l")                    // List
        .arg(iso_path)
        .output()
        .map_err(|e| format!("Failed to run 7-Zip: {}", e))?;

    if !output.status.success() {
        return Err(format!("7-Zip failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Check for boot.wim and install.wim/install.esd
    let has_boot_wim = stdout.contains("boot.wim");
    let has_install_wim = stdout.contains("install.wim") || stdout.contains("install.esd");

    // Get file size
    let size_display = if let Ok(metadata) = fs::metadata(iso_path) {
        format_file_size(metadata.len())
    } else {
        "Unknown".to_string()
    };

    Ok(IsoInfo {
        path: iso_path.to_path_buf(),
        has_boot_wim,
        has_install_wim,
        size_display,
    })
}

/// Extract boot.wim from a Windows ISO
/// Returns the path to the extracted boot.wim
#[allow(dead_code)]
pub fn extract_boot_wim(iso_path: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    let seven_zip = find_7zip().ok_or("7-Zip not found. Please install 7-Zip.")?;

    println!("Extracting boot.wim from ISO...");

    // Create destination directory
    fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create directory: {}", e))?;

    // Extract sources/boot.wim from ISO
    let output = Command::new(&seven_zip)
        .arg("e")                    // Extract (flat)
        .arg("-y")                   // Yes to all
        .arg(format!("-o{}", dest_dir.display()))
        .arg(iso_path)
        .arg("sources/boot.wim")     // Just extract boot.wim
        .output()
        .map_err(|e| format!("Failed to run 7-Zip: {}", e))?;

    if !output.status.success() {
        return Err(format!("Failed to extract boot.wim: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let boot_wim_path = dest_dir.join("boot.wim");
    if boot_wim_path.exists() {
        println!("Extracted boot.wim to: {}", boot_wim_path.display());
        Ok(boot_wim_path)
    } else {
        Err("boot.wim not found in ISO".to_string())
    }
}

/// Extract boot files from Windows ISO for BIOS/UEFI boot
/// Extracts: bootmgr, bootmgr.efi, boot folder, EFI folder
#[allow(dead_code)]
pub fn extract_boot_files(iso_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let seven_zip = find_7zip().ok_or("7-Zip not found. Please install 7-Zip.")?;

    println!("Extracting boot files from ISO...");

    // Files/folders needed for booting
    let boot_items = [
        "bootmgr",
        "bootmgr.efi",
        "boot/",
        "efi/",
    ];

    for item in &boot_items {
        let output = Command::new(&seven_zip)
            .arg("x")                    // Extract with paths
            .arg("-y")                   // Yes to all
            .arg(format!("-o{}", dest_dir.display()))
            .arg(iso_path)
            .arg(item)
            .output()
            .map_err(|e| format!("Failed to run 7-Zip: {}", e))?;

        // Don't fail if some items are missing (e.g., BIOS-only ISO won't have EFI)
        if output.status.success() {
            println!("Extracted: {}", item);
        }
    }

    Ok(())
}

// ============================================
// WIM MOUNTING AND CUSTOMIZATION
// ============================================
// These functions handle mounting a WIM file, injecting tools,
// configuring the shell, and unmounting with changes saved.
//
// This is the core of WinPE customization - we:
// 1. Mount the WIM to a folder (makes it editable)
// 2. Copy our tools into it
// 3. Configure winpeshl.ini to launch our shell
// 4. Create desktop shortcuts
// 5. Unmount and commit changes (saves everything)

use crate::tools::pe_tools;

/// Mount a WIM file using DISM
///
/// DISM (Deployment Image Servicing and Management) is a Windows tool
/// that can mount WIM files so we can modify their contents.
///
/// # Arguments
/// * `wim_path` - Path to the WIM file (e.g., boot.wim)
/// * `mount_path` - Folder to mount the WIM to
/// * `image_index` - Which image in the WIM to mount (usually 1 for boot.wim)
///
/// # Returns
/// Ok(()) on success, Err with message on failure
pub fn mount_wim(wim_path: &Path, mount_path: &Path, image_index: u32) -> Result<(), String> {
    println!("Mounting WIM: {} to {}", wim_path.display(), mount_path.display());

    // Create mount directory if it doesn't exist
    fs::create_dir_all(mount_path)
        .map_err(|e| format!("Failed to create mount directory: {}", e))?;

    // Run DISM to mount the WIM
    // Command: dism /Mount-Wim /WimFile:path /Index:1 /MountDir:path
    let output = Command::new("dism")
        .arg("/Mount-Wim")
        .arg(format!("/WimFile:{}", wim_path.display()))
        .arg(format!("/Index:{}", image_index))
        .arg(format!("/MountDir:{}", mount_path.display()))
        .output()
        .map_err(|e| format!("Failed to run DISM: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("DISM mount failed:\n{}\n{}\n\n\
            What to do:\n\
            1. Run MasterBooter as Administrator (DISM requires elevated privileges)\n\
            2. Run 'dism /Cleanup-Wim' in an admin command prompt to clear stale mounts\n\
            3. Check that no antivirus is blocking DISM operations", stdout, stderr));
    }

    println!("WIM mounted successfully");
    Ok(())
}

/// Unmount a WIM file and optionally commit changes
///
/// # Arguments
/// * `mount_path` - Folder where WIM is mounted
/// * `commit` - If true, save changes; if false, discard them
///
/// # Returns
/// Ok(()) on success, Err with message on failure
pub fn unmount_wim(mount_path: &Path, commit: bool) -> Result<(), String> {
    println!("Unmounting WIM from {} (commit: {})", mount_path.display(), commit);

    let commit_arg = if commit { "/Commit" } else { "/Discard" };

    // Run DISM to unmount
    // Command: dism /Unmount-Wim /MountDir:path /Commit (or /Discard)
    let output = Command::new("dism")
        .arg("/Unmount-Wim")
        .arg(format!("/MountDir:{}", mount_path.display()))
        .arg(commit_arg)
        .output()
        .map_err(|e| format!("Failed to run DISM: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("DISM unmount failed:\n{}\n{}", stdout, stderr));
    }

    println!("WIM unmounted successfully");
    Ok(())
}

/// Check if a WIM is currently mounted at a path
pub fn is_wim_mounted(mount_path: &Path) -> bool {
    // Check if the Windows folder exists in the mount path
    // This indicates a WIM is mounted there
    mount_path.join("Windows").exists()
}

/// Force cleanup any mounted WIMs (useful after crashes)
#[allow(dead_code)]
pub fn cleanup_mounted_wims() -> Result<(), String> {
    println!("Cleaning up any mounted WIMs...");

    let output = Command::new("dism")
        .arg("/Cleanup-Wim")
        .output()
        .map_err(|e| format!("Failed to run DISM cleanup: {}", e))?;

    // Don't fail if cleanup has nothing to do
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("DISM cleanup: {}", stdout);

    Ok(())
}

/// Inject PE tools into a mounted WIM
///
/// This copies all enabled PE tools into the mounted WIM image.
/// Tools are placed in X:\Tools\<ToolName>\ where X: is the PE drive.
///
/// # Arguments
/// * `mount_path` - Path where WIM is mounted
/// * `tools` - List of PE tools to inject (only enabled ones are copied)
/// * `progress` - Progress callback (tool_name, current, total)
///
/// # Returns
/// Ok with list of injected tool names, Err on failure
pub fn inject_pe_tools(
    mount_path: &Path,
    tools: &[pe_tools::PeTool],
    progress: impl Fn(&str, usize, usize),
) -> Result<Vec<String>, String> {
    println!("Injecting PE tools into mounted WIM...");

    // Create Tools folder in the mounted WIM
    // In WinPE, this will be X:\Tools
    let tools_dest = mount_path.join("Tools");
    fs::create_dir_all(&tools_dest)
        .map_err(|e| format!("Failed to create Tools folder: {}", e))?;

    // Get enabled tools that are present (downloaded)
    let enabled_tools: Vec<&pe_tools::PeTool> = tools.iter()
        .filter(|t| t.enabled && t.is_present)
        .collect();

    let total = enabled_tools.len();
    let mut injected = Vec::new();

    println!("Injecting {} tools...", total);

    for (index, tool) in enabled_tools.iter().enumerate() {
        progress(&tool.name, index + 1, total);
        println!("  [{}/{}] Copying {}...", index + 1, total, tool.name);

        // Destination folder for this tool
        let tool_dest = tools_dest.join(&tool.name);

        // Copy the entire tool folder
        if let Err(e) = copy_folder_recursive(&tool.folder_path, &tool_dest) {
            println!("    Warning: Failed to copy {}: {}", tool.name, e);
            continue;
        }

        injected.push(tool.name.clone());
        println!("    Copied {} to {}", tool.name, tool_dest.display());
    }

    // ============================================
    // ALSO COPY MASTERBOOTER ITSELF
    // ============================================
    // This allows running MasterBooter from within the PE environment
    let masterbooter_dest = tools_dest.join("MasterBooter");
    if let Err(e) = fs::create_dir_all(&masterbooter_dest) {
        println!("  Warning: Failed to create MasterBooter folder: {}", e);
    } else {
        // Get the path to the current MasterBooter executable
        if let Ok(exe_path) = std::env::current_exe() {
            let dest_exe = masterbooter_dest.join("masterbooter.exe");
            match fs::copy(&exe_path, &dest_exe) {
                Ok(_) => {
                    println!("  Copied MasterBooter to {}", dest_exe.display());
                    injected.push("MasterBooter".to_string());
                }
                Err(e) => {
                    println!("  Warning: Failed to copy MasterBooter: {}", e);
                }
            }

            // Copy only tool.toml manifests (not binaries — those are already
            // in Tools/<ToolName>/ so copying them again would double the size)
            if let Some(exe_dir) = exe_path.parent() {
                let pe_tools_src = exe_dir.join("pe_tools");
                if pe_tools_src.exists() {
                    let pe_tools_dest = masterbooter_dest.join("pe_tools");
                    if let Err(e) = copy_toml_manifests_only(&pe_tools_src, &pe_tools_dest) {
                        println!("  Note: Could not copy pe_tools manifests: {}", e);
                    }
                }
            }
        }
    }

    println!("Injected {} tools successfully", injected.len());
    Ok(injected)
}

/// Configure the WinPE shell to launch WinXShell (or another shell)
///
/// This creates/modifies winpeshl.ini which controls what runs at PE startup.
/// We configure it to:
/// 1. Run PENetwork (if enabled) for network connectivity
/// 2. Launch WinXShell as the main shell
///
/// # Arguments
/// * `mount_path` - Path where WIM is mounted
/// * `tools` - List of PE tools (to find shell and auto-launch tools)
///
/// # Returns
/// Ok(shell_name) on success, Err on failure
pub fn configure_pe_shell(
    mount_path: &Path,
    tools: &[pe_tools::PeTool],
) -> Result<String, String> {
    println!("Configuring PE shell with launcher script (AMPIPIT-style)...");

    // Find the enabled shell (WinXShell, etc.)
    let shell_tool = tools.iter()
        .find(|t| t.is_shell && t.enabled && t.is_present);

    let shell_name = match shell_tool {
        Some(tool) => tool.name.clone(),
        None => {
            println!("No shell tool enabled - PE will boot to cmd.exe");
            return Ok("cmd.exe".to_string());
        }
    };

    // Find auto-launch tools (like PENetwork)
    let auto_launch_tools: Vec<&pe_tools::PeTool> = tools.iter()
        .filter(|t| t.auto_launch && t.enabled && t.is_present)
        .collect();

    // ============================================
    // CREATE LAUNCHER SCRIPT (like AMPIPIT does)
    // ============================================
    // Instead of launching tools directly from winpeshl.ini,
    // we use a launcher script that sets up the environment first.
    // This is critical for WinXShell to work properly!

    // Create Launchers folder for our scripts
    let launchers_dir = mount_path.join("Tools").join("Launchers");
    fs::create_dir_all(&launchers_dir)
        .map_err(|e| format!("Failed to create Launchers folder: {}", e))?;

    // Build the launcher script content
    let mut launch_script = String::from("@echo off\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM MasterBooter WinPE Launcher\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM This script initializes the PE environment before launching the shell.\r\n");
    launch_script.push_str("REM Based on AMPIPIT's proven launcher approach.\r\n\r\n");

    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM STEP 1: INITIALIZE WINPE (CRITICAL!)\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM wpeinit loads drivers and initializes hardware\r\n");
    launch_script.push_str("echo Initializing WinPE environment...\r\n");
    launch_script.push_str("wpeinit\r\n\r\n");

    launch_script.push_str("REM Small delay for services to start\r\n");
    launch_script.push_str("ping 127.0.0.1 -n 3 > nul\r\n\r\n");

    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM STEP 1.5: LOAD DRIVERS\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM Auto-load .inf drivers from:\r\n");
    launch_script.push_str("REM   1. X:\\Drivers (baked into PE image during build)\r\n");
    launch_script.push_str("REM   2. USB drives: Drivers/ and MasterBooter/Drivers/ folders\r\n");
    launch_script.push_str("REM This enables WiFi, NVMe, and other hardware not in base WinPE.\r\n");
    launch_script.push_str("echo Loading additional drivers...\r\n");
    // First: load drivers baked into the PE image (X: = WinPE RAM disk)
    launch_script.push_str("if exist \"X:\\Drivers\" (\r\n");
    launch_script.push_str("    echo Loading built-in PE drivers...\r\n");
    launch_script.push_str("    for /r \"X:\\Drivers\" %%f in (*.inf) do (\r\n");
    launch_script.push_str("        drvload \"%%f\" >nul 2>&1\r\n");
    launch_script.push_str("    )\r\n");
    launch_script.push_str(")\r\n");
    // Then: scan all other drives for user-provided drivers on USB
    launch_script.push_str("for %%d in (C D E F G H I J K L M N O P Q R S T U V W Y Z) do (\r\n");
    launch_script.push_str("    if exist \"%%d:\\Drivers\" (\r\n");
    launch_script.push_str("        echo Found drivers on %%d:\r\n");
    launch_script.push_str("        for /r \"%%d:\\Drivers\" %%f in (*.inf) do (\r\n");
    launch_script.push_str("            drvload \"%%f\" >nul 2>&1\r\n");
    launch_script.push_str("        )\r\n");
    launch_script.push_str("    )\r\n");
    launch_script.push_str("    if exist \"%%d:\\MasterBooter\\Drivers\" (\r\n");
    launch_script.push_str("        echo Found drivers on %%d:\\MasterBooter\r\n");
    launch_script.push_str("        for /r \"%%d:\\MasterBooter\\Drivers\" %%f in (*.inf) do (\r\n");
    launch_script.push_str("            drvload \"%%f\" >nul 2>&1\r\n");
    launch_script.push_str("        )\r\n");
    launch_script.push_str("    )\r\n");
    launch_script.push_str(")\r\n");
    launch_script.push_str("echo Drivers loaded.\r\n\r\n");

    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM STEP 1.6: IMPROVE MOUSE/TOUCHPAD SPEED\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM WinPE defaults to slow mouse speed. Increase it for touchpads.\r\n");
    launch_script.push_str("REM MouseSpeed=2 enables acceleration, Threshold values control when it kicks in.\r\n");
    launch_script.push_str("reg add \"HKCU\\Control Panel\\Mouse\" /v MouseSpeed /t REG_SZ /d \"2\" /f >nul 2>&1\r\n");
    launch_script.push_str("reg add \"HKCU\\Control Panel\\Mouse\" /v MouseThreshold1 /t REG_SZ /d \"4\" /f >nul 2>&1\r\n");
    launch_script.push_str("reg add \"HKCU\\Control Panel\\Mouse\" /v MouseThreshold2 /t REG_SZ /d \"8\" /f >nul 2>&1\r\n");
    launch_script.push_str("reg add \"HKCU\\Control Panel\\Mouse\" /v MouseSensitivity /t REG_SZ /d \"14\" /f >nul 2>&1\r\n");
    launch_script.push_str("REM Apply immediately via SystemParametersInfo (rundll32 approach)\r\n");
    launch_script.push_str("rundll32 user32.dll,SystemParametersInfoA 113 0 14 0 >nul 2>&1\r\n\r\n");

    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM STEP 2: CREATE USER PROFILE FOLDERS\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM Many programs expect these folders to exist\r\n");
    launch_script.push_str("echo Creating user profile folders...\r\n");
    launch_script.push_str("mkdir X:\\Users\\Default\\AppData\\Local\\Temp 2>nul\r\n");
    launch_script.push_str("mkdir X:\\Users\\Default\\AppData\\Roaming 2>nul\r\n");
    launch_script.push_str("mkdir X:\\Users\\Default\\Desktop 2>nul\r\n");
    launch_script.push_str("mkdir X:\\Users\\Default\\Documents 2>nul\r\n");
    launch_script.push_str("mkdir X:\\Users\\Default\\Downloads 2>nul\r\n\r\n");

    launch_script.push_str("REM Also create in current USERPROFILE location\r\n");
    launch_script.push_str("mkdir \"%USERPROFILE%\\Downloads\" 2>nul\r\n");
    launch_script.push_str("mkdir \"%USERPROFILE%\\Documents\" 2>nul\r\n");
    launch_script.push_str("mkdir \"%USERPROFILE%\\Desktop\" 2>nul\r\n");
    launch_script.push_str("mkdir \"%USERPROFILE%\\AppData\\Local\\Temp\" 2>nul\r\n");
    launch_script.push_str("mkdir \"%USERPROFILE%\\AppData\\Roaming\" 2>nul\r\n\r\n");

    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM STEP 3: SET ENVIRONMENT VARIABLES\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("echo Setting up environment...\r\n");
    launch_script.push_str("set USERPROFILE=X:\\Users\\Default\r\n");
    launch_script.push_str("set APPDATA=X:\\Users\\Default\\AppData\\Roaming\r\n");
    launch_script.push_str("set LOCALAPPDATA=X:\\Users\\Default\\AppData\\Local\r\n");
    launch_script.push_str("set TEMP=X:\\Users\\Default\\AppData\\Local\\Temp\r\n");
    launch_script.push_str("set TMP=X:\\Users\\Default\\AppData\\Local\\Temp\r\n");
    launch_script.push_str("set HOMEPATH=\\Users\\Default\r\n");
    launch_script.push_str("set HOMEDRIVE=X:\r\n");
    launch_script.push_str("set HOME=X:\\Users\\Default\r\n\r\n");

    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM STEP 4: INITIALIZE NETWORK AND WIFI\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM Start the WLAN AutoConfig service for WiFi support.\r\n");
    launch_script.push_str("REM This requires WiFi/WLAN files to have been injected during build.\r\n");
    launch_script.push_str("REM The wlansvc service must be running for PENetwork to see WiFi adapters.\r\n");
    launch_script.push_str("echo Initializing network services...\r\n");
    launch_script.push_str("net start dot3svc 2>nul\r\n");
    launch_script.push_str("net start Eaphost 2>nul\r\n");
    launch_script.push_str("net start wlansvc 2>nul\r\n");
    launch_script.push_str("if %errorlevel% neq 0 (\r\n");
    launch_script.push_str("    echo   Note: WLAN service not available - WiFi requires injected WLAN files\r\n");
    launch_script.push_str(") else (\r\n");
    launch_script.push_str("    echo   WLAN service started - WiFi adapters should be available\r\n");
    launch_script.push_str(")\r\n\r\n");

    // --- Start netprofm with SystemSetupInProgress trick ---
    // WinPE sets HKLM\SYSTEM\Setup\SystemSetupInProgress = 1 which tells Windows
    // "we're in setup mode." The netprofm (Network List Manager) service refuses
    // to start properly while this flag is set. PhoenixPE's trick: temporarily
    // set it to 0 before starting netprofm, then restore it to 1 after.
    launch_script.push_str("REM Temporarily clear SystemSetupInProgress so netprofm will start.\r\n");
    launch_script.push_str("REM netprofm (Network List Manager) won't start in WinPE setup mode.\r\n");
    launch_script.push_str("REM PhoenixPE uses this trick: clear the flag, start the service, restore it.\r\n");
    launch_script.push_str("reg add \"HKLM\\SYSTEM\\Setup\" /v SystemSetupInProgress /t REG_DWORD /d 0 /f >nul 2>&1\r\n");
    launch_script.push_str("net start netprofm 2>nul\r\n");
    launch_script.push_str("net start NlaSvc 2>nul\r\n");
    launch_script.push_str("REM Restore SystemSetupInProgress for WinPE compatibility\r\n");
    launch_script.push_str("reg add \"HKLM\\SYSTEM\\Setup\" /v SystemSetupInProgress /t REG_DWORD /d 1 /f >nul 2>&1\r\n\r\n");

    launch_script.push_str("REM Give network adapters time to initialize after driver loading\r\n");
    launch_script.push_str("ping 127.0.0.1 -n 3 > nul\r\n\r\n");

    launch_script.push_str("REM Quick network check\r\n");
    launch_script.push_str("ping -n 1 -w 1000 8.8.8.8 >nul 2>&1\r\n");
    launch_script.push_str("if %errorlevel%==0 (\r\n");
    launch_script.push_str("    echo Network connected!\r\n");
    launch_script.push_str(") else (\r\n");
    launch_script.push_str("    echo No network yet - use PENetwork to configure WiFi or check Ethernet\r\n");
    launch_script.push_str(")\r\n\r\n");

    // Create proper .lnk shortcuts using PowerShell (if script exists)
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("REM STEP 4.5: CREATE DESKTOP SHORTCUTS\r\n");
    launch_script.push_str("REM ============================================\r\n");
    launch_script.push_str("if exist \"X:\\Tools\\Scripts\\CreateShortcuts.ps1\" (\r\n");
    launch_script.push_str("    echo Creating desktop shortcuts...\r\n");
    launch_script.push_str("    powershell -ExecutionPolicy Bypass -File \"X:\\Tools\\Scripts\\CreateShortcuts.ps1\" 2>nul\r\n");
    launch_script.push_str(")\r\n\r\n");

    // Add auto-launch tools (like PENetwork)
    if !auto_launch_tools.is_empty() {
        launch_script.push_str("REM ============================================\r\n");
        launch_script.push_str("REM STEP 5: LAUNCH AUTO-START TOOLS\r\n");
        launch_script.push_str("REM ============================================\r\n");

        for tool in &auto_launch_tools {
            let tool_path = format!("X:\\Tools\\{}\\{}", tool.name, tool.exe);
            launch_script.push_str(&format!("echo Starting {}...\r\n", tool.name));
            // Use /MIN so auto-launched tools start minimized (don't pop up over the shell)
            launch_script.push_str(&format!("if exist \"{}\" start /MIN \"{}\" \"{}\"\r\n", tool_path, tool.name, tool_path));
            launch_script.push_str("ping 127.0.0.1 -n 2 > nul\r\n\r\n");
            println!("  Auto-launch: {}", tool_path);
        }
    }

    // Add shell launch at the end
    if let Some(shell) = shell_tool {
        let shell_path = format!("X:\\Tools\\{}\\{}", shell.name, shell.exe);
        let shell_dir = format!("X:\\Tools\\{}", shell.name);
        launch_script.push_str("REM ============================================\r\n");
        launch_script.push_str("REM STEP 6: LAUNCH SHELL\r\n");
        launch_script.push_str("REM ============================================\r\n");
        launch_script.push_str(&format!("echo Launching {}...\r\n", shell.name));
        launch_script.push_str(&format!("if exist \"{}\" (\r\n", shell_path));

        // Shell-specific launch commands
        if shell.name == "WinXShell" {
            // WinXShell REQUIRES -winpe flag for proper WinPE operation!
            // Also set working directory to WinXShell folder for config files
            launch_script.push_str(&format!("    cd /d \"{}\"\r\n", shell_dir));
            // Use 'start' so the script doesn't wait, and pass -winpe flag
            launch_script.push_str(&format!("    start \"\" \"{}\" -winpe\r\n", shell_path));
            // Give WinXShell time to start before we exit
            launch_script.push_str("    ping 127.0.0.1 -n 5 > nul\r\n");
        } else if shell.name == "Explorer++" {
            // Explorer++ doesn't need special flags for PE
            launch_script.push_str(&format!("    cd /d \"{}\"\r\n", shell_dir));
            launch_script.push_str(&format!("    start \"\" \"{}\"\r\n", shell_path));
            launch_script.push_str("    ping 127.0.0.1 -n 5 > nul\r\n");
        } else {
            // Generic shell launch
            launch_script.push_str(&format!("    start \"\" \"{}\"\r\n", shell_path));
            launch_script.push_str("    ping 127.0.0.1 -n 3 > nul\r\n");
        }

        launch_script.push_str(") else (\r\n");
        launch_script.push_str(&format!("    echo ERROR: {} not found at {}\r\n", shell.name, shell_path));
        launch_script.push_str("    echo.\r\n");
        launch_script.push_str("    echo Available files in Tools folder:\r\n");
        launch_script.push_str("    dir X:\\Tools /s /b\r\n");
        launch_script.push_str("    echo.\r\n");
        launch_script.push_str("    echo Press any key to open command prompt...\r\n");
        launch_script.push_str("    pause > nul\r\n");
        launch_script.push_str("    cmd.exe\r\n");
        launch_script.push_str(")\r\n\r\n");
        println!("  Shell: {} (with -winpe flag)", shell_path);
    }

    // Add fallback if shell exits
    launch_script.push_str("REM If shell exits, keep cmd open\r\n");
    launch_script.push_str("echo.\r\n");
    launch_script.push_str("echo Shell exited. Press any key for command prompt...\r\n");
    launch_script.push_str("pause > nul\r\n");
    launch_script.push_str("cmd.exe\r\n");

    // Write the launcher script
    let launcher_path = launchers_dir.join("launch.cmd");
    fs::write(&launcher_path, &launch_script)
        .map_err(|e| format!("Failed to write launch.cmd: {}", e))?;

    println!("Created launcher script at {}", launcher_path.display());

    // ============================================
    // UPDATE WINPESHL.INI TO USE LAUNCHER
    // ============================================
    let winpeshl_path = mount_path.join("Windows").join("System32").join("winpeshl.ini");

    // winpeshl.ini just calls our launcher script
    let winpeshl_content = "[LaunchApps]\r\nX:\\Tools\\Launchers\\launch.cmd\r\n";

    fs::write(&winpeshl_path, winpeshl_content)
        .map_err(|e| format!("Failed to write winpeshl.ini: {}", e))?;

    println!("Created winpeshl.ini pointing to launcher script");

    Ok(shell_name)
}

/// Create desktop shortcuts for PE tools
///
/// Creates desktop items for WinXShell in WinPE. Uses two methods:
/// 1. .cmd batch files on desktop (reliable, works in all PE shells)
/// 2. PowerShell script to create proper .lnk files at startup (for icons)
///
/// # Arguments
/// * `mount_path` - Path where WIM is mounted
/// * `tools` - List of PE tools
///
/// # Returns
/// Ok with number of shortcuts created, Err on failure
pub fn create_pe_shortcuts(
    mount_path: &Path,
    tools: &[pe_tools::PeTool],
) -> Result<usize, String> {
    println!("Creating desktop shortcuts...");

    // Desktop location in WinPE (using Default user profile)
    // In mounted WIM: Users\Default\Desktop
    let desktop_path = mount_path.join("Users").join("Default").join("Desktop");
    fs::create_dir_all(&desktop_path)
        .map_err(|e| format!("Failed to create Desktop folder: {}", e))?;

    // Get tools that want shortcuts (not shells - they ARE the desktop)
    let shortcut_tools: Vec<&pe_tools::PeTool> = tools.iter()
        .filter(|t| t.create_shortcut && t.enabled && t.is_present && !t.is_shell)
        .collect();

    // ============================================
    // Create PowerShell script that generates proper .lnk shortcuts at boot
    // ============================================
    // .lnk files have proper icons and work with all PE shells.
    // The launcher script (launch.cmd) runs this PowerShell script at PE startup.
    let scripts_path = mount_path.join("Tools").join("Scripts");
    let _ = fs::create_dir_all(&scripts_path);

    let mut ps_script = String::from(
        "# MasterBooter Desktop Shortcuts Creator\r\n\
        # Creates proper .lnk shortcuts with icons for WinXShell\r\n\
        $WshShell = New-Object -ComObject WScript.Shell\r\n\
        $Desktop = [Environment]::GetFolderPath('Desktop')\r\n\
        if (-not $Desktop) { $Desktop = \"$env:USERPROFILE\\Desktop\" }\r\n\
        if (-not (Test-Path $Desktop)) { New-Item -ItemType Directory -Path $Desktop -Force | Out-Null }\r\n\r\n"
    );

    for tool in &shortcut_tools {
        let target_path = format!("X:\\Tools\\{}\\{}", tool.name, tool.exe);
        let tool_dir = format!("X:\\Tools\\{}", tool.name);

        ps_script.push_str(&format!(
            "# {}\r\n\
            $shortcut = $WshShell.CreateShortcut(\"$Desktop\\{}.lnk\")\r\n\
            $shortcut.TargetPath = \"{}\"\r\n\
            $shortcut.WorkingDirectory = \"{}\"\r\n\
            $shortcut.Description = \"{}\"\r\n\
            $shortcut.Save()\r\n\r\n",
            tool.name,
            tool.name,
            target_path.replace('\\', "\\\\"),
            tool_dir.replace('\\', "\\\\"),
            tool.description.replace('"', "'")
        ));
    }

    // Add MasterBooter shortcut
    ps_script.push_str(
        "# MasterBooter\r\n\
        $shortcut = $WshShell.CreateShortcut(\"$Desktop\\MasterBooter.lnk\")\r\n\
        $shortcut.TargetPath = \"X:\\Tools\\MasterBooter\\masterbooter.exe\"\r\n\
        $shortcut.WorkingDirectory = \"X:\\Tools\\MasterBooter\"\r\n\
        $shortcut.Description = \"MasterBooter - Windows Deployment Toolkit\"\r\n\
        $shortcut.Save()\r\n\r\n"
    );

    ps_script.push_str("Write-Host \"Desktop shortcuts created successfully.\"\r\n");

    let ps_path = scripts_path.join("CreateShortcuts.ps1");
    fs::write(&ps_path, &ps_script)
        .map_err(|e| format!("Failed to write shortcut script: {}", e))?;

    // Count: all tool shortcuts + MasterBooter shortcut
    let created = shortcut_tools.len() + 1;
    println!("  Created PowerShell shortcut script ({} shortcuts) at X:\\Tools\\Scripts\\CreateShortcuts.ps1", created);

    // ============================================
    // Create launchers folder with .cmd files (used by launch.cmd, NOT on Desktop)
    // ============================================
    let launchers_path = mount_path.join("Tools").join("Launchers");
    let _ = fs::create_dir_all(&launchers_path);

    for tool in &shortcut_tools {
        let batch_path = launchers_path.join(format!("{}.cmd", tool.name));
        let content = format!(
            "@echo off\r\ncd /d \"X:\\Tools\\{}\"\r\nstart \"\" \"X:\\Tools\\{}\\{}\"\r\n",
            tool.name, tool.name, tool.exe
        );
        let _ = fs::write(&batch_path, &content);
    }

    println!("Created {} desktop shortcuts (via PowerShell .lnk)", created);
    Ok(created)
}

/// Export a single image from a multi-image WIM to create a clean single-image WIM
///
/// Windows ISO boot.wim contains 2 images:
///   Index 1: Windows PE (minimal environment)
///   Index 2: Windows Setup (install wizard)
///
/// By default, the BCD boots Index 2 (Windows Setup), which is why you see
/// "Select driver to install" instead of our custom shell.
///
/// This function exports only the specified index to a new WIM file,
/// then replaces the original. This ensures the ISO boots our custom PE.
///
/// # Arguments
/// * `wim_path` - Path to the original boot.wim
/// * `index` - Which index to export (usually 1 for Windows PE)
fn export_single_image(wim_path: &Path, index: u32) -> Result<(), String> {
    println!("Exporting WIM Index {} to single-image file...", index);

    // Create temp path for the new WIM
    let temp_wim = wim_path.with_extension("wim.new");

    // Use DISM to export only the specified index
    // Command: dism /Export-Image /SourceImageFile:boot.wim /SourceIndex:1
    //                /DestinationImageFile:boot_new.wim /Compress:max
    let output = Command::new("dism")
        .arg("/Export-Image")
        .arg(format!("/SourceImageFile:{}", wim_path.display()))
        .arg(format!("/SourceIndex:{}", index))
        .arg(format!("/DestinationImageFile:{}", temp_wim.display()))
        .arg("/Compress:max")
        .output()
        .map_err(|e| format!("Failed to run DISM export: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("DISM export failed:\n{}\n{}", stdout, stderr));
    }

    // Verify the new WIM was created
    if !temp_wim.exists() {
        return Err("Export succeeded but new WIM file not found".to_string());
    }

    // Get sizes for logging
    let old_size = fs::metadata(wim_path).map(|m| m.len()).unwrap_or(0);
    let new_size = fs::metadata(&temp_wim).map(|m| m.len()).unwrap_or(0);
    println!("  Original WIM: {} bytes", old_size);
    println!("  Exported WIM: {} bytes", new_size);

    // Replace original with exported version
    // First, remove the original
    fs::remove_file(wim_path)
        .map_err(|e| format!("Failed to remove original WIM: {}", e))?;

    // Rename the new WIM to the original name
    fs::rename(&temp_wim, wim_path)
        .map_err(|e| format!("Failed to rename exported WIM: {}", e))?;

    println!("  Replaced boot.wim with single-image version");
    println!("  ISO will now boot to customized WinPE!");

    Ok(())
}

/// Copy a folder recursively
fn copy_folder_recursive(src: &Path, dest: &Path) -> Result<(), String> {
    if !src.exists() {
        return Err(format!("Source folder does not exist: {}", src.display()));
    }

    fs::create_dir_all(dest)
        .map_err(|e| format!("Failed to create destination: {}", e))?;

    for entry in fs::read_dir(src).map_err(|e| format!("Failed to read source: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if src_path.is_dir() {
            copy_folder_recursive(&src_path, &dest_path)?;
        } else {
            fs::copy(&src_path, &dest_path)
                .map_err(|e| format!("Failed to copy file: {}", e))?;
        }
    }

    Ok(())
}

/// Copy only tool.toml manifest files from pe_tools, preserving folder structure.
/// This gives MasterBooter its config when running inside PE without duplicating
/// all the tool binaries (which are already in Tools/<ToolName>/).
fn copy_toml_manifests_only(src: &Path, dest: &Path) -> Result<(), String> {
    if !src.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(src).map_err(|e| format!("Failed to read: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if src_path.is_dir() {
            // Recurse into subdirectories
            copy_toml_manifests_only(&src_path, &dest_path)?;
        } else if src_path.extension().and_then(|e| e.to_str()) == Some("toml") {
            // Only copy .toml files
            fs::create_dir_all(dest)
                .map_err(|e| format!("Failed to create dir: {}", e))?;
            fs::copy(&src_path, &dest_path)
                .map_err(|e| format!("Failed to copy toml: {}", e))?;
        }
    }

    Ok(())
}

/// Full WIM customization process
///
/// This is the main function that orchestrates the entire WIM customization:
/// 1. Mount the WIM
/// 2. Download any missing tools
/// 3. Inject tools
/// 4. Configure shell
/// 5. Create shortcuts
/// 6. Unmount and commit
///
/// # Arguments
/// * `wim_path` - Path to boot.wim
/// * `progress` - Progress callback (percent, message)
///
/// # Returns
/// Ok(()) on success, Err on failure
pub fn customize_wim(
    wim_path: &Path,
    progress: impl Fn(i32, &str),
) -> Result<(), String> {
    println!("\n========================================");
    println!("WIM Customization Starting");
    println!("========================================\n");

    // Create mount directory
    let mount_dir = std::env::temp_dir().join("MasterBooter_WIM_Mount");

    // Cleanup any previous mounts
    if is_wim_mounted(&mount_dir) {
        progress(0, "Cleaning up previous mount...");
        let _ = unmount_wim(&mount_dir, false);
    }
    if mount_dir.exists() {
        let _ = fs::remove_dir_all(&mount_dir);
    }

    // Step 1: Mount WIM with RAII guard (auto-unmounts on error)
    progress(5, "Mounting WIM image...");
    let mut guard = WimMountGuard::new(&mount_dir);
    mount_wim(wim_path, &mount_dir, 1)?;
    guard.mark_mounted(); // Now the guard will auto-unmount if we return early

    // Step 2: Discover and download PE tools
    progress(15, "Loading PE tools...");
    let mut tools = pe_tools::discover_pe_tools();

    // Count how many enabled tools we're expecting to inject
    let enabled_tool_count = tools.iter().filter(|t| t.enabled).count();

    // Check which enabled tools need downloading
    let tools_to_download: Vec<&pe_tools::PeTool> = tools.iter()
        .filter(|t| t.enabled && !t.is_present && !t.download_url.is_empty())
        .collect();
    let download_count = tools_to_download.len();

    if !tools_to_download.is_empty() {
        progress(20, &format!("Downloading {} of {} enabled tools...", download_count, enabled_tool_count));
        println!("Downloading {} missing tools...", download_count);

        let results = pe_tools::download_enabled_pe_tools(&tools, |name, current, total, _pct| {
            let msg = format!("Downloading {}/{}: {}", current, total, name);
            progress(20 + (current as i32 * 30 / total as i32), &msg);
        });

        // Update tool presence status and track failures
        let mut download_failures: Vec<String> = Vec::new();
        for result in &results {
            if result.success {
                if let Some(tool) = tools.iter_mut().find(|t| t.name == result.tool_name) {
                    tool.is_present = true;
                }
            } else {
                let err_msg = result.error_message.as_deref().unwrap_or("unknown error");
                download_failures.push(format!("{}: {}", result.tool_name, err_msg));
            }
        }

        // Surface download failures through the progress callback (visible in UI)
        if !download_failures.is_empty() {
            let warning = format!(
                "Warning: {} tool(s) failed to download - {}",
                download_failures.len(),
                download_failures.join(", ")
            );
            progress(50, &warning);
            println!("{}", warning);
        }
    }

    // Re-discover tools to update is_present flags
    tools = pe_tools::discover_pe_tools();

    // Step 3: Inject tools
    progress(55, &format!("Injecting tools into WIM ({} enabled)...", enabled_tool_count));
    let injected = inject_pe_tools(&mount_dir, &tools, |name, current, total| {
        let msg = format!("Injecting {}/{}: {}", current, total, name);
        progress(55 + (current as i32 * 15 / total.max(1) as i32), &msg);
    })?;

    // Report injection results through progress callback
    let skipped = enabled_tool_count.saturating_sub(injected.len());
    if skipped > 0 && enabled_tool_count > 0 {
        let warning = format!(
            "Injected {} of {} enabled tools ({} skipped - not downloaded)",
            injected.len(), enabled_tool_count, skipped
        );
        progress(70, &warning);
        println!("{}", warning);
    }

    // Step 4: Configure shell (only if tools were injected)
    let shell_name = if !injected.is_empty() {
        progress(75, "Configuring PE shell...");
        match configure_pe_shell(&mount_dir, &tools) {
            Ok(name) => name,
            Err(e) => {
                println!("Warning: Failed to configure shell: {}", e);
                "cmd.exe".to_string()
            }
        }
    } else if enabled_tool_count > 0 {
        // User enabled tools but none were injected — warn clearly
        progress(75, "Warning: No tools were injected! PE will boot to cmd.exe");
        println!("WARNING: {} tools were enabled but none could be injected", enabled_tool_count);
        "cmd.exe".to_string()
    } else {
        // No tools enabled - PE will boot to cmd.exe (default behavior)
        progress(75, "No tools enabled - PE will boot to cmd.exe");
        println!("No PE tools enabled - WinPE will boot to cmd.exe");
        "cmd.exe".to_string()
    };
    println!("Shell configured: {}", shell_name);

    // Step 5: Create shortcuts
    progress(80, "Creating shortcuts...");
    let shortcut_count = create_pe_shortcuts(&mount_dir, &tools)?;
    println!("Created {} shortcuts", shortcut_count);

    // Step 6: Unmount and commit using the guard (saves changes)
    progress(85, "Saving changes to WIM...");
    guard.commit_and_disarm()?;

    // Cleanup mount directory
    let _ = fs::remove_dir_all(&mount_dir);

    // Step 7: Export only Index 1 to create a single-image WIM
    // This is critical! Windows ISO boot.wim has 2 images:
    //   Index 1: Windows PE (our customized one)
    //   Index 2: Windows Setup (asks for drivers to install Windows)
    // We need to export ONLY Index 1, otherwise the ISO boots to Windows Setup
    progress(90, "Exporting customized PE image...");
    export_single_image(wim_path, 1)?;

    progress(100, "WIM customization complete!");

    println!("\n========================================");
    println!("WIM Customization Complete!");
    println!("  Shell: {}", shell_name);
    println!("  Tools injected: {}", injected.len());
    println!("  Shortcuts created: {}", shortcut_count);
    println!("========================================\n");

    Ok(())
}

// ============================================
// ENHANCED WIM CUSTOMIZATION WITH CONFIG
// ============================================
// This is the new enhanced version that uses PeBuildConfig
// and includes ADK package installation and PE fixes.

/// Full WIM customization with enhanced configuration
///
/// This is the enhanced version that supports:
/// - ADK package installation (PowerShell, WMI, Network, etc.)
/// - PE fixes (DPI scaling, WallpaperHost, profile folders, etc.)
/// - Driver injection
/// - Tool injection
/// - Configurable options
///
/// # Arguments
/// * `wim_path` - Path to boot.wim
/// * `config` - Build configuration with all options
/// * `progress` - Progress callback (percent, message)
///
/// # Returns
/// Ok(()) on success, Err on failure
pub fn customize_wim_with_config(
    wim_path: &Path,
    config: &PeBuildConfig,
    progress: impl Fn(i32, &str),
) -> Result<(), String> {
    println!("\n========================================");
    println!("Enhanced WIM Customization Starting");
    println!("========================================");
    println!("  Architecture: {}", config.architecture);
    println!("  Install packages: {} ({} selected)", config.install_packages, config.enabled_packages.len());
    println!("  Apply fixes: {} ({} selected)", config.apply_fixes, config.enabled_fixes.len());
    println!("========================================\n");

    // Create mount directory
    let mount_dir = std::env::temp_dir().join("MasterBooter_WIM_Mount");

    // Cleanup any previous mounts
    if is_wim_mounted(&mount_dir) {
        progress(0, "Cleaning up previous mount...");
        let _ = unmount_wim(&mount_dir, false);
    }
    if mount_dir.exists() {
        let _ = fs::remove_dir_all(&mount_dir);
    }

    // ============================================
    // STEP 1: Mount WIM with RAII guard (auto-unmounts on error)
    // ============================================
    progress(2, "Mounting WIM image...");
    let mut guard = WimMountGuard::new(&mount_dir);
    mount_wim(wim_path, &mount_dir, 1)?;
    guard.mark_mounted(); // Now the guard will auto-unmount if we return early

    // ============================================
    // STEP 2: Install ADK Packages (if enabled)
    // ============================================
    let mut packages_installed = 0;
    if config.install_packages && !config.enabled_packages.is_empty() {
        progress(5, "Detecting ADK packages location...");

        // Find ADK packages
        if let Some(adk_location) = adk_packages::detect_adk_packages_path(&config.architecture) {
            progress(8, &format!("Installing {} ADK packages...", config.enabled_packages.len()));
            println!("\nInstalling ADK packages from: {}", adk_location.winpe_ocs_path.display());

            let results = adk_packages::install_packages(
                &mount_dir,
                &adk_location,
                &config.enabled_packages,
                |name, current, total| {
                    let pct = 8 + (current as i32 * 20 / total.max(1) as i32);
                    progress(pct, &format!("Installing package {}/{}: {}", current, total, name));
                },
            );

            packages_installed = results.iter().filter(|r| r.success).count();

            // Log any failures
            let failed: Vec<_> = results.iter().filter(|r| !r.success).collect();
            if !failed.is_empty() {
                println!("\nWarning: {} packages failed to install:", failed.len());
                for f in &failed {
                    println!("  - {}: {}", f.package_name, f.message);
                }
            }

            println!("Installed {} of {} packages", packages_installed, config.enabled_packages.len());
        } else {
            println!("Warning: ADK packages not found - skipping package installation");
            println!("Install Windows ADK with WinPE add-on to enable packages");
        }
    }

    // ============================================
    // STEP 3: Apply PE Fixes (if enabled)
    // ============================================
    let mut fixes_applied = 0;
    if config.apply_fixes && !config.enabled_fixes.is_empty() {
        progress(30, &format!("Applying {} PE fixes...", config.enabled_fixes.len()));
        println!("\nApplying PE fixes...");

        let results = pe_fixes::apply_fixes(
            &mount_dir,
            &config.enabled_fixes,
            &config.fix_options,
            |name, current, total| {
                let pct = 30 + (current as i32 * 10 / total.max(1) as i32);
                progress(pct, &format!("Applying fix {}/{}: {}", current, total, name));
            },
        );

        fixes_applied = results.iter().filter(|r| r.success).count();

        // Log any failures
        let failed: Vec<_> = results.iter().filter(|r| !r.success).collect();
        if !failed.is_empty() {
            println!("\nWarning: {} fixes failed to apply:", failed.len());
            for f in &failed {
                println!("  - {}: {}", f.fix_name, f.message);
            }
        }

        println!("Applied {} of {} fixes", fixes_applied, config.enabled_fixes.len());
    }

    // ============================================
    // STEP 4: Inject Drivers (if enabled)
    // ============================================
    // Drivers come from these sources (in order):
    // 1. WiFi drivers extracted from ISO's install.wim (if source is ISO + WiFi enabled)
    // 2. User-provided driver folders in config.driver_paths
    // 3. User-provided Drivers/ folder next to the EXE
    // Also copies drivers into the PE filesystem for drvload fallback at boot.
    let mut drivers_injected = 0;       // Count of drivers DISM successfully installed
    let mut drivers_copied_for_drvload: usize = 0;  // Count of driver files copied for drvload fallback
    let wifi_extract_dir = std::env::temp_dir().join("MasterBooter_WiFi_Extract");

    // Determine source type for WiFi decision-making
    let source_lower = config.source_path.to_string_lossy().to_lowercase();
    let source_is_winre = source_lower.contains("winre") || source_lower.contains("recovery");
    let source_is_iso = source_lower.ends_with(".iso");

    if config.include_drivers {
        // Collect all driver paths (config-provided + auto-detected)
        let mut all_driver_paths: Vec<PathBuf> = config.driver_paths.clone();

        // ============================================
        // WiFi driver extraction — from ISO's install.wim
        // ============================================
        // The PE could be deployed to a DIFFERENT machine than the build machine,
        // so WiFi drivers must come from the Windows ISO source media (install.wim),
        // NOT from the local machine's C:\Windows.
        //
        // Source scenarios:
        //   ISO_PE  → Extract WiFi from install.wim in ISO (new approach)
        //   ISO_RE  → WinRE has WiFi built in → skip
        //   LocalRE → WinRE has WiFi built in → skip
        //   copype  → No install.wim available → skip, user can place drivers in Drivers/ folder
        if config.enable_wifi {
            if source_is_winre {
                println!("Source is WinRE - WiFi drivers already built in, skipping extraction");
            } else if source_is_iso {
                progress(42, "Extracting WiFi + touchpad drivers from ISO...");
                match extract_wifi_files_from_source(&config.source_path) {
                    Ok(wifi_dir) => {
                        // The extracted folder contains complete DriverStore packages
                        // that DISM can properly install (unlike loose INF+sys pairs).
                        // Includes WiFi adapter drivers AND touchpad/I2C HID drivers.
                        let driver_store = wifi_dir.join("1").join("Windows")
                            .join("System32").join("DriverStore").join("FileRepository");
                        if driver_store.exists() {
                            println!("  Found WiFi + touchpad driver packages in DriverStore");
                            all_driver_paths.push(driver_store);
                        }

                        // Note: We also extract hidi2c.sys and hidi2c.inf as loose files
                        // but DISM can't inject loose INFs that reference files in other dirs.
                        // The DriverStore packages (hidi2c.inf_*, ialpss2_i2c*, etc.) are
                        // self-contained and DISM handles them properly via the DriverStore path above.
                    }
                    Err(e) => {
                        println!("Warning: WiFi/touchpad extraction from ISO failed: {}", e);
                        println!("WiFi adapters and touchpads may not work in the PE.");
                        println!("Tip: Place drivers in a Drivers/ folder next to the EXE.");
                    }
                }
            } else {
                // copype/ADK mode — no install.wim available
                println!("Source is not an ISO — no install.wim available for WiFi driver extraction");
                println!("Tip: Place WiFi drivers in a Drivers/ folder next to the EXE.");
            }
        }

        // Also check for user-provided Drivers folder next to the EXE
        let app_dir = crate::tools::get_app_directory();
        let user_drivers = app_dir.join("Drivers");
        if user_drivers.exists() && !all_driver_paths.contains(&user_drivers) {
            println!("Found user drivers folder: {}", user_drivers.display());
            all_driver_paths.push(user_drivers);
        }

        if !all_driver_paths.is_empty() {
            progress(45, &format!("Injecting drivers from {} source(s)...", all_driver_paths.len()));
            println!("\nInjecting drivers into WIM...");

            for driver_path in &all_driver_paths {
                if driver_path.exists() {
                    match inject_drivers(&mount_dir, driver_path) {
                        Ok(count) => {
                            drivers_injected += count;
                            println!("  Injected {} drivers from {}", count, driver_path.display());
                        }
                        Err(e) => {
                            println!("  Warning: Failed to inject from {}: {}", driver_path.display(), e);
                        }
                    }
                } else {
                    println!("  Warning: Driver path not found: {}", driver_path.display());
                }
            }

            // Also copy extracted drivers into the PE filesystem for drvload fallback
            // This allows loading drivers at boot time if DISM injection missed any
            let pe_drivers_dir = mount_dir.join("Drivers");
            let _ = fs::create_dir_all(&pe_drivers_dir);
            for driver_path in &all_driver_paths {
                if driver_path.exists() {
                    // Copy .inf, .sys, and .cat files recursively
                    match copy_drivers_to_pe(&pe_drivers_dir, driver_path) {
                        Ok(count) => {
                            drivers_copied_for_drvload += count;
                            println!("  Copied {} driver files to PE for drvload fallback", count);
                        }
                        Err(e) => {
                            println!("  Warning: Could not copy drivers to PE filesystem: {}", e);
                        }
                    }
                }
            }
        } else {
            println!("\nNo drivers found to inject (no driver_paths configured, WiFi extraction may have been skipped)");
        }
    }

    // ============================================
    // STEP 4.5: Inject WiFi/WLAN Support (if enabled)
    // ============================================
    // WinPE does NOT include WiFi support by default. WinPE-WiFi-Package only exists
    // inside WinRE.wim and is NOT available as a standalone ADK optional component.
    // We must manually copy the WLAN service infrastructure (DLLs, drivers, registry)
    // from the ISO's install.wim into the mounted PE image.
    if config.enable_wifi {
        progress(48, "Injecting WiFi/WLAN support...");
        println!("\nWiFi support enabled - injecting WLAN service infrastructure...");

        if source_is_winre {
            println!("Source is WinRE - WiFi already built in, skipping injection");
        } else if source_is_iso {
            // Use the already-extracted WiFi files from the ISO's install.wim
            let source_windows = wifi_extract_dir.join("1").join("Windows");
            if source_windows.exists() {
                match inject_wifi_support(&mount_dir, &source_windows) {
                    Ok(()) => {
                        println!("WiFi/WLAN support injected successfully (from ISO source)");
                    }
                    Err(e) => {
                        println!("Warning: WiFi injection failed: {}", e);
                        println!("WiFi may not work in the PE. Consider using WinRE as the source instead.");
                    }
                }
            } else {
                println!("Warning: WiFi source files not available (extraction may have failed)");
                println!("WiFi may not work in the PE.");
            }
        } else {
            // copype/ADK mode — no install.wim, can't inject WiFi infrastructure
            println!("Source is not an ISO — cannot inject WiFi service infrastructure");
            println!("Tip: Use WinRE as source (WiFi built in) or use an ISO source.");
        }
    }

    // Cleanup WiFi extraction temp folder
    if wifi_extract_dir.exists() {
        let _ = fs::remove_dir_all(&wifi_extract_dir);
    }

    // ============================================
    // STEP 4.6: Inject Branding Wallpaper (if found)
    // ============================================
    // Look for a branding wallpaper and copy it into the PE so WinXShell displays it.
    // The registry keys are already set by apply_wallpaper_host_fix() in pe_fixes.rs.
    progress(49, "Checking for branding wallpaper...");
    match inject_branding(&mount_dir) {
        Ok(true) => println!("Branding wallpaper injected into PE"),
        Ok(false) => println!("No branding wallpaper found (skipped)"),
        Err(e) => println!("Warning: Branding wallpaper injection failed: {}", e),
    }

    // ============================================
    // STEP 5: Inject PE Tools (if enabled)
    // ============================================
    let mut tools_injected = Vec::new();
    if config.include_tools {
        progress(50, "Loading PE tools...");
        let mut tools = pe_tools::discover_pe_tools();

        // Count how many enabled tools we're expecting to inject
        let enabled_tool_count = tools.iter().filter(|t| t.enabled).count();

        // Check which enabled tools need downloading
        let tools_to_download: Vec<&pe_tools::PeTool> = tools.iter()
            .filter(|t| t.enabled && !t.is_present && !t.download_url.is_empty())
            .collect();
        let download_count = tools_to_download.len();

        if !tools_to_download.is_empty() {
            progress(52, &format!("Downloading {} of {} enabled tools...", download_count, enabled_tool_count));
            println!("\nDownloading {} missing tools...", download_count);

            let results = pe_tools::download_enabled_pe_tools(&tools, |name, current, total, _pct| {
                let msg = format!("Downloading {}/{}: {}", current, total, name);
                progress(52 + (current as i32 * 8 / total.max(1) as i32), &msg);
            });

            // Update tool presence status and track failures
            let mut download_failures: Vec<String> = Vec::new();
            for result in &results {
                if result.success {
                    if let Some(tool) = tools.iter_mut().find(|t| t.name == result.tool_name) {
                        tool.is_present = true;
                    }
                } else {
                    // Record the failure so we can warn the user
                    let err_msg = result.error_message.as_deref().unwrap_or("unknown error");
                    download_failures.push(format!("{}: {}", result.tool_name, err_msg));
                }
            }

            // Surface download failures through the progress callback (visible in UI)
            if !download_failures.is_empty() {
                let warning = format!(
                    "Warning: {} tool(s) failed to download - {}",
                    download_failures.len(),
                    download_failures.join(", ")
                );
                progress(60, &warning);
                println!("{}", warning);
            }
        }

        // Re-discover tools to update is_present flags
        tools = pe_tools::discover_pe_tools();

        // Inject tools
        progress(62, &format!("Injecting tools into WIM ({} enabled)...", enabled_tool_count));
        match inject_pe_tools(&mount_dir, &tools, |name, current, total| {
            let msg = format!("Injecting {}/{}: {}", current, total, name);
            progress(62 + (current as i32 * 8 / total.max(1) as i32), &msg);
        }) {
            Ok(injected) => {
                tools_injected = injected.clone();
                // Report how many tools were actually injected vs enabled
                let injected_count = injected.len();
                let skipped = enabled_tool_count.saturating_sub(injected_count);
                if skipped > 0 {
                    // Some enabled tools weren't injected (missing/failed download)
                    let warning = format!(
                        "Injected {} of {} enabled tools ({} skipped - not downloaded)",
                        injected_count, enabled_tool_count, skipped
                    );
                    progress(70, &warning);
                    println!("{}", warning);
                } else {
                    progress(70, &format!("All {} enabled tools injected successfully", injected_count));
                }
            }
            Err(e) => {
                println!("Warning: Tool injection failed: {}", e);
                progress(70, &format!("Warning: Tool injection failed: {}", e));
            }
        }

        // Configure shell
        if !tools_injected.is_empty() {
            progress(72, "Configuring PE shell...");
            match configure_pe_shell(&mount_dir, &tools) {
                Ok(shell_name) => {
                    println!("Shell configured: {}", shell_name);
                }
                Err(e) => {
                    println!("Warning: Failed to configure shell: {}", e);
                }
            }

            // Create shortcuts
            progress(75, "Creating shortcuts...");
            match create_pe_shortcuts(&mount_dir, &tools) {
                Ok(count) => {
                    println!("Created {} shortcuts", count);
                }
                Err(e) => {
                    println!("Warning: Failed to create shortcuts: {}", e);
                }
            }
        } else if enabled_tool_count > 0 {
            // User enabled tools but none were injected — warn them clearly
            progress(72, "Warning: No tools were injected! PE will boot to cmd.exe");
            println!("WARNING: {} tools were enabled but none could be injected", enabled_tool_count);
        }
    }

    // ============================================
    // STEP 6: Unmount and Commit using the guard (saves changes)
    // ============================================
    progress(80, "Saving changes to WIM...");
    guard.commit_and_disarm()?;

    // Cleanup mount directory (WiFi temp folder already cleaned up after step 4.5)
    let _ = fs::remove_dir_all(&mount_dir);

    // ============================================
    // STEP 7: Export Single Image
    // ============================================
    // This is critical! Windows ISO boot.wim has 2 images:
    //   Index 1: Windows PE (our customized one)
    //   Index 2: Windows Setup (asks for drivers to install Windows)
    // We need to export ONLY Index 1, otherwise the ISO boots to Windows Setup
    progress(90, "Exporting customized PE image...");
    export_single_image(wim_path, 1)?;

    progress(100, "WIM customization complete!");

    // ============================================
    // SUMMARY
    // ============================================
    println!("\n========================================");
    println!("Enhanced WIM Customization Complete!");
    println!("========================================");
    println!("  ADK Packages installed: {}", packages_installed);
    println!("  PE Fixes applied: {}", fixes_applied);
    // Report driver counts: DISM injection + drvload fallback copies
    if drivers_injected > 0 {
        println!("  Drivers injected via DISM: {}", drivers_injected);
    }
    if drivers_copied_for_drvload > 0 {
        if drivers_injected == 0 {
            // DISM didn't install any, but drvload has them — explain to the user
            println!("  Drivers: {} files available via drvload at boot", drivers_copied_for_drvload);
            println!("    (DISM injection returned 0 — drivers will load at PE boot instead)");
        } else {
            println!("  Drivers copied for drvload fallback: {}", drivers_copied_for_drvload);
        }
    }
    if drivers_injected == 0 && drivers_copied_for_drvload == 0 && config.include_drivers {
        println!("  Drivers: none found to inject");
    }
    println!("  Tools injected: {}", tools_injected.len());
    println!("========================================\n");

    Ok(())
}

/// Inject drivers from a folder into a mounted WIM
///
/// Uses DISM to add all drivers from the specified path.
/// Supports recursive search for .inf files.
///
/// # Arguments
/// * `mount_path` - Path where WIM is mounted
/// * `driver_path` - Path to folder containing drivers
///
/// # Returns
/// Ok(count) with number of drivers injected, Err on failure
pub fn inject_drivers(mount_path: &Path, driver_path: &Path) -> Result<usize, String> {
    println!("Injecting drivers from: {}", driver_path.display());

    // Use DISM to add all drivers from the path recursively
    let output = Command::new("dism")
        .arg(format!("/Image:{}", mount_path.display()))
        .arg("/Add-Driver")
        .arg(format!("/Driver:{}", driver_path.display()))
        .arg("/Recurse")
        .arg("/ForceUnsigned")
        .output()
        .map_err(|e| format!("Failed to run DISM: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        // Check if it's just "no drivers found" which is not an error
        if stdout.contains("0 driver package") || stdout.contains("No driver packages") {
            return Ok(0);
        }
        return Err(format!("DISM failed: {}\n{}", stdout, stderr));
    }

    // Parse output to get count of installed drivers
    // Look for "Total driver packages installed: X"
    let count = if let Some(line) = stdout.lines().find(|l| l.contains("driver package")) {
        line.split_whitespace()
            .find_map(|word| word.parse::<usize>().ok())
            .unwrap_or(0)
    } else {
        0
    };

    Ok(count)
}

/// Copy driver files (.inf, .sys, .cat) from a source folder into the PE filesystem
///
/// This places driver files inside the mounted WIM so that the PE launcher script
/// can load them via `drvload` at boot time as a fallback if DISM injection missed any.
///
/// # Arguments
/// * `pe_drivers_dir` - Destination folder inside the mounted WIM (e.g., mount/Drivers)
/// * `source_path` - Source folder containing driver files (recursively searched)
fn copy_drivers_to_pe(pe_drivers_dir: &Path, source_path: &Path) -> Result<usize, String> {
    // Walk the source directory recursively and copy driver-related files
    fn copy_recursive(src: &Path, dest: &Path) -> Result<u32, String> {
        let mut count = 0;
        let entries = fs::read_dir(src)
            .map_err(|e| format!("Cannot read {}: {}", src.display(), e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Dir entry error: {}", e))?;
            let path = entry.path();

            if path.is_dir() {
                // Recurse into subdirectory
                let sub_dest = dest.join(entry.file_name());
                let _ = fs::create_dir_all(&sub_dest);
                count += copy_recursive(&path, &sub_dest)?;
            } else if let Some(ext) = path.extension() {
                // Copy driver-related file types
                let ext_lower = ext.to_string_lossy().to_lowercase();
                if matches!(ext_lower.as_str(), "inf" | "sys" | "cat" | "dll") {
                    let dest_file = dest.join(entry.file_name());
                    let _ = fs::create_dir_all(dest);
                    if fs::copy(&path, &dest_file).is_ok() {
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    let count = copy_recursive(source_path, pe_drivers_dir)?;
    println!("  Copied {} driver files to PE filesystem", count);
    Ok(count as usize)
}

// ============================================
// BRANDING / WALLPAPER INJECTION
// ============================================
// Copies the user's branding wallpaper into the PE image so WinXShell
// can display it as the desktop background. The registry keys pointing
// to this wallpaper are set by apply_wallpaper_host_fix() in pe_fixes.rs.

/// The branding wallpaper is embedded directly into the EXE at compile time.
/// This means the wallpaper is always available — no external files needed.
static EMBEDDED_WALLPAPER: &[u8] = include_bytes!("../assets/wallpaper.jpg");

/// Inject branding wallpaper into the mounted WIM
///
/// Writes the embedded wallpaper.jpg (compiled into the EXE) to the WIM at
/// `Windows\Web\Wallpaper\Windows\wallpaper.jpg` — the standard location
/// that WinXShell reads for the desktop background.
///
/// # Returns
/// Ok(()) on success, Err on failure
fn inject_branding(mount_dir: &Path) -> Result<bool, String> {
    println!("\n--- Injecting Branding Wallpaper ---");

    // Create the destination directory inside the mounted WIM
    // Standard Windows wallpaper location that WinXShell reads
    let dest_dir = mount_dir
        .join("Windows")
        .join("Web")
        .join("Wallpaper")
        .join("Windows");
    fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create wallpaper directory: {}", e))?;

    // Write the embedded wallpaper bytes to the WIM
    let dest_file = dest_dir.join("wallpaper.jpg");
    fs::write(&dest_file, EMBEDDED_WALLPAPER)
        .map_err(|e| format!("Failed to write wallpaper: {}", e))?;

    println!("  Wallpaper written: {} ({} bytes)", dest_file.display(), EMBEDDED_WALLPAPER.len());
    println!("  WinXShell will display this wallpaper on boot (registry keys set by wallpaper_host fix)");
    println!("--- Branding wallpaper injection complete ---\n");

    Ok(true)
}

// ============================================
// WIFI SUPPORT INJECTION
// ============================================
// WinPE does NOT include WiFi (WLAN) support by default.
// Microsoft's WinPE-WiFi-Package only exists inside WinRE.wim and is NOT available
// as a standalone ADK optional component (.cab file).
//
// To enable WiFi in WinPE, we extract the WLAN service infrastructure from
// the ISO's install.wim (NOT the local machine). This is critical because the PE
// could be deployed to a completely different machine than the build machine.
//
// What we extract from install.wim:
// - Core WLAN DLLs (wlansvc.dll, wlanapi.dll, etc.)
// - NativeWiFi driver files (nwifi.sys, vwififlt.sys, etc.)
// - WiFi adapter drivers (DriverStore\FileRepository — complete packages)
// - L2Schemas (XML schema files for WLAN profiles)
// - en-US MUI files for wlanext and wlancfg
//
// Registry entries are hardcoded (service config doesn't vary between Windows versions).
//
// At boot time, the launcher script runs "net start wlansvc" to activate WiFi.
// PENetwork then uses the WLAN API to enumerate and connect to WiFi networks.
//
// Reference: PhoenixPE's NetworkDrivers.script uses the same approach —
// extracting WiFi components from the source media, not the build machine.

/// Extract all WiFi-related files from the ISO's install.wim
///
/// Mounts the ISO, uses 7-Zip to extract only WiFi-specific files from
/// install.wim image index 1, then dismounts the ISO. This avoids extracting
/// the full 4+ GB install.wim — only WiFi files are pulled.
///
/// # Arguments
/// * `iso_path` - Path to the Windows ISO file
///
/// # Returns
/// Ok(PathBuf) to temp folder containing extracted `1\Windows\...` structure,
/// or Err if extraction failed.
pub fn extract_wifi_files_from_source(iso_path: &Path) -> Result<PathBuf, String> {
    println!("\n--- Extracting WiFi + Touchpad Drivers from ISO Source Media ---");
    println!("  ISO: {}", iso_path.display());

    // We need 7-Zip to extract specific files from the WIM inside the ISO
    let seven_zip = find_7zip().ok_or(
        "7-Zip not found. Install 7-Zip to enable WiFi extraction from ISO.\n\
         Download from: https://www.7-zip.org/".to_string()
    )?;
    println!("  Using 7-Zip: {}", seven_zip.display());

    // ============================================
    // STEP 1: Mount the ISO to get a drive letter
    // ============================================
    println!("  Mounting ISO...");
    let mount_output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!(
            "$img = Mount-DiskImage -ImagePath '{}' -PassThru; \
             ($img | Get-Volume).DriveLetter",
            iso_path.display()
        )])
        .output()
        .map_err(|e| format!("Failed to run PowerShell to mount ISO: {}", e))?;

    if !mount_output.status.success() {
        let stderr = String::from_utf8_lossy(&mount_output.stderr);
        return Err(format!("Failed to mount ISO: {}", stderr.trim()));
    }

    let drive_letter = String::from_utf8_lossy(&mount_output.stdout)
        .trim()
        .to_string();

    if drive_letter.is_empty() || drive_letter.len() > 2 {
        return Err(format!("Got unexpected drive letter from ISO mount: '{}'", drive_letter));
    }

    println!("  ISO mounted at {}:\\", drive_letter);

    // ============================================
    // STEP 2: Find install.wim (or install.esd) on the mounted ISO
    // ============================================
    let iso_sources = format!("{}:\\sources", drive_letter);
    let install_wim = PathBuf::from(&iso_sources).join("install.wim");
    let install_esd = PathBuf::from(&iso_sources).join("install.esd");

    let wim_path = if install_wim.exists() {
        println!("  Found: {}", install_wim.display());
        install_wim
    } else if install_esd.exists() {
        println!("  Found: {}", install_esd.display());
        install_esd
    } else {
        // Dismount before returning error
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!(
                "Dismount-DiskImage -ImagePath '{}'", iso_path.display()
            )])
            .output();
        return Err(format!(
            "No install.wim or install.esd found at {}:\\sources\\\n\
             This ISO may not contain a full Windows installation.",
            drive_letter
        ));
    };

    // ============================================
    // STEP 3: Create temp folder for extracted files
    // ============================================
    let extract_dir = std::env::temp_dir().join("MasterBooter_WiFi_Extract");
    if extract_dir.exists() {
        let _ = fs::remove_dir_all(&extract_dir);
    }
    fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create temp extraction folder: {}", e))?;

    // ============================================
    // STEP 4: Extract WiFi + Touchpad drivers using 7-Zip
    // ============================================
    // The "1\" prefix selects WIM image index 1 (the first Windows edition).
    // We extract WiFi files AND touchpad/I2C drivers — NOT the full 4+ GB image.
    // Touchpad drivers are included because WinPE only has basic USB HID;
    // most modern laptops use I2C-connected touchpads that won't work without drivers.
    println!("  Extracting WiFi + touchpad drivers from install.wim (this may take a moment)...");

    let output = Command::new(&seven_zip)
        .arg("x")
        .arg(wim_path.to_string_lossy().as_ref())
        .arg(format!("-o{}", extract_dir.display()))
        // --- Core WLAN DLLs and executables ---
        .arg(r"1\Windows\System32\wlansvc.dll")
        .arg(r"1\Windows\System32\wlanapi.dll")
        .arg(r"1\Windows\System32\wlancfg.dll")
        .arg(r"1\Windows\System32\wlanhlp.dll")
        .arg(r"1\Windows\System32\wlanmsm.dll")
        .arg(r"1\Windows\System32\wlansec.dll")
        .arg(r"1\Windows\System32\wlanui.dll")
        .arg(r"1\Windows\System32\wlgpclnt.dll")
        .arg(r"1\Windows\System32\wlanext.exe")
        .arg(r"1\Windows\System32\wifitask.exe")
        // --- Dependency DLLs (required for Windows 10 1607+) ---
        .arg(r"1\Windows\System32\dmcmnutils.dll")
        .arg(r"1\Windows\System32\mdmregistration.dll")
        .arg(r"1\Windows\System32\mdmpostprocessevaluator.dll")
        // --- NativeWiFi kernel drivers ---
        .arg(r"1\Windows\System32\Drivers\nwifi.sys")
        .arg(r"1\Windows\System32\Drivers\vwififlt.sys")
        .arg(r"1\Windows\System32\Drivers\vwifibus.sys")
        .arg(r"1\Windows\System32\Drivers\WdiWiFi.sys")
        // --- WiFi INF files (NativeWiFi protocol/filter drivers) ---
        .arg(r"1\Windows\INF\netnwifi.inf")
        .arg(r"1\Windows\INF\netvwififlt.inf")
        .arg(r"1\Windows\INF\netvwifibus.inf")
        // --- L2Schemas and AvailableNetwork schemas ---
        .arg(r"1\Windows\L2Schemas\*")
        .arg(r"1\Windows\schemas\AvailableNetwork\*")
        // --- en-US MUI files ---
        .arg(r"1\Windows\System32\en-US\wlanext.exe.mui")
        .arg(r"1\Windows\System32\en-US\wlancfg.dll.mui")
        // --- WiFi adapter drivers from DriverStore (complete packages!) ---
        // Intel WiFi (covers WiFi 5/6/6E/7)
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netwtw*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netwbw*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netwew*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netwlv*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netwns*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netwsw*")
        // Realtek WiFi
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netrtwlane*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\net81*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\net819*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netrtwlanu*")
        // Broadcom WiFi
        .arg(r"1\Windows\System32\DriverStore\FileRepository\bcmwdi*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netbc6*")
        // Qualcomm/Atheros WiFi
        .arg(r"1\Windows\System32\DriverStore\FileRepository\athw*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netathr*")
        // Ralink/MediaTek WiFi
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netr28*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netr73*")
        // Marvell WiFi
        .arg(r"1\Windows\System32\DriverStore\FileRepository\mrvlpcie*")
        // --- Touchpad / I2C HID / Input device drivers ---
        // WinPE only includes basic USB HID. Modern laptops use I2C-connected
        // touchpads (ELAN, Synaptics, Alps) which need these drivers for
        // touchpad movement, clicking, and scroll gestures.
        //
        // Driver stack: I2C controller → hidi2c.sys → HID class → touchpad filter
        //
        // Microsoft I2C HID miniport (connects I2C bus to Windows HID stack)
        .arg(r"1\Windows\System32\DriverStore\FileRepository\hidi2c.inf*")
        // I2C HID system driver + INF (fallback if not in DriverStore)
        .arg(r"1\Windows\System32\Drivers\hidi2c.sys")
        .arg(r"1\Windows\INF\hidi2c.inf")
        // Intel SerialIO I2C controllers (most common on Intel laptops)
        .arg(r"1\Windows\System32\DriverStore\FileRepository\ialpss2_i2c*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\ialpss2_gpio*")
        // Intel SerialIO (older Intel platforms — Haswell through Skylake)
        .arg(r"1\Windows\System32\DriverStore\FileRepository\iaLPSS_I2C*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\iaLPSS_GPIO*")
        // Intel THC (Touch Host Controller — newer platforms like Alder Lake+)
        .arg(r"1\Windows\System32\DriverStore\FileRepository\intcthc*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\iathc*")
        // AMD I2C controller (AMD laptops)
        .arg(r"1\Windows\System32\DriverStore\FileRepository\amdi2c*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\amdgpio*")
        // Synaptics touchpad drivers
        .arg(r"1\Windows\System32\DriverStore\FileRepository\synpd*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\smbus*")
        // ELAN touchpad drivers
        .arg(r"1\Windows\System32\DriverStore\FileRepository\etd*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\elan*")
        // Alps touchpad drivers
        .arg(r"1\Windows\System32\DriverStore\FileRepository\alps*")
        // Goodix / FocalTech touchpad (used on some AMD/ARM laptops)
        .arg(r"1\Windows\System32\DriverStore\FileRepository\goodix*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\focal*")
        // Microsoft Precision Touchpad (class driver — provides gestures)
        .arg(r"1\Windows\System32\DriverStore\FileRepository\mshidkmdf*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\hidinterrupt*")
        // --- Registry hives (for copying service entries into PE) ---
        // PhoenixPE copies entire service subtrees from install.wim's registry
        // instead of manually creating individual values. This gets ALL subkeys,
        // parameters, security descriptors, and binding info automatically.
        .arg(r"1\Windows\System32\config\SYSTEM")
        .arg(r"1\Windows\System32\config\SOFTWARE")
        // --- Additional WLAN DLLs (PhoenixPE includes these for full WiFi stack) ---
        // Connection dialog and preferences
        .arg(r"1\Windows\System32\WLanConn.dll")
        .arg(r"1\Windows\System32\wlandlg.dll")
        .arg(r"1\Windows\System32\WLanHC.dll")
        .arg(r"1\Windows\System32\WlanMediaManager.dll")
        .arg(r"1\Windows\System32\WlanMM.dll")
        .arg(r"1\Windows\System32\wlanpref.dll")
        .arg(r"1\Windows\System32\wlansvcpal.dll")
        .arg(r"1\Windows\System32\wlanutil.dll")
        .arg(r"1\Windows\System32\WlanRadioManager.dll")
        .arg(r"1\Windows\System32\mobilenetworking.dll")
        // --- dot3 (802.1X) DLLs — needed for WiFi authentication ---
        .arg(r"1\Windows\System32\dot3api.dll")
        .arg(r"1\Windows\System32\dot3cfg.dll")
        .arg(r"1\Windows\System32\dot3dlg.dll")
        .arg(r"1\Windows\System32\dot3gpclnt.dll")
        .arg(r"1\Windows\System32\dot3gpui.dll")
        .arg(r"1\Windows\System32\dot3hc.dll")
        .arg(r"1\Windows\System32\dot3msm.dll")
        .arg(r"1\Windows\System32\dot3svc.dll")
        .arg(r"1\Windows\System32\dot3ui.dll")
        // --- L2/802.1X authentication DLLs ---
        .arg(r"1\Windows\System32\l2gpstore.dll")
        .arg(r"1\Windows\System32\l2nacp.dll")
        .arg(r"1\Windows\System32\onex.dll")
        .arg(r"1\Windows\System32\onexui.dll")
        // --- Windows Connection Manager (WCM) DLLs ---
        // wcmsvc is a dependency of WlanSvc — PhoenixPE installs it fully
        .arg(r"1\Windows\System32\wcmapi.dll")
        .arg(r"1\Windows\System32\wcmcsp.dll")
        .arg(r"1\Windows\System32\wcmsvc.dll")
        .arg(r"1\Windows\System32\NetworkUXBroker.dll")
        // --- EAP credential DLLs ---
        .arg(r"1\Windows\System32\cngcredui.dll")
        .arg(r"1\Windows\System32\cngprovider.dll")
        // --- Network helper DLLs ---
        .arg(r"1\Windows\System32\VAN.dll")
        .arg(r"1\Windows\System32\RMapi.dll")
        .arg(r"1\Windows\System32\netevent.dll")
        // --- Additional kernel drivers ---
        .arg(r"1\Windows\System32\Drivers\wfplwfs.sys")
        // --- Additional INF files ---
        .arg(r"1\Windows\INF\netlldp.inf")
        .arg(r"1\Windows\INF\ndiscap.inf")
        // --- Additional DriverStore packages for WiFi protocol/filter drivers ---
        // These contain the complete Ndi binding info that DISM needs
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netnwifi.inf*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netvwifibus.inf*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netvwififlt.inf*")
        .arg(r"1\Windows\System32\DriverStore\FileRepository\netvwifimp.inf*")
        // --- Suppress prompts, don't show progress bar ---
        .arg("-y")
        .output()
        .map_err(|e| format!("Failed to run 7-Zip: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // 7-Zip may return non-zero even on partial success (some files not found is OK)
    // We check for actual fatal errors vs "no files found" warnings
    if !output.status.success() {
        // Check if it's just "no files found" warnings (exit code 1 = warning)
        if let Some(code) = output.status.code() {
            if code == 1 {
                // Warning level — some files not found, which is expected
                // (not all Windows versions have all WiFi driver vendors)
                println!("  7-Zip warnings (some WiFi files not in this ISO — this is normal)");
            } else {
                // Fatal error
                // Dismount ISO before returning
                let _ = Command::new("powershell")
                    .args(["-NoProfile", "-Command", &format!(
                        "Dismount-DiskImage -ImagePath '{}'", iso_path.display()
                    )])
                    .output();
                return Err(format!("7-Zip extraction failed (exit code {}):\n{}\n{}", code, stdout, stderr));
            }
        }
    }

    // Count what was extracted
    let source_windows = extract_dir.join("1").join("Windows");
    if !source_windows.exists() {
        // Nothing was extracted — dismount and return error
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!(
                "Dismount-DiskImage -ImagePath '{}'", iso_path.display()
            )])
            .output();
        return Err("No WiFi files could be extracted from install.wim.\n\
                    The ISO may not contain inbox WiFi drivers.".to_string());
    }

    // Log what we found
    let sys32_check = source_windows.join("System32").join("wlansvc.dll");
    let driver_store = source_windows.join("System32").join("DriverStore").join("FileRepository");
    println!("  WLAN DLLs: {}", if sys32_check.exists() { "found" } else { "NOT found" });
    if driver_store.exists() {
        // Count WiFi driver folders
        if let Ok(entries) = fs::read_dir(&driver_store) {
            let count = entries.filter(|e| e.is_ok()).count();
            println!("  WiFi driver packages in DriverStore: {}", count);
        }
    }

    // ============================================
    // STEP 5: Dismount the ISO
    // ============================================
    println!("  Dismounting ISO...");
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!(
            "Dismount-DiskImage -ImagePath '{}'", iso_path.display()
        )])
        .output();

    println!("  WiFi files extracted to: {}", extract_dir.display());
    println!("--- WiFi extraction from ISO complete ---\n");

    Ok(extract_dir)
}

/// Extract WiFi adapter drivers from the local Windows installation (LEGACY).
///
/// NOTE: This function extracts from the BUILD machine's C:\Windows, which is
/// incorrect when the PE will be deployed to a different machine. The new
/// approach is extract_wifi_files_from_source() which uses the ISO's install.wim.
/// This function is kept as a fallback for non-ISO source types.
///
/// Returns the path to a temp folder containing the extracted drivers.
#[allow(dead_code)]  // Kept as legacy fallback — new code uses extract_wifi_files_from_source()
pub fn extract_wifi_drivers_from_local_windows() -> Result<PathBuf, String> {
    println!("\n--- Extracting WiFi Drivers from Local Windows ---");

    let windows_dir = PathBuf::from(r"C:\Windows");
    let inf_dir = windows_dir.join("INF");
    let sys_drivers = windows_dir.join("System32").join("Drivers");

    if !inf_dir.exists() {
        return Err("Cannot find C:\\Windows\\INF - must build from a full Windows installation".to_string());
    }

    // Create temp folder for extracted WiFi drivers
    let extract_dir = std::env::temp_dir().join("MasterBooter_WiFi_Drivers");
    if extract_dir.exists() {
        let _ = fs::remove_dir_all(&extract_dir);
    }
    fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create temp driver folder: {}", e))?;

    // WiFi driver INF files by manufacturer (from PhoenixPE NetworkDrivers.script)
    // These are the standard Windows inbox WiFi drivers covering most hardware.
    // x64 only — our PE target is always x64.
    let wifi_inf_files: &[(&str, &[&str])] = &[
        // Intel WiFi drivers (covers WiFi 5, 6, 6E, 7 adapters)
        ("Intel", &[
            "netwbw02.inf",     // Intel Dual Band Wireless older
            "netwew00.inf",     // Intel Wireless older
            "netwew01.inf",     // Intel Wireless
            "netwlv64.inf",     // Intel WiFi Link
            "netwns64.inf",     // Intel WiFi
            "netwsw00.inf",     // Intel WiFi
            "netwtw02.inf",     // Intel WiFi 5 (AC)
            "netwtw04.inf",     // Intel WiFi 6 (AX200/201)
            "netwtw06.inf",     // Intel WiFi 6E (AX210/211)
            "netwtw08.inf",     // Intel WiFi 7 (BE200)
            "netwtw10.inf",     // Intel WiFi newer (Windows 11 24H2+)
        ]),
        // Broadcom WiFi drivers (common in older laptops, some Dell/HP)
        ("Broadcom", &[
            "bcmwdidhdpcie.inf",
            "netbc63a.inf",
            "netbc64.inf",
        ]),
        // Realtek WiFi drivers (very common in budget/mid-range laptops)
        ("Realtek", &[
            "net8185.inf",
            "net8187bv64.inf",
            "net8187se64.inf",
            "net8192se64.inf",
            "net8192su64.inf",
            "net819xp.inf",
            "netrtwlane01.inf",
            "netrtwlane_13.inf",
            "netrtwlane.inf",
            "netrtwlanu.inf",
        ]),
        // Qualcomm/Atheros WiFi drivers (common in some Dell, Lenovo, HP)
        ("Qualcomm", &[
            "athw8x.inf",
            "netathrx.inf",
            "netathr10x.inf",
        ]),
        // Ralink/MediaTek WiFi drivers
        ("Ralink_MediaTek", &[
            "netr28ux.inf",
            "netr28x.inf",
            "netr7364.inf",
        ]),
        // Marvell WiFi drivers (some Surface devices, older laptops)
        ("Marvell", &[
            "mrvlpcie8897.inf",
        ]),
    ];

    let mut total_copied = 0;
    let mut total_missing = 0;

    for (manufacturer, inf_files) in wifi_inf_files {
        // Create a subfolder per manufacturer for organization
        let mfr_dir = extract_dir.join(manufacturer);
        let _ = fs::create_dir_all(&mfr_dir);

        let mut mfr_copied = 0;

        for inf_name in *inf_files {
            let inf_source = inf_dir.join(inf_name);

            if !inf_source.exists() {
                // This is normal — not all Windows versions have all drivers
                total_missing += 1;
                continue;
            }

            // Copy the .inf file
            let inf_dest = mfr_dir.join(inf_name);
            if let Err(e) = fs::copy(&inf_source, &inf_dest) {
                println!("  Warning: Failed to copy {}: {}", inf_name, e);
                continue;
            }

            // Parse the INF to find associated .sys driver files
            // The INF file lists driver binaries in [SourceDisksFiles] or references
            // We also look for matching .sys files by convention
            if let Ok(inf_content) = fs::read_to_string(&inf_source) {
                // Extract .sys filenames mentioned in the INF
                for line in inf_content.lines() {
                    let trimmed = line.trim().to_lowercase();
                    // Look for .sys references in the INF
                    if trimmed.ends_with(".sys") || trimmed.contains(".sys,") || trimmed.contains(".sys ") {
                        // Extract the .sys filename
                        let parts: Vec<&str> = line.split(|c: char| c == '=' || c == ',' || c == ';' || c == ' ')
                            .map(|s| s.trim())
                            .filter(|s| s.to_lowercase().ends_with(".sys"))
                            .collect();

                        for sys_name in parts {
                            let sys_name = sys_name.trim();
                            if sys_name.is_empty() { continue; }

                            // Try to find the .sys file in System32\Drivers
                            let sys_source = sys_drivers.join(sys_name);
                            if sys_source.exists() {
                                let sys_dest = mfr_dir.join(sys_name);
                                let _ = fs::copy(&sys_source, &sys_dest);
                            }
                        }
                    }
                }

                // Also copy any .cat (catalog) files with matching names
                let inf_stem = Path::new(inf_name).file_stem()
                    .and_then(|s| s.to_str()).unwrap_or("");
                // Look for .cat files in the CatRoot or alongside the INF
                let cat_name = format!("{}.cat", inf_stem);
                let cat_source = inf_dir.join(&cat_name);
                if cat_source.exists() {
                    let _ = fs::copy(&cat_source, &mfr_dir.join(&cat_name));
                }
            }

            mfr_copied += 1;
            total_copied += 1;
        }

        if mfr_copied > 0 {
            println!("  {} - {} driver INFs extracted", manufacturer, mfr_copied);
        }
    }

    println!("  Total: {} WiFi drivers extracted, {} not present on this system",
             total_copied, total_missing);

    if total_copied == 0 {
        return Err("No WiFi driver INF files found in C:\\Windows\\INF. \
            This Windows installation may not have inbox WiFi drivers.".to_string());
    }

    println!("  Drivers saved to: {}", extract_dir.display());
    println!("--- WiFi driver extraction complete ---\n");

    Ok(extract_dir)
}

/// Recursively copy an entire directory tree from src to dst.
/// Creates all subdirectories and copies all files.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create dir {}: {}", dst.display(), e))?;

    let entries = fs::read_dir(src)
        .map_err(|e| format!("Failed to read dir {}: {}", src.display(), e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)
                .map_err(|e| format!("Failed to copy {}: {}", path.display(), e))?;
        }
    }
    Ok(())
}

pub fn inject_wifi_support(mount_dir: &Path, source_windows_dir: &Path) -> Result<(), String> {
    println!("\n--- Injecting WiFi/WLAN Support ---");
    println!("  Source: {}", source_windows_dir.display());

    // source_windows_dir points to the extracted Windows directory
    // (e.g., <temp>/1/Windows/ from the ISO's install.wim)
    let sys32 = source_windows_dir.join("System32");

    // Verify the source has a System32 folder
    if !sys32.exists() {
        return Err(format!(
            "Cannot find System32 in WiFi source: {}\n\
             WiFi extraction from ISO may have failed.",
            sys32.display()
        ));
    }

    let pe_sys32 = mount_dir.join("Windows").join("System32");
    let pe_drivers = pe_sys32.join("Drivers");

    // Make sure destination directories exist
    let _ = fs::create_dir_all(&pe_sys32);
    let _ = fs::create_dir_all(&pe_drivers);

    // ============================================
    // STEP A: Copy WLAN DLLs and executables
    // ============================================
    // These are the core files that make up the WLAN service infrastructure.
    // Without these, "net start wlansvc" will fail because the service doesn't exist.

    let wlan_dlls = [
        // ===== Core WLAN service and API files (REQUIRED) =====
        "wlansvc.dll",          // WLAN AutoConfig service DLL
        "wlanapi.dll",          // WLAN API (used by PENetwork and other tools)
        "wlancfg.dll",          // WLAN configuration (used by netsh wlan)
        "wlanhlp.dll",          // WLAN helper library
        "wlanmsm.dll",          // WLAN media streaming manager
        "wlansec.dll",          // WLAN security
        "wlanui.dll",           // WLAN user interface components
        "wlgpclnt.dll",        // WLAN Group Policy client
        "wlanext.exe",          // WLAN extensibility framework
        "wifitask.exe",         // WiFi background task
        // ===== Additional WLAN DLLs (PhoenixPE includes these) =====
        "WLanConn.dll",         // WLAN connection dialog
        "wlandlg.dll",          // WLAN dialog
        "WLanHC.dll",           // WLAN health check
        "WlanMediaManager.dll", // WLAN media manager
        "WlanMM.dll",           // WLAN multimedia
        "wlanpref.dll",         // WLAN preferences
        "wlansvcpal.dll",       // WLAN service PAL (Platform Abstraction Layer)
        "wlanutil.dll",         // WLAN utilities
        "WlanRadioManager.dll", // WLAN radio/airplane mode manager
        "mobilenetworking.dll", // Mobile networking support
        // ===== dot3 (802.1X) DLLs — needed for WiFi authentication =====
        "dot3api.dll",          // dot3 API (wired/wireless 802.1X)
        "dot3cfg.dll",          // dot3 configuration
        "dot3dlg.dll",          // dot3 dialog
        "dot3gpclnt.dll",       // dot3 Group Policy client
        "dot3gpui.dll",         // dot3 GP UI
        "dot3hc.dll",           // dot3 health check
        "dot3msm.dll",          // dot3 media streaming manager
        "dot3svc.dll",          // dot3 service DLL
        "dot3ui.dll",           // dot3 user interface
        // ===== L2/802.1X authentication DLLs =====
        "l2gpstore.dll",        // L2 GP store
        "l2nacp.dll",           // L2 NACP (Network Access Control Protocol)
        "onex.dll",             // 802.1X authentication engine
        "onexui.dll",           // 802.1X UI
        // ===== Windows Connection Manager (WCM) DLLs =====
        // wcmsvc is a dependency of WlanSvc — PhoenixPE installs it fully
        "wcmapi.dll",           // WCM API
        "wcmcsp.dll",           // WCM CSP (Configuration Service Provider)
        "wcmsvc.dll",           // WCM service DLL
        "NetworkUXBroker.dll",  // Network UX broker (notifications)
        // ===== Cryptographic provider DLLs =====
        // rsaenh.dll is the RSA Enhanced Cryptographic Provider — it implements
        // the actual WPA-PSK/WPA2-PSK key derivation and encryption. Without it,
        // the WiFi handshake fails even if all WLAN services start correctly.
        // PhoenixPE includes this, and every PENetwork guide mentions it.
        "rsaenh.dll",           // RSA Enhanced Crypto Provider (WPA2 key handshake)
        // ===== EAP credential DLLs =====
        "cngcredui.dll",        // CNG credential UI (EAP authentication)
        "cngprovider.dll",      // CNG provider (EAP)
        // ===== Network helper DLLs =====
        "VAN.dll",              // Virtual Adapter Networking
        "RMapi.dll",            // Radio Management API
        "netevent.dll",         // Network event logging
        // ===== Dependency DLLs (required for Windows 10 1607+) =====
        // Without these, wlancfg.dll fails to load and netsh wlan commands break
        "dmcmnutils.dll",       // Device Management common utilities
        "mdmregistration.dll",  // MDM registration
        "mdmpostprocessevaluator.dll", // MDM post-process evaluator
    ];

    let mut copied_count = 0;
    let mut missing_count = 0;

    for dll in &wlan_dlls {
        let source = sys32.join(dll);
        let dest = pe_sys32.join(dll);
        if source.exists() {
            match fs::copy(&source, &dest) {
                Ok(_) => {
                    copied_count += 1;
                    println!("  Copied: {}", dll);
                }
                Err(e) => {
                    println!("  Warning: Failed to copy {}: {}", dll, e);
                }
            }
        } else {
            missing_count += 1;
            println!("  Not found (may be OK): {}", dll);
        }
    }

    println!("  WLAN DLLs: {} copied, {} not found", copied_count, missing_count);

    // Also copy en-US MUI files for wlanext and wlancfg
    let pe_en_us = pe_sys32.join("en-US");
    let _ = fs::create_dir_all(&pe_en_us);
    let sys32_en_us = sys32.join("en-US");
    for mui in &["wlanext.exe.mui", "wlancfg.dll.mui"] {
        let source = sys32_en_us.join(mui);
        let dest = pe_en_us.join(mui);
        if source.exists() {
            let _ = fs::copy(&source, &dest);
        }
    }

    // ============================================
    // STEP B: Copy NativeWiFi driver files
    // ============================================
    // These kernel-mode drivers are required for the WiFi stack to function.
    // nwifi.sys is the core NativeWiFi driver that all WiFi adapters depend on.

    let driver_files = [
        ("Drivers/nwifi.sys", "nwifi.sys"),           // Core NativeWiFi driver
        ("Drivers/vwififlt.sys", "vwififlt.sys"),     // Virtual WiFi filter
        ("Drivers/vwifibus.sys", "vwifibus.sys"),     // Virtual WiFi bus
        ("Drivers/WdiWiFi.sys", "WdiWiFi.sys"),      // WiFi diagnostics driver
        ("Drivers/wfplwfs.sys", "wfplwfs.sys"),       // Windows Filtering Platform Lightweight Filter
    ];

    for (src_rel, name) in &driver_files {
        let source = sys32.join(src_rel);
        let dest = pe_drivers.join(name);
        if source.exists() {
            match fs::copy(&source, &dest) {
                Ok(_) => println!("  Copied driver: {}", name),
                Err(e) => println!("  Warning: Failed to copy driver {}: {}", name, e),
            }
        } else {
            println!("  Driver not found (may be OK): {}", name);
        }
    }

    // Copy INF files for the WiFi drivers
    let inf_dir = source_windows_dir.join("INF");
    let pe_inf = mount_dir.join("Windows").join("INF");
    let _ = fs::create_dir_all(&pe_inf);

    let inf_files = [
        "netnwifi.inf",        // NativeWiFi protocol driver
        "netvwififlt.inf",     // Virtual WiFi filter driver
        "netvwifibus.inf",     // Virtual WiFi bus driver
        "netlldp.inf",         // LLDP (Link Layer Discovery Protocol)
        "ndiscap.inf",         // NDIS capture filter
    ];

    for inf in &inf_files {
        let source = inf_dir.join(inf);
        let dest = pe_inf.join(inf);
        if source.exists() {
            let _ = fs::copy(&source, &dest);
            println!("  Copied INF: {}", inf);
        }
    }

    // Copy WiFi protocol DriverStore packages (contain Ndi binding info)
    // These are the protocol-level driver packages, NOT adapter drivers.
    // They tell Windows how NativeWifiP, vwifibus, vwififlt bind to the network stack.
    let ds_src = sys32.join("DriverStore").join("FileRepository");
    let pe_ds = pe_sys32.join("DriverStore").join("FileRepository");
    if ds_src.exists() {
        let _ = fs::create_dir_all(&pe_ds);
        let wifi_ds_patterns = ["netnwifi.inf", "netvwifibus.inf", "netvwififlt.inf", "netvwifimp.inf"];
        for pattern in &wifi_ds_patterns {
            // Each DriverStore folder looks like "netnwifi.inf_amd64_abc123..."
            if let Ok(entries) = fs::read_dir(&ds_src) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name.starts_with(pattern) {
                        let src_folder = entry.path();
                        let dst_folder = pe_ds.join(entry.file_name());
                        // Recursively copy the entire DriverStore package folder
                        if let Err(e) = copy_dir_recursive(&src_folder, &dst_folder) {
                            println!("  Warning: Failed to copy DriverStore {}: {}", name, e);
                        } else {
                            println!("  Copied DriverStore package: {}", name);
                        }
                    }
                }
            }
        }
    }

    // ============================================
    // STEP C: Copy L2Schemas (WLAN profile schemas)
    // ============================================
    // Without these XML schema files, wlansvc fails with "The handle is invalid"
    // when trying to parse WiFi profiles.

    let l2schemas_src = source_windows_dir.join("L2Schemas");
    let l2schemas_dest = mount_dir.join("Windows").join("L2Schemas");
    if l2schemas_src.exists() {
        let _ = fs::create_dir_all(&l2schemas_dest);
        if let Ok(entries) = fs::read_dir(&l2schemas_src) {
            let mut schema_count = 0;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e.to_string_lossy().to_lowercase() == "xsd") {
                    let dest = l2schemas_dest.join(entry.file_name());
                    let _ = fs::copy(&path, &dest);
                    schema_count += 1;
                }
            }
            println!("  Copied {} L2Schema files", schema_count);
        }
    }

    // Also copy AvailableNetwork schemas
    let avail_net_src = source_windows_dir.join("schemas").join("AvailableNetwork");
    let avail_net_dest = mount_dir.join("Windows").join("schemas").join("AvailableNetwork");
    if avail_net_src.exists() {
        let _ = fs::create_dir_all(&avail_net_dest);
        if let Ok(entries) = fs::read_dir(&avail_net_src) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e.to_string_lossy().to_lowercase() == "xsd") {
                    let dest = avail_net_dest.join(entry.file_name());
                    let _ = fs::copy(&path, &dest);
                }
            }
        }
    }

    // ============================================
    // STEP C.5: Copy wlan.mof (WMI WiFi definition file)
    // ============================================
    // wlan.mof defines WMI classes for WiFi (e.g., MSNdis_80211_*).
    // Some network tools and PENetwork extensions use WMI to query WiFi state.
    // PhoenixPE copies this file. Located at System32\wbem\wlan.mof in install.wim.
    let wbem_src = source_windows_dir.join("System32").join("wbem");
    let wbem_dest = mount_dir.join("Windows").join("System32").join("wbem");
    let wlan_mof_src = wbem_src.join("wlan.mof");
    if wlan_mof_src.exists() {
        // wbem directory should already exist in PE, but ensure it does
        let _ = fs::create_dir_all(&wbem_dest);
        match fs::copy(&wlan_mof_src, wbem_dest.join("wlan.mof")) {
            Ok(_) => println!("  Copied wlan.mof (WMI WiFi definitions)"),
            Err(e) => println!("  Warning: Failed to copy wlan.mof: {}", e),
        }
    } else {
        println!("  wlan.mof not found in source (may be OK for older Windows versions)");
    }

    // ============================================
    // STEP D: Copy WLAN service registry entries from install.wim
    // ============================================
    // CRITICAL CHANGE: Instead of manually creating individual registry values
    // (which was missing critical subkeys like NativeWifiP\Linkage, Ndi binding
    // info, network filter registrations, etc.), we now copy ENTIRE service
    // subtrees from install.wim's SYSTEM/SOFTWARE hives into the PE's hives.
    //
    // This approach matches how PhoenixPE does it — using "reg copy /s /f" to
    // get ALL subkeys, parameters, security descriptors, and binding info
    // automatically. The old manual approach was confirmed NOT working because
    // it missed critical registry subkeys that Windows needs for WLAN binding.

    println!("  Copying WLAN service registry entries from install.wim...");

    // PE hive paths (inside the mounted WIM)
    let pe_system_hive = pe_sys32.join("config").join("SYSTEM");
    let pe_software_hive = pe_sys32.join("config").join("SOFTWARE");

    // Source hive paths (extracted from install.wim via 7-Zip)
    let src_system_hive = sys32.join("config").join("SYSTEM");
    let src_software_hive = sys32.join("config").join("SOFTWARE");

    if !pe_system_hive.exists() {
        println!("  Warning: PE SYSTEM hive not found at {}", pe_system_hive.display());
        println!("  WiFi may not work - registry entries could not be added");
        return Ok(());
    }

    if !src_system_hive.exists() {
        println!("  Warning: Source SYSTEM hive not found at {}", src_system_hive.display());
        println!("  The SYSTEM hive was not extracted from install.wim.");
        println!("  WiFi registry entries cannot be copied — WiFi will not work.");
        return Ok(());
    }

    // Helper: Load a registry hive, handling "already loaded" gracefully.
    // Returns true if the hive is now loaded (either freshly or was already).
    fn load_hive(key_name: &str, hive_path: &Path) -> bool {
        // Try to unload first in case it was left from a previous run
        let _ = Command::new("reg").args(["unload", key_name]).output();

        let result = Command::new("reg")
            .args(["load", key_name, &hive_path.to_string_lossy()])
            .output();

        match result {
            Ok(out) => {
                if out.status.success() {
                    println!("  Loaded hive: {} -> {}", hive_path.display(), key_name);
                    true
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    if stderr.contains("already in use") || stderr.contains("being used") {
                        println!("  Hive already loaded: {}", key_name);
                        true
                    } else {
                        println!("  Warning: Failed to load hive {}: {}", key_name, stderr.trim());
                        false
                    }
                }
            }
            Err(e) => {
                println!("  Warning: Could not run reg load for {}: {}", key_name, e);
                false
            }
        }
    }

    // Helper: Copy a registry subtree from source to destination.
    // Uses "reg copy /s /f" which copies ALL subkeys and values recursively.
    fn reg_copy_subtree(src_key: &str, dst_key: &str, name: &str) {
        let result = Command::new("reg")
            .args(["copy", src_key, dst_key, "/s", "/f"])
            .output();

        match result {
            Ok(out) => {
                if out.status.success() {
                    println!("    Copied: {}", name);
                } else {
                    // Not all keys exist in every Windows version — this is OK
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    if stderr.contains("unable to find") || stderr.contains("not find") {
                        println!("    Not found (OK): {}", name);
                    } else {
                        println!("    Warning: {} - {}", name, stderr.trim());
                    }
                }
            }
            Err(e) => println!("    Warning: reg copy failed for {}: {}", name, e),
        }
    }

    // ============================================
    // STEP D.1: Load all four hives
    // ============================================
    // We load the install.wim's SYSTEM as SRC-SYSTEM, and the PE's SYSTEM as PE-SYSTEM.
    // Then we copy service subtrees from SRC to PE using "reg copy /s /f".
    let src_sys_loaded = load_hive(r"HKLM\SRC-SYSTEM", &src_system_hive);
    let pe_sys_loaded = load_hive(r"HKLM\PE-SYSTEM", &pe_system_hive);

    if src_sys_loaded && pe_sys_loaded {
        // ============================================
        // STEP D.2: Copy service subtrees from install.wim → PE
        // ============================================
        // These are the complete service registrations that the WLAN stack needs.
        // Copying entire subtrees gets ALL subkeys (Linkage, Ndi, Parameters,
        // Security, Enum, etc.) that manual "reg add" commands were missing.

        println!("  Copying WLAN service subtrees...");

        // --- Core WLAN services ---
        let services = [
            ("WlanSvc",      "WLAN AutoConfig service"),
            ("Wcmsvc",       "Windows Connection Manager"),
            ("NativeWifiP",  "NativeWiFi protocol driver"),
            ("vwifibus",     "Virtual WiFi bus driver"),
            ("vwififlt",     "Virtual WiFi filter driver"),
            ("wdiwifi",      "WiFi Diagnostics driver"),
            ("WFPLWFS",      "WFP Lightweight Filter driver"),
            ("dot3svc",      "Wired AutoConfig (802.1X dependency)"),
            ("EapHost",      "EAP authentication host"),
            ("wcncsvc",      "Windows Connect Now service"),
            ("tdx",          "TDI translation layer"),
            // --- Network state/connectivity services ---
            // netprofm = Network List Manager — PENetwork queries it to determine
            // whether WiFi is connected/disconnected and public/private. Without
            // the full service definition (not just AllowStart), WinPE doesn't
            // even know what netprofm IS.
            ("netprofm",     "Network List Manager (PENetwork needs this)"),
            // NlaSvc = Network Location Awareness — detects whether you actually
            // have internet connectivity after connecting to WiFi. PENetwork and
            // Windows networking depend on NlaSvc to report network status.
            ("NlaSvc",       "Network Location Awareness (connectivity detection)"),
        ];

        for (svc_name, description) in &services {
            let src_key = format!(r"HKLM\SRC-SYSTEM\ControlSet001\Services\{}", svc_name);
            let dst_key = format!(r"HKLM\PE-SYSTEM\ControlSet001\Services\{}", svc_name);
            reg_copy_subtree(&src_key, &dst_key, description);
        }

        // --- WLAN event log registration ---
        reg_copy_subtree(
            r"HKLM\SRC-SYSTEM\ControlSet001\Services\EventLog\System\Microsoft-Windows-WLAN-AutoConfig",
            r"HKLM\PE-SYSTEM\ControlSet001\Services\EventLog\System\Microsoft-Windows-WLAN-AutoConfig",
            "WLAN event log",
        );

        // ============================================
        // STEP D.3: Copy network filter/binding registrations
        // ============================================
        // These tell Windows how NativeWifiP and WFPLWFS bind to the network stack.
        // Without these, the WiFi driver loads but can't communicate with the stack.

        println!("  Copying network binding registrations...");

        // Network filter GUIDs for WFPLWFS and vwifibus
        let network_guids = [
            ("{5CBF81BF-5055-47CD-9055-A76B2B4E3698}", "vwifibus network binding"),
            ("{3BFD7820-D65C-4C1B-9FEA-983A019639EA}", "WFPLWFS filter #1"),
            ("{B70D6460-3635-4D42-B866-B8AB1A24454C}", "WFPLWFS filter #2"),
            ("{E7C3B2F0-F3C5-48DF-AF2B-10FED6D72E7A}", "WFPLWFS filter #3 (x64)"),
            ("{E475CF9A-60CD-4439-A75F-0079CE0E18A1}", "WFPLWFS filter #4"),
        ];

        let net_class = r"{4d36e974-e325-11ce-bfc1-08002be10318}";
        for (guid, description) in &network_guids {
            let src_key = format!(
                r"HKLM\SRC-SYSTEM\ControlSet001\Control\Network\{}\{}",
                net_class, guid
            );
            let dst_key = format!(
                r"HKLM\PE-SYSTEM\ControlSet001\Control\Network\{}\{}",
                net_class, guid
            );
            reg_copy_subtree(&src_key, &dst_key, description);
        }

        // Copy NetworkSetup2 filter/plugin registrations
        // These are critical for NativeWifiP and WFPLWFS to bind properly
        reg_copy_subtree(
            r"HKLM\SRC-SYSTEM\ControlSet001\Control\NetworkSetup2\Filters",
            r"HKLM\PE-SYSTEM\ControlSet001\Control\NetworkSetup2\Filters",
            "NetworkSetup2 Filters",
        );
        reg_copy_subtree(
            r"HKLM\SRC-SYSTEM\ControlSet001\Control\NetworkSetup2\Plugins",
            r"HKLM\PE-SYSTEM\ControlSet001\Control\NetworkSetup2\Plugins",
            "NetworkSetup2 Plugins",
        );

        // ============================================
        // STEP D.4: Copy Winlogon notification components
        // ============================================
        // These enable dot3svc and WlanSvc to receive session change events
        // from Winlogon, which are needed for proper service initialization.

        println!("  Copying Winlogon notification components...");
        reg_copy_subtree(
            r"HKLM\SRC-SYSTEM\ControlSet001\Control\Winlogon\Notifications\Components\Dot3svc",
            r"HKLM\PE-SYSTEM\ControlSet001\Control\Winlogon\Notifications\Components\Dot3svc",
            "Dot3svc Winlogon notification",
        );
        reg_copy_subtree(
            r"HKLM\SRC-SYSTEM\ControlSet001\Control\Winlogon\Notifications\Components\Wlansvc",
            r"HKLM\PE-SYSTEM\ControlSet001\Control\Winlogon\Notifications\Components\Wlansvc",
            "Wlansvc Winlogon notification",
        );

        // ============================================
        // STEP D.5: Copy additional Control keys
        // ============================================
        println!("  Copying additional WiFi control keys...");

        // WiFi WMI tracing session
        reg_copy_subtree(
            r"HKLM\SRC-SYSTEM\ControlSet001\Control\WMI\Autologger\WiFiSession",
            r"HKLM\PE-SYSTEM\ControlSet001\Control\WMI\Autologger\WiFiSession",
            "WiFi WMI tracing session",
        );

        // Radio Management (airplane mode support)
        reg_copy_subtree(
            r"HKLM\SRC-SYSTEM\ControlSet001\Control\RadioManagement",
            r"HKLM\PE-SYSTEM\ControlSet001\Control\RadioManagement",
            "Radio Management",
        );

        // ============================================
        // STEP D.6: Add AllowStart entries
        // ============================================
        // In WinPE, services need explicit AllowStart entries under Setup
        // to be allowed to start. Without these, "net start wlansvc" may fail.
        println!("  Adding AllowStart entries for WiFi services...");

        let allow_start_services = ["dnscache", "nlasvc", "wcmsvc", "netprofm", "WlanSvc"];
        for svc in &allow_start_services {
            let key = format!(r"HKLM\PE-SYSTEM\Setup\AllowStart\{}", svc);
            // AllowStart entries are just empty keys (REG_NONE) — no values needed
            let _ = Command::new("reg").args(["add", &key, "/f"]).output();
            println!("    AllowStart: {}", svc);
        }

        // ============================================
        // STEP D.7: Write NetworkSetup2 filter class values
        // ============================================
        // These FilterClass values tell the network stack how WFPLWFS filters
        // should be ordered. Required for NativeWifiP and WlanSvc to work.
        println!("  Writing NetworkSetup2 FilterClass values...");

        let filter_guids = [
            "{3BFD7820-D65C-4C1B-9FEA-983A019639EA}",
            "{B70D6460-3635-4D42-B866-B8AB1A24454C}",
            "{E475CF9A-60CD-4439-A75F-0079CE0E18A1}",
        ];
        for guid in &filter_guids {
            let key = format!(
                r"HKLM\PE-SYSTEM\ControlSet001\Control\NetworkSetup2\Filters\{}\Kernel",
                guid
            );
            let _ = Command::new("reg").args([
                "add", &key, "/v", "FilterClass",
                "/t", "REG_SZ", "/d", "ms_medium_converter_top", "/f",
            ]).output();
        }
        println!("    Set FilterClass for 3 WFPLWFS filters");

        println!("  SYSTEM hive registry copy complete");
    } else {
        println!("  Warning: Could not load SYSTEM hives for registry copy");
        println!("  WiFi registry entries will be missing — WiFi will not work");
    }

    // Always unload SYSTEM hives (even if there were errors)
    let _ = Command::new("reg").args(["unload", r"HKLM\SRC-SYSTEM"]).output();
    let _ = Command::new("reg").args(["unload", r"HKLM\PE-SYSTEM"]).output();
    println!("  Unloaded SYSTEM hives");

    // ============================================
    // STEP D.8: Copy SOFTWARE hive entries
    // ============================================
    // The SOFTWARE hive contains WlanSvc/wcmsvc configuration, netsh helper
    // registration, svchost group assignments, and the 24H2 WiFi fix.

    println!("  Copying SOFTWARE hive entries...");

    let src_sw_loaded = if src_software_hive.exists() {
        load_hive(r"HKLM\SRC-SOFTWARE", &src_software_hive)
    } else {
        println!("  Source SOFTWARE hive not found — using PE hive only");
        false
    };

    let pe_sw_loaded = if pe_software_hive.exists() {
        load_hive(r"HKLM\PE-SOFTWARE", &pe_software_hive)
    } else {
        println!("  Warning: PE SOFTWARE hive not found");
        false
    };

    if pe_sw_loaded {
        // Copy SOFTWARE subtrees from install.wim if available
        if src_sw_loaded {
            // WlanSvc and wcmsvc configuration
            reg_copy_subtree(
                r"HKLM\SRC-SOFTWARE\Microsoft\WlanSvc",
                r"HKLM\PE-SOFTWARE\Microsoft\WlanSvc",
                "WlanSvc SOFTWARE config",
            );
            reg_copy_subtree(
                r"HKLM\SRC-SOFTWARE\Microsoft\wcmsvc",
                r"HKLM\PE-SOFTWARE\Microsoft\wcmsvc",
                "wcmsvc SOFTWARE config",
            );
            reg_copy_subtree(
                r"HKLM\SRC-SOFTWARE\Policies\Microsoft\Windows\WcmSvc",
                r"HKLM\PE-SOFTWARE\Policies\Microsoft\Windows\WcmSvc",
                "WCM service policies",
            );
        }

        // Register netsh wlan helper DLL (enables "netsh wlan show networks" etc.)
        let netsh_path = r"HKLM\PE-SOFTWARE\Microsoft\NetSh";
        let _ = Command::new("reg").args(["add", netsh_path, "/v", "wlancfg",
            "/t", "REG_SZ", "/d", "wlancfg.dll", "/f"]).output();
        println!("    Added netsh wlan helper registration");

        // Add wlansvc to the LocalSystemNetworkRestricted svchost group
        // This tells svchost.exe which services belong to this group.
        // We use PowerShell to safely append to the existing MULTI_SZ value.
        let ps_cmd = concat!(
            "$path = 'HKLM:\\PE-SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Svchost'; ",
            "$val = (Get-ItemProperty -Path $path -Name 'LocalSystemNetworkRestricted' ",
            "-ErrorAction SilentlyContinue).LocalSystemNetworkRestricted; ",
            "$add = @('WlanSvc','Wcmsvc','dot3svc'); ",
            "if ($val) { ",
            "  foreach ($s in $add) { if ($val -notcontains $s) { $val = @($val) + $s } }; ",
            "  Set-ItemProperty -Path $path -Name 'LocalSystemNetworkRestricted' -Value $val -Type MultiString ",
            "} else { ",
            "  New-ItemProperty -Path $path -Name 'LocalSystemNetworkRestricted' ",
            "  -Value $add -PropertyType MultiString -Force ",
            "}"
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", ps_cmd])
            .output();
        println!("    Added WlanSvc/Wcmsvc/dot3svc to svchost group");

        // ============================================
        // STEP D.9: Windows 11 24H2 WiFi fix
        // ============================================
        // Windows 11 24H2 introduced a CapabilityAccessManager check that
        // causes a BLANK WiFi network list if the wlanLocationBypass
        // capability isn't present. This fixes it by setting RequireWindowsCert=0.
        // Reference: PhoenixPE issue #147
        let cap_key = r"HKLM\PE-SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\Capabilities\wlanLocationBypass";
        let _ = Command::new("reg").args([
            "add", cap_key, "/v", "RequireWindowsCert",
            "/t", "REG_DWORD", "/d", "0", "/f",
        ]).output();
        println!("    Added 24H2 WiFi fix (wlanLocationBypass)");

        println!("  SOFTWARE hive registry copy complete");
    }

    // Always unload SOFTWARE hives
    let _ = Command::new("reg").args(["unload", r"HKLM\SRC-SOFTWARE"]).output();
    let _ = Command::new("reg").args(["unload", r"HKLM\PE-SOFTWARE"]).output();
    println!("  Unloaded SOFTWARE hives");

    println!("--- WiFi/WLAN injection complete ---\n");
    println!("  At PE boot, the launcher will run 'net start wlansvc' to activate WiFi.");
    println!("  PENetwork can then enumerate and connect to wireless networks.");

    Ok(())
}

// ============================================
// PUBLIC API FOR UI
// ============================================
// These functions expose the package and fix information to the UI

/// Get all available ADK packages for display in the UI
#[allow(dead_code)]
pub fn get_available_packages() -> Vec<AdkPackage> {
    adk_packages::get_all_packages()
}

/// Get default enabled package IDs
#[allow(dead_code)]
pub fn get_default_packages() -> Vec<String> {
    adk_packages::get_default_enabled_packages()
}

/// Get all available PE fixes for display in the UI
#[allow(dead_code)]
pub fn get_available_fixes() -> Vec<PeFix> {
    pe_fixes::get_all_fixes()
}

/// Get default enabled fix IDs
#[allow(dead_code)]
pub fn get_default_fixes() -> Vec<String> {
    pe_fixes::get_default_enabled_fixes()
}

/// Check if ADK packages are available on this system
#[allow(dead_code)]
pub fn check_adk_packages_available(architecture: &str) -> bool {
    adk_packages::detect_adk_packages_path(architecture).is_some()
}

// ============================================
// TESTS
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(0), "0 bytes");
        assert_eq!(format_file_size(500), "500 bytes");
        assert_eq!(format_file_size(1024), "1.00 KB");
        assert_eq!(format_file_size(1024 * 1024), "1.00 MB");
        assert_eq!(format_file_size(1024 * 1024 * 1024), "1.00 GB");
    }
}
