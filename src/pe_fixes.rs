// ============================================
// MasterBooter - pe_fixes.rs
// ============================================
// This module handles special fixes and workarounds for WinPE builds.
//
// WinPE has various quirks that need offline registry modifications at build
// time (before the image boots). These fixes modify registry hives inside the
// mounted WIM so the settings are active when WinPE starts.
//
// Fixes that create folders or set environment variables at runtime are NOT
// here — those are handled by the launcher script (launch.cmd) which runs
// at boot time after wpeinit.
//
// Based on research from:
// - AMPIPIT (removes WallpaperHost.exe)
// - Windows Setup Helper (DPI fix, font fix)
// - GhostWin (DPI fix via registry)
// ============================================

use std::path::Path;
use std::process::Command;
use std::fs;

// ============================================
// PE FIX DEFINITIONS
// ============================================
// Each fix can be toggled on/off in the UI.
// All fixes modify offline registry hives — they can't be done at boot time.

/// Represents a single PE fix/workaround
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PeFix {
    /// Internal identifier
    pub id: &'static str,

    /// Display name shown in the UI
    pub display_name: &'static str,

    /// Description of what this fix does
    pub description: &'static str,

    /// Category for grouping
    pub category: FixCategory,

    /// Whether enabled by default
    pub default_enabled: bool,

    /// Whether ADK is required for this fix
    pub requires_adk: bool,
}

/// Categories for organizing fixes in the UI
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FixCategory {
    /// Display and UI related fixes
    Display,
    /// Compatibility fixes
    Compatibility,
}

#[allow(dead_code)]
impl FixCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            FixCategory::Display => "Display & UI",
            FixCategory::Compatibility => "Compatibility",
        }
    }
}

/// Get all available PE fixes
///
/// These are offline registry modifications applied to the mounted WIM at
/// build time. Runtime tasks (profile folders, TEMP vars, env setup) are
/// handled by the launcher script instead.
pub fn get_all_fixes() -> Vec<PeFix> {
    vec![
        // ============================================
        // DISPLAY FIXES
        // ============================================

        PeFix {
            id: "dpi_scaling",
            display_name: "DPI Scaling Fix",
            description: "Fix blurry/small text on high-DPI displays by disabling automatic scaling",
            category: FixCategory::Display,
            default_enabled: true,
            requires_adk: false,
        },

        PeFix {
            id: "wallpaper_host",
            display_name: "Remove WallpaperHost.exe",
            description: "Remove WallpaperHost.exe to fix display issues with software rendering",
            category: FixCategory::Display,
            default_enabled: true,
            requires_adk: false,
        },

        PeFix {
            id: "font_fix",
            display_name: "Font Rendering Fix",
            description: "Fix Segoe UI italic rendering issue that causes garbled text",
            category: FixCategory::Display,
            default_enabled: true,
            requires_adk: false,
        },

        // ============================================
        // COMPATIBILITY
        // ============================================

        PeFix {
            id: "disable_crash_dialogs",
            display_name: "Disable Crash Dialogs",
            description: "Prevent Windows Error Reporting dialogs from appearing",
            category: FixCategory::Compatibility,
            default_enabled: true,
            requires_adk: false,
        },

        PeFix {
            id: "enable_long_paths",
            display_name: "Enable Long Paths",
            description: "Enable support for paths longer than 260 characters",
            category: FixCategory::Compatibility,
            default_enabled: true,
            requires_adk: false,
        },
    ]
}

/// Get fixes that should be enabled by default
pub fn get_default_enabled_fixes() -> Vec<String> {
    get_all_fixes()
        .iter()
        .filter(|f| f.default_enabled)
        .map(|f| f.id.to_string())
        .collect()
}

// ============================================
// FIX IMPLEMENTATION
// ============================================

/// Result of applying a fix
#[derive(Debug)]
#[allow(dead_code)]
pub struct FixResult {
    pub fix_id: String,
    pub fix_name: String,
    pub success: bool,
    pub message: String,
}

/// Options for fixes that need additional configuration
#[derive(Debug, Clone, Default)]
pub struct FixOptions {
    // Currently empty — all remaining fixes are self-contained.
    // Kept for API compatibility with PeBuildConfig.
}

/// Apply a single fix to a mounted WIM
pub fn apply_fix(mount_path: &Path, fix_id: &str, _options: &FixOptions) -> FixResult {
    match fix_id {
        "dpi_scaling" => apply_dpi_scaling_fix(mount_path),
        "wallpaper_host" => apply_wallpaper_host_fix(mount_path),
        "font_fix" => apply_font_fix(mount_path),
        "disable_crash_dialogs" => apply_crash_dialogs_fix(mount_path),
        "enable_long_paths" => apply_long_paths_fix(mount_path),
        _ => FixResult {
            fix_id: fix_id.to_string(),
            fix_name: fix_id.to_string(),
            success: false,
            message: format!("Unknown fix: {}", fix_id),
        },
    }
}

/// Apply all enabled fixes to a mounted WIM
pub fn apply_fixes(
    mount_path: &Path,
    enabled_fix_ids: &[String],
    options: &FixOptions,
    progress: impl Fn(&str, usize, usize),
) -> Vec<FixResult> {
    println!("Applying {} fixes...", enabled_fix_ids.len());

    let all_fixes = get_all_fixes();
    let fix_map: std::collections::HashMap<&str, &PeFix> = all_fixes
        .iter()
        .map(|f| (f.id, f))
        .collect();

    let total = enabled_fix_ids.len();
    let mut results = Vec::new();

    for (index, fix_id) in enabled_fix_ids.iter().enumerate() {
        if let Some(fix) = fix_map.get(fix_id.as_str()) {
            progress(fix.display_name, index + 1, total);
            let result = apply_fix(mount_path, fix_id, options);
            results.push(result);
        }
    }

    println!("Fix application complete. {} of {} succeeded",
        results.iter().filter(|r| r.success).count(),
        results.len()
    );

    results
}

// ============================================
// INDIVIDUAL FIX IMPLEMENTATIONS
// ============================================

/// Apply DPI scaling fix
///
/// This modifies the default user registry hive to disable automatic DPI scaling.
/// Based on GhostWin and Windows Setup Helper implementations.
///
/// Registry modifications:
/// - HKEY_USERS\.DEFAULT\Control Panel\Desktop\LogPixels = 96 (100% scaling)
/// - HKEY_USERS\.DEFAULT\Control Panel\Desktop\Win8DpiScaling = 1
/// - HKEY_USERS\.DEFAULT\Control Panel\Desktop\DpiScalingVer = 0x1018
fn apply_dpi_scaling_fix(mount_path: &Path) -> FixResult {
    println!("Applying DPI scaling fix...");

    // Path to the default user registry hive
    let default_hive = mount_path.join("Windows").join("System32").join("config").join("default");

    if !default_hive.exists() {
        return FixResult {
            fix_id: "dpi_scaling".to_string(),
            fix_name: "DPI Scaling Fix".to_string(),
            success: false,
            message: "Default registry hive not found".to_string(),
        };
    }

    // Load the hive with a temporary name
    let hive_name = "_WinPE_DPI_Fix";

    // Load the hive
    let load_result = Command::new("reg")
        .arg("load")
        .arg(format!("HKLM\\{}", hive_name))
        .arg(&default_hive)
        .output();

    if let Err(e) = load_result {
        return FixResult {
            fix_id: "dpi_scaling".to_string(),
            fix_name: "DPI Scaling Fix".to_string(),
            success: false,
            message: format!("Failed to load registry hive: {}", e),
        };
    }

    // Apply registry values
    let registry_commands = [
        // Set DPI to 96 (100% scaling)
        ("Control Panel\\Desktop", "LogPixels", "REG_DWORD", "96"),
        // Enable Win8 DPI scaling mode
        ("Control Panel\\Desktop", "Win8DpiScaling", "REG_DWORD", "1"),
        // Set DPI scaling version
        ("Control Panel\\Desktop", "DpiScalingVer", "REG_DWORD", "4120"),  // 0x1018
    ];

    let mut all_success = true;

    for (subkey, value_name, value_type, data) in registry_commands {
        let full_key = format!("HKLM\\{}\\{}", hive_name, subkey);

        let result = Command::new("reg")
            .arg("add")
            .arg(&full_key)
            .arg("/v")
            .arg(value_name)
            .arg("/t")
            .arg(value_type)
            .arg("/d")
            .arg(data)
            .arg("/f")
            .output();

        if let Ok(out) = result {
            if !out.status.success() {
                println!("  Warning: Failed to set {} in {}", value_name, subkey);
                all_success = false;
            }
        }
    }

    // Unload the hive
    let _ = Command::new("reg")
        .arg("unload")
        .arg(format!("HKLM\\{}", hive_name))
        .output();

    if all_success {
        println!("  DPI scaling fix applied successfully");
        FixResult {
            fix_id: "dpi_scaling".to_string(),
            fix_name: "DPI Scaling Fix".to_string(),
            success: true,
            message: "DPI scaling disabled (100% forced)".to_string(),
        }
    } else {
        FixResult {
            fix_id: "dpi_scaling".to_string(),
            fix_name: "DPI Scaling Fix".to_string(),
            success: false,
            message: "Some registry values could not be set".to_string(),
        }
    }
}

/// Remove WallpaperHost.exe to fix display issues, and set wallpaper via registry
///
/// From AMPIPIT: WallpaperHost.exe can cause display problems when
/// using software rendering in WinPE. WinXShell handles wallpaper natively
/// by reading the system wallpaper registry setting, so WallpaperHost is not needed.
///
/// After removing WallpaperHost, we also set the wallpaper path in the DEFAULT
/// user hive so that WinXShell displays the branding wallpaper on boot.
fn apply_wallpaper_host_fix(mount_path: &Path) -> FixResult {
    println!("Applying WallpaperHost.exe removal + wallpaper registry setup...");

    let wallpaper_host = mount_path
        .join("Windows")
        .join("System32")
        .join("WallpaperHost.exe");

    // ============================================
    // PART 1: Remove WallpaperHost.exe
    // ============================================
    // Files in the mounted WIM may be owned by TrustedInstaller,
    // so we need to take ownership and grant permissions before deleting.
    if wallpaper_host.exists() {
        let path_str = wallpaper_host.to_string_lossy().to_string();

        // Take ownership from TrustedInstaller so we can delete it
        let _ = Command::new("takeown")
            .args(["/f", &path_str, "/a"])
            .output();

        // Grant Administrators full control
        let _ = Command::new("icacls")
            .args([&path_str, "/grant", "Administrators:F"])
            .output();

        // Now try to delete
        match fs::remove_file(&wallpaper_host) {
            Ok(_) => {
                println!("  Removed WallpaperHost.exe");
            }
            Err(e) => {
                // Last resort: rename to .bak so it won't run
                println!("  Warning: Could not delete WallpaperHost.exe ({}), renaming to .bak", e);
                let bak_path = wallpaper_host.with_extension("exe.bak");
                if let Err(e2) = fs::rename(&wallpaper_host, &bak_path) {
                    println!("  Warning: Rename also failed: {}", e2);
                    return FixResult {
                        fix_id: "wallpaper_host".to_string(),
                        fix_name: "Remove WallpaperHost.exe".to_string(),
                        success: false,
                        message: format!("Failed to remove or rename: {}", e),
                    };
                }
                println!("  Renamed to WallpaperHost.exe.bak");
            }
        }
    } else {
        println!("  WallpaperHost.exe not found (already removed or not present)");
    }

    // ============================================
    // PART 2: Set wallpaper registry keys
    // ============================================
    // Load the DEFAULT user hive and write the wallpaper path so that
    // WinXShell will display it on boot. This is the same approach PhoenixPE uses.
    // The wallpaper file itself is injected by inject_branding() in winpe.rs.
    let default_hive = mount_path
        .join("Windows")
        .join("System32")
        .join("config")
        .join("default");

    if default_hive.exists() {
        let hive_name = "PE-DEFAULT";

        // Load the DEFAULT user registry hive
        let load_result = Command::new("reg")
            .args(["load", &format!("HKLM\\{}", hive_name), &default_hive.to_string_lossy()])
            .output();

        let hive_loaded = match load_result {
            Ok(out) => {
                if out.status.success() {
                    true
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    stderr.contains("already in use") || stderr.contains("being used")
                }
            }
            Err(_) => false,
        };

        if hive_loaded {
            // The wallpaper will be at this path inside the PE (X: drive)
            let wallpaper_path = r"X:\Windows\Web\Wallpaper\Windows\wallpaper.jpg";

            // Set the wallpaper path in Control Panel\Desktop
            let desktop_key = format!(r"HKLM\{}\Control Panel\Desktop", hive_name);
            let _ = Command::new("reg").args(["add", &desktop_key, "/v", "Wallpaper",
                "/t", "REG_SZ", "/d", wallpaper_path, "/f"]).output();
            let _ = Command::new("reg").args(["add", &desktop_key, "/v", "WallpaperStyle",
                "/t", "REG_SZ", "/d", "10", "/f"]).output();  // 10 = Fill (stretch to cover)
            let _ = Command::new("reg").args(["add", &desktop_key, "/v", "TileWallpaper",
                "/t", "REG_SZ", "/d", "0", "/f"]).output();

            // Also set in Internet Explorer Desktop\General (legacy path WinXShell may read)
            let ie_desktop_key = format!(
                r"HKLM\{}\Software\Microsoft\Internet Explorer\Desktop\General", hive_name
            );
            let _ = Command::new("reg").args(["add", &ie_desktop_key, "/v", "WallpaperSource",
                "/t", "REG_SZ", "/d", wallpaper_path, "/f"]).output();

            println!("  Set wallpaper registry keys -> {}", wallpaper_path);

            // Unload the hive
            let _ = Command::new("reg")
                .args(["unload", &format!("HKLM\\{}", hive_name)])
                .output();
        } else {
            println!("  Warning: Could not load DEFAULT hive for wallpaper registry keys");
        }
    } else {
        println!("  Warning: DEFAULT hive not found, skipping wallpaper registry setup");
    }

    FixResult {
        fix_id: "wallpaper_host".to_string(),
        fix_name: "Remove WallpaperHost.exe".to_string(),
        success: true,
        message: "WallpaperHost removed, wallpaper registry keys set".to_string(),
    }
}

/// Apply font rendering fix
///
/// From Windows Setup Helper: Fixes Segoe UI italic font rendering issue
/// by remapping the italic variant to the regular font.
fn apply_font_fix(mount_path: &Path) -> FixResult {
    println!("Applying font rendering fix...");

    // Create a .reg file with the font fixes
    let reg_content = r#"Windows Registry Editor Version 5.00

; Fix Segoe UI Italic rendering issue in WinPE
; Maps italic variant to regular to prevent garbled text

[HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts]
"Segoe UI Italic (TrueType)"="segoeui.ttf"
"Segoe UI Bold Italic (TrueType)"="segoeuib.ttf"
"#;

    // Write the reg file to the mount
    let reg_path = mount_path.join("Windows").join("Setup").join("FontFix.reg");

    // Ensure directory exists
    if let Some(parent) = reg_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    match fs::write(&reg_path, reg_content) {
        Ok(_) => {
            // Also apply directly to the SOFTWARE hive
            let software_hive = mount_path
                .join("Windows")
                .join("System32")
                .join("config")
                .join("SOFTWARE");

            if software_hive.exists() {
                let hive_name = "_WinPE_Font_Fix";

                // Load the hive
                let _ = Command::new("reg")
                    .arg("load")
                    .arg(format!("HKLM\\{}", hive_name))
                    .arg(&software_hive)
                    .output();

                // Apply font fixes
                let _ = Command::new("reg")
                    .arg("add")
                    .arg(format!("HKLM\\{}\\Microsoft\\Windows NT\\CurrentVersion\\Fonts", hive_name))
                    .arg("/v")
                    .arg("Segoe UI Italic (TrueType)")
                    .arg("/t")
                    .arg("REG_SZ")
                    .arg("/d")
                    .arg("segoeui.ttf")
                    .arg("/f")
                    .output();

                // Unload the hive
                let _ = Command::new("reg")
                    .arg("unload")
                    .arg(format!("HKLM\\{}", hive_name))
                    .output();
            }

            println!("  Font fix applied");
            FixResult {
                fix_id: "font_fix".to_string(),
                fix_name: "Font Rendering Fix".to_string(),
                success: true,
                message: "Segoe UI italic fix applied".to_string(),
            }
        }
        Err(e) => FixResult {
            fix_id: "font_fix".to_string(),
            fix_name: "Font Rendering Fix".to_string(),
            success: false,
            message: format!("Failed to write reg file: {}", e),
        },
    }
}

/// Disable Windows Error Reporting crash dialogs
fn apply_crash_dialogs_fix(mount_path: &Path) -> FixResult {
    println!("Applying crash dialogs fix...");

    let software_hive = mount_path
        .join("Windows")
        .join("System32")
        .join("config")
        .join("SOFTWARE");

    if !software_hive.exists() {
        return FixResult {
            fix_id: "disable_crash_dialogs".to_string(),
            fix_name: "Disable Crash Dialogs".to_string(),
            success: false,
            message: "SOFTWARE hive not found".to_string(),
        };
    }

    let hive_name = "_WinPE_Crash_Fix";

    // Load the hive
    let _ = Command::new("reg")
        .arg("load")
        .arg(format!("HKLM\\{}", hive_name))
        .arg(&software_hive)
        .output();

    // Disable WER dialogs
    let _ = Command::new("reg")
        .arg("add")
        .arg(format!("HKLM\\{}\\Microsoft\\Windows\\Windows Error Reporting", hive_name))
        .arg("/v")
        .arg("DontShowUI")
        .arg("/t")
        .arg("REG_DWORD")
        .arg("/d")
        .arg("1")
        .arg("/f")
        .output();

    // Disable Dr. Watson
    let _ = Command::new("reg")
        .arg("add")
        .arg(format!("HKLM\\{}\\Microsoft\\Windows NT\\CurrentVersion\\AeDebug", hive_name))
        .arg("/v")
        .arg("Auto")
        .arg("/t")
        .arg("REG_SZ")
        .arg("/d")
        .arg("0")
        .arg("/f")
        .output();

    // Unload the hive
    let _ = Command::new("reg")
        .arg("unload")
        .arg(format!("HKLM\\{}", hive_name))
        .output();

    println!("  Crash dialogs disabled");
    FixResult {
        fix_id: "disable_crash_dialogs".to_string(),
        fix_name: "Disable Crash Dialogs".to_string(),
        success: true,
        message: "WER and crash dialogs disabled".to_string(),
    }
}

/// Enable long path support
fn apply_long_paths_fix(mount_path: &Path) -> FixResult {
    println!("Applying long paths fix...");

    let system_hive = mount_path
        .join("Windows")
        .join("System32")
        .join("config")
        .join("SYSTEM");

    if !system_hive.exists() {
        return FixResult {
            fix_id: "enable_long_paths".to_string(),
            fix_name: "Enable Long Paths".to_string(),
            success: false,
            message: "SYSTEM hive not found".to_string(),
        };
    }

    let hive_name = "_WinPE_LongPath_Fix";

    // Load the hive
    let _ = Command::new("reg")
        .arg("load")
        .arg(format!("HKLM\\{}", hive_name))
        .arg(&system_hive)
        .output();

    // Enable long paths
    let result = Command::new("reg")
        .arg("add")
        .arg(format!("HKLM\\{}\\ControlSet001\\Control\\FileSystem", hive_name))
        .arg("/v")
        .arg("LongPathsEnabled")
        .arg("/t")
        .arg("REG_DWORD")
        .arg("/d")
        .arg("1")
        .arg("/f")
        .output();

    // Unload the hive
    let _ = Command::new("reg")
        .arg("unload")
        .arg(format!("HKLM\\{}", hive_name))
        .output();

    match result {
        Ok(out) if out.status.success() => {
            println!("  Long paths enabled");
            FixResult {
                fix_id: "enable_long_paths".to_string(),
                fix_name: "Enable Long Paths".to_string(),
                success: true,
                message: "Long path support enabled".to_string(),
            }
        }
        _ => FixResult {
            fix_id: "enable_long_paths".to_string(),
            fix_name: "Enable Long Paths".to_string(),
            success: false,
            message: "Failed to modify registry".to_string(),
        },
    }
}

// ============================================
// TESTS
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_fixes() {
        let fixes = get_all_fixes();
        // 5 fixes: dpi_scaling, wallpaper_host, font_fix, crash_dialogs, long_paths
        assert_eq!(fixes.len(), 5);

        // Check required fixes exist
        let dpi = fixes.iter().find(|f| f.id == "dpi_scaling");
        assert!(dpi.is_some());
        assert!(dpi.unwrap().default_enabled);
    }

    #[test]
    fn test_default_fixes() {
        let defaults = get_default_enabled_fixes();

        // All 5 fixes should be enabled by default
        assert_eq!(defaults.len(), 5);
        assert!(defaults.contains(&"dpi_scaling".to_string()));
        assert!(defaults.contains(&"wallpaper_host".to_string()));
        assert!(defaults.contains(&"disable_crash_dialogs".to_string()));
    }
}
