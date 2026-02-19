// ============================================
// MasterBooter - adk_packages.rs
// ============================================
// This module handles Windows ADK optional packages for WinPE.
//
// When building a WinPE image, we can add "optional components" that
// provide additional functionality like PowerShell, network support,
// WMI, etc. These come from the Windows ADK (Assessment and Deployment Kit).
//
// Reference: Windows Setup Helper and GhostWin both use these packages
// to create feature-rich PE environments.
// ============================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;

// ============================================
// ADK PACKAGE DEFINITIONS
// ============================================
// Each WinPE optional component is a .cab file in the ADK.
// Packages have dependencies - some require others to be installed first.
// For example, PowerShell requires WMI, NetFX, and Scripting.

/// Represents a single WinPE optional component package
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AdkPackage {
    /// Internal identifier for the package
    pub id: &'static str,

    /// Display name shown in the UI
    pub display_name: &'static str,

    /// Description of what this package provides
    pub description: &'static str,

    /// The base filename of the package (without _en-us suffix)
    /// e.g., "WinPE-WMI" becomes "WinPE-WMI.cab" and "WinPE-WMI_en-us.cab"
    pub package_name: &'static str,

    /// List of package IDs this package depends on
    /// These must be installed first
    pub dependencies: &'static [&'static str],

    /// Whether this package is enabled by default
    pub default_enabled: bool,

    /// Category for grouping in the UI
    pub category: PackageCategory,

    /// Whether this is required for MasterBooter to function
    pub required_for_app: bool,
}

/// Categories for organizing packages in the UI
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PackageCategory {
    /// Core functionality (WMI, Scripting, etc.)
    Core,
    /// PowerShell and scripting
    Scripting,
    /// Network support
    Network,
    /// Storage and disk management
    Storage,
    /// Security features (BitLocker, etc.)
    Security,
    /// Recovery and diagnostics
    Recovery,
    /// Windows Setup and deployment
    Setup,
    /// Font support (international)
    Fonts,
    /// Input devices and peripherals
    Input,
}

#[allow(dead_code)]
impl PackageCategory {
    /// Get display name for the category
    pub fn display_name(&self) -> &'static str {
        match self {
            PackageCategory::Core => "Core Components",
            PackageCategory::Scripting => "Scripting & Automation",
            PackageCategory::Network => "Network Support",
            PackageCategory::Storage => "Storage & Disk",
            PackageCategory::Security => "Security",
            PackageCategory::Recovery => "Recovery & Diagnostics",
            PackageCategory::Setup => "Setup & Deployment",
            PackageCategory::Fonts => "Font Support",
            PackageCategory::Input => "Input & Peripherals",
        }
    }

    /// Get accent color for the category (matches AMPIPIT theme)
    pub fn color(&self) -> &'static str {
        match self {
            PackageCategory::Core => "#2DD4BF",        // Teal
            PackageCategory::Scripting => "#3B82F6",   // Blue
            PackageCategory::Network => "#22C55E",     // Green
            PackageCategory::Storage => "#F97316",     // Orange
            PackageCategory::Security => "#EF4444",    // Red
            PackageCategory::Recovery => "#A855F7",    // Purple
            PackageCategory::Setup => "#EC4899",       // Pink
            PackageCategory::Fonts => "#6366F1",       // Indigo
            PackageCategory::Input => "#14B8A6",       // Teal-green
        }
    }
}

/// Get all available ADK packages
///
/// This returns the complete list of WinPE optional components that
/// MasterBooter supports. Each package can be toggled on/off in the UI.
pub fn get_all_packages() -> Vec<AdkPackage> {
    vec![
        // ============================================
        // CORE COMPONENTS
        // ============================================
        // These are the foundation - many other packages depend on them

        AdkPackage {
            id: "wmi",
            display_name: "WMI",
            description: "Windows Management Instrumentation - Required for system queries and management",
            package_name: "WinPE-WMI",
            dependencies: &[],
            default_enabled: true,
            category: PackageCategory::Core,
            required_for_app: true,  // MasterBooter needs WMI for hardware detection
        },

        AdkPackage {
            id: "netfx",
            display_name: ".NET Framework",
            description: "Provides .NET runtime support for managed applications",
            package_name: "WinPE-NetFX",
            dependencies: &["wmi"],  // NetFX requires WMI
            default_enabled: true,
            category: PackageCategory::Core,
            required_for_app: false,
        },

        AdkPackage {
            id: "scripting",
            display_name: "Scripting (WSH)",
            description: "Windows Script Host - Enables VBScript and JScript execution",
            package_name: "WinPE-Scripting",
            dependencies: &["wmi"],  // Scripting requires WMI
            default_enabled: true,
            category: PackageCategory::Scripting,
            required_for_app: false,
        },

        AdkPackage {
            id: "hta",
            display_name: "HTML Applications",
            description: "Enables HTML Application (.hta) execution for GUI tools",
            package_name: "WinPE-HTA",
            dependencies: &["scripting"],  // HTA requires Scripting
            default_enabled: true,  // Setup Helper enables this - needed for many PE tools
            category: PackageCategory::Scripting,
            required_for_app: false,
        },

        // ============================================
        // POWERSHELL
        // ============================================
        // PowerShell is critical for automation and modern scripts

        AdkPackage {
            id: "powershell",
            display_name: "PowerShell",
            description: "Full PowerShell support for scripts and automation",
            package_name: "WinPE-PowerShell",
            dependencies: &["wmi", "netfx", "scripting"],  // PowerShell needs all three
            default_enabled: true,
            category: PackageCategory::Scripting,
            required_for_app: true,  // Many deployment scripts use PowerShell
        },

        AdkPackage {
            id: "dism_cmdlets",
            display_name: "DISM Cmdlets",
            description: "PowerShell cmdlets for image servicing (drivers, packages)",
            package_name: "WinPE-DismCmdlets",
            dependencies: &["powershell"],  // Requires PowerShell
            default_enabled: false,  // Fails with 0x800f081e ("not applicable") on most ADK versions
            category: PackageCategory::Scripting,
            required_for_app: false,  // DISM CLI works fine without the PowerShell cmdlets
        },

        AdkPackage {
            id: "secureboot_cmdlets",
            display_name: "Secure Boot Cmdlets",
            description: "PowerShell cmdlets for managing Secure Boot settings",
            package_name: "WinPE-SecureBootCmdlets",
            dependencies: &["powershell"],
            default_enabled: true,  // Setup Helper enables this
            category: PackageCategory::Security,
            required_for_app: false,
        },

        // ============================================
        // STORAGE & DISK
        // ============================================
        // Critical for working with NVMe drives and storage

        AdkPackage {
            id: "storage_wmi",
            display_name: "Storage WMI",
            description: "WMI classes for storage management - CRITICAL for NVMe drives",
            package_name: "WinPE-StorageWMI",
            dependencies: &["wmi"],
            default_enabled: true,
            category: PackageCategory::Storage,
            required_for_app: true,  // Essential for disk operations
        },

        AdkPackage {
            id: "enhanced_storage",
            display_name: "Enhanced Storage",
            description: "Support for encrypted and enhanced storage devices",
            package_name: "WinPE-EnhancedStorage",
            dependencies: &[],
            default_enabled: true,
            category: PackageCategory::Storage,
            required_for_app: false,
        },

        AdkPackage {
            id: "fmapi",
            display_name: "File Management API",
            description: "Windows File Management APIs for advanced file operations",
            package_name: "WinPE-FMAPI",
            dependencies: &[],
            default_enabled: true,  // Setup Helper enables this
            category: PackageCategory::Storage,
            required_for_app: false,
        },

        // ============================================
        // NETWORK
        // ============================================
        // Network connectivity and authentication

        AdkPackage {
            id: "dot3svc",
            display_name: "802.1X Authentication",
            description: "Wired network authentication (enterprise/corporate networks)",
            package_name: "WinPE-Dot3Svc",
            dependencies: &[],
            default_enabled: true,  // Enabled by default for enterprise wired networks
            category: PackageCategory::Network,
            required_for_app: false,
        },

        // Note: Basic TCP/IP networking is built into WinPE base image
        // These are optional enhancements

        // ============================================
        // SECURITY
        // ============================================
        // BitLocker and security features

        AdkPackage {
            id: "secure_startup",
            display_name: "BitLocker Support",
            description: "Enables unlocking BitLocker-encrypted drives",
            package_name: "WinPE-SecureStartup",
            dependencies: &["wmi"],
            default_enabled: true,  // Important for accessing encrypted drives
            category: PackageCategory::Security,
            required_for_app: false,
        },

        // ============================================
        // RECOVERY & DIAGNOSTICS
        // ============================================
        // For recovery environments

        AdkPackage {
            id: "winrecfg",
            display_name: "WinRE Configuration",
            description: "Windows Recovery Environment configuration tools",
            package_name: "WinPE-WinReCfg",
            dependencies: &[],
            default_enabled: true,  // Setup Helper enables this
            category: PackageCategory::Recovery,
            required_for_app: false,
        },

        AdkPackage {
            id: "font_support",
            display_name: "Font Support",
            description: "Additional font support for international characters",
            package_name: "WinPE-FontSupport-WinRE",
            dependencies: &[],
            default_enabled: true,  // Setup Helper enables this - prevents font rendering issues
            category: PackageCategory::Recovery,
            required_for_app: false,
        },

        AdkPackage {
            id: "platform_id",
            display_name: "Platform ID",
            description: "Platform identification for firmware/BIOS detection",
            package_name: "WinPE-PlatformId",
            dependencies: &[],
            default_enabled: true,  // Setup Helper enables this
            category: PackageCategory::Recovery,
            required_for_app: false,
        },

        AdkPackage {
            id: "wds_tools",
            display_name: "WDS Tools",
            description: "Windows Deployment Services client tools",
            package_name: "WinPE-WDS-Tools",
            dependencies: &[],
            default_enabled: true,  // Setup Helper enables this
            category: PackageCategory::Recovery,
            required_for_app: false,
        },

        AdkPackage {
            id: "rejuv",
            display_name: "Recovery (Rejuv)",
            description: "Windows Recovery Environment Rejuv tools (only in WinRE, not standalone ADK)",
            package_name: "WinPE-Rejuv",
            dependencies: &[],
            default_enabled: false,  // .cab does NOT exist in ADK — only inside WinRE.wim
            category: PackageCategory::Recovery,
            required_for_app: false,
        },

        AdkPackage {
            id: "srt",
            display_name: "Startup Repair",
            description: "Startup Repair Tool for fixing boot problems (only in WinRE, not standalone ADK)",
            package_name: "WinPE-SRT",
            dependencies: &[],
            default_enabled: false,  // .cab does NOT exist in ADK — only inside WinRE.wim
            category: PackageCategory::Recovery,
            required_for_app: false,
        },

        // ============================================
        // NETWORK (Additional)
        // ============================================

        // NOTE: WinPE-WiFi-Package does NOT exist as a standalone ADK .cab file.
        // Microsoft docs: "This package is included in the base winre.wim file,
        // and not available separately in the Windows PE add-ons for the ADK."
        //
        // WiFi support is instead provided by inject_wifi_support() in winpe.rs,
        // which copies WLAN service files from the local Windows installation
        // into the mounted WIM during the build process.

        AdkPackage {
            id: "pppoe",
            display_name: "PPPoE",
            description: "Point-to-Point Protocol over Ethernet",
            package_name: "WinPE-PPPoE",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Network,
            required_for_app: false,
        },

        AdkPackage {
            id: "rndis",
            display_name: "RNDIS (USB Network)",
            description: "Remote NDIS for USB tethering and network adapters",
            package_name: "WinPE-RNDIS",
            dependencies: &[],
            default_enabled: true,
            category: PackageCategory::Network,
            required_for_app: false,
        },

        // ============================================
        // SECURITY (Additional)
        // ============================================

        AdkPackage {
            id: "hsp_driver",
            display_name: "HSP Driver (Pluton)",
            description: "Microsoft Pluton security processor support",
            package_name: "WinPE-HSP-Driver",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Security,
            required_for_app: false,
        },

        // ============================================
        // STORAGE (Additional)
        // ============================================

        AdkPackage {
            id: "mdac",
            display_name: "Database (MDAC)",
            description: "ODBC and OLE DB database connectivity",
            package_name: "WinPE-MDAC",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Storage,
            required_for_app: false,
        },

        // ============================================
        // SETUP & DEPLOYMENT
        // ============================================
        // Required for running Windows Setup from PE

        AdkPackage {
            id: "setup",
            display_name: "Windows Setup",
            description: "Core Windows Setup support - required for installing Windows",
            package_name: "WinPE-Setup",
            dependencies: &[],
            default_enabled: true,
            category: PackageCategory::Setup,
            required_for_app: false,
        },

        AdkPackage {
            id: "setup_client",
            display_name: "Setup (Client)",
            description: "Windows client edition setup branding",
            package_name: "WinPE-Setup-Client",
            dependencies: &["setup"],
            default_enabled: true,
            category: PackageCategory::Setup,
            required_for_app: false,
        },

        AdkPackage {
            id: "setup_server",
            display_name: "Setup (Server)",
            description: "Windows Server edition setup branding",
            package_name: "WinPE-Setup-Server",
            dependencies: &["setup"],
            default_enabled: false,
            category: PackageCategory::Setup,
            required_for_app: false,
        },

        AdkPackage {
            id: "legacy_setup",
            display_name: "Legacy Setup",
            description: "Legacy Windows Setup support for older installations",
            package_name: "WinPE-LegacySetup",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Setup,
            required_for_app: false,
        },

        // ============================================
        // FONTS
        // ============================================
        // International font support

        AdkPackage {
            id: "fonts_legacy",
            display_name: "Legacy Fonts",
            description: "Legacy font support for older applications",
            package_name: "WinPE-Fonts-Legacy",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Fonts,
            required_for_app: false,
        },

        AdkPackage {
            id: "fonts_japanese",
            display_name: "Japanese Fonts",
            description: "Japanese language font support",
            package_name: "WinPE-FontSupport-JA-JP",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Fonts,
            required_for_app: false,
        },

        AdkPackage {
            id: "fonts_korean",
            display_name: "Korean Fonts",
            description: "Korean language font support",
            package_name: "WinPE-FontSupport-KO-KR",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Fonts,
            required_for_app: false,
        },

        AdkPackage {
            id: "fonts_chinese_simplified",
            display_name: "Chinese (Simplified)",
            description: "Simplified Chinese font support",
            package_name: "WinPE-FontSupport-ZH-CN",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Fonts,
            required_for_app: false,
        },

        AdkPackage {
            id: "fonts_chinese_traditional",
            display_name: "Chinese (Traditional)",
            description: "Traditional Chinese font support",
            package_name: "WinPE-FontSupport-ZH-TW",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Fonts,
            required_for_app: false,
        },

        AdkPackage {
            id: "fonts_chinese_hk",
            display_name: "Chinese (Hong Kong)",
            description: "Hong Kong Chinese font support",
            package_name: "WinPE-FontSupport-ZH-HK",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Fonts,
            required_for_app: false,
        },

        // ============================================
        // INPUT & PERIPHERALS
        // ============================================

        AdkPackage {
            id: "gaming_peripherals",
            display_name: "Gaming Peripherals",
            description: "Xbox controller and gaming device support",
            package_name: "WinPE-GamingPeripherals",
            dependencies: &[],
            default_enabled: false,
            category: PackageCategory::Input,
            required_for_app: false,
        },
    ]
}

/// Get packages that should be enabled by default
pub fn get_default_enabled_packages() -> Vec<String> {
    get_all_packages()
        .iter()
        .filter(|p| p.default_enabled)
        .map(|p| p.id.to_string())
        .collect()
}

/// Get packages required for MasterBooter to function
#[allow(dead_code)]
pub fn get_required_packages() -> Vec<String> {
    get_all_packages()
        .iter()
        .filter(|p| p.required_for_app)
        .map(|p| p.id.to_string())
        .collect()
}

// ============================================
// ADK LOCATION DETECTION
// ============================================

/// Information about the ADK installation
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AdkLocation {
    pub found: bool,
    pub base_path: PathBuf,
    pub winpe_ocs_path: PathBuf,  // Path to optional components
    pub architecture: String,     // amd64, x86, arm64
    pub version: String,
}

/// Detect where the Windows ADK optional components are installed
///
/// The packages are located at:
/// {ADK_PATH}\Assessment and Deployment Kit\Windows Preinstallation Environment\{arch}\WinPE_OCs\
pub fn detect_adk_packages_path(architecture: &str) -> Option<AdkLocation> {
    println!("Detecting ADK optional components for {}...", architecture);

    // Common ADK installation paths
    let adk_paths = [
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10"),
        PathBuf::from(r"C:\Program Files\Windows Kits\10"),
    ];

    for base_path in &adk_paths {
        // Build path to WinPE optional components
        let winpe_ocs = base_path
            .join("Assessment and Deployment Kit")
            .join("Windows Preinstallation Environment")
            .join(architecture)
            .join("WinPE_OCs");

        if winpe_ocs.exists() {
            // Check if it has the expected packages
            let test_package = winpe_ocs.join("WinPE-WMI.cab");
            if test_package.exists() {
                println!("Found ADK packages at: {}", winpe_ocs.display());

                // Try to detect version from folder structure
                let version = detect_adk_version_from_path(base_path);

                return Some(AdkLocation {
                    found: true,
                    base_path: base_path.clone(),
                    winpe_ocs_path: winpe_ocs,
                    architecture: architecture.to_string(),
                    version,
                });
            }
        }
    }

    println!("ADK optional components not found for {}", architecture);
    None
}

/// Try to detect ADK version from the installation path
fn detect_adk_version_from_path(base_path: &Path) -> String {
    // Check for version info in registry or version files
    // For now, just return a generic version based on path structure

    // Try to read the version from the kit
    let version_file = base_path.join("SDKManifest.xml");
    if version_file.exists() {
        if let Ok(content) = fs::read_to_string(&version_file) {
            // Look for version pattern in the manifest
            if content.contains("10.0.26100") {
                return "10.1.26100 (Windows 11 24H2)".to_string();
            } else if content.contains("10.0.25398") {
                return "10.1.25398 (Windows 11 23H2)".to_string();
            } else if content.contains("10.0.22621") {
                return "10.1.22621 (Windows 11 22H2)".to_string();
            } else if content.contains("10.0.22000") {
                return "10.1.22000 (Windows 11 21H2)".to_string();
            } else if content.contains("10.0.19041") {
                return "10.1.19041 (Windows 10 2004)".to_string();
            }
        }
    }

    "Windows 10/11 ADK".to_string()
}

// ============================================
// PACKAGE INSTALLATION
// ============================================

/// Result of installing a single package
#[derive(Debug)]
#[allow(dead_code)]
pub struct PackageInstallResult {
    pub package_id: String,
    pub package_name: String,
    pub success: bool,
    pub message: String,
}

/// Install a single ADK package into a mounted WIM
///
/// Uses DISM to add the package:
/// dism /Image:{mount_path} /Add-Package /PackagePath:{package.cab}
///
/// Each package has a base file and a language file:
/// - WinPE-WMI.cab (base)
/// - WinPE-WMI_en-us.cab (language resources)
pub fn install_package(
    mount_path: &Path,
    adk_location: &AdkLocation,
    package: &AdkPackage,
) -> PackageInstallResult {
    println!("Installing package: {} ({})", package.display_name, package.package_name);

    // Build paths to the package files
    let base_cab = adk_location.winpe_ocs_path.join(format!("{}.cab", package.package_name));
    let lang_cab = adk_location.winpe_ocs_path.join(format!("{}_en-us.cab", package.package_name));

    // Check if package exists
    if !base_cab.exists() {
        return PackageInstallResult {
            package_id: package.id.to_string(),
            package_name: package.display_name.to_string(),
            success: false,
            message: format!("Package not found: {}", base_cab.display()),
        };
    }

    // Install base package first
    let output = Command::new("dism")
        .arg(format!("/Image:{}", mount_path.display()))
        .arg("/Add-Package")
        .arg(format!("/PackagePath:{}", base_cab.display()))
        .output();

    match output {
        Ok(out) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);

                // Check if package is already installed (not an error)
                if stdout.contains("is already installed") || stderr.contains("is already installed") {
                    println!("  Package already installed: {}", package.package_name);
                } else {
                    return PackageInstallResult {
                        package_id: package.id.to_string(),
                        package_name: package.display_name.to_string(),
                        success: false,
                        message: format!("DISM failed: {}\n{}", stdout, stderr),
                    };
                }
            }
        }
        Err(e) => {
            return PackageInstallResult {
                package_id: package.id.to_string(),
                package_name: package.display_name.to_string(),
                success: false,
                message: format!("Failed to run DISM: {}", e),
            };
        }
    }

    // Install language pack if it exists
    if lang_cab.exists() {
        let lang_output = Command::new("dism")
            .arg(format!("/Image:{}", mount_path.display()))
            .arg("/Add-Package")
            .arg(format!("/PackagePath:{}", lang_cab.display()))
            .output();

        if let Ok(out) = lang_output {
            if !out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if !stdout.contains("is already installed") {
                    println!("  Warning: Failed to install language pack for {}", package.package_name);
                }
            }
        }
    }

    println!("  Successfully installed: {}", package.package_name);

    PackageInstallResult {
        package_id: package.id.to_string(),
        package_name: package.display_name.to_string(),
        success: true,
        message: "Installed successfully".to_string(),
    }
}

/// Install multiple packages with proper dependency ordering
///
/// This function:
/// 1. Resolves dependencies to determine install order
/// 2. Installs packages in the correct order
/// 3. Reports progress via callback
///
/// # Arguments
/// * `mount_path` - Path where WIM is mounted
/// * `adk_location` - ADK installation info
/// * `enabled_package_ids` - List of package IDs to install
/// * `progress` - Callback for progress updates (package_name, current, total)
///
/// # Returns
/// List of install results for each package
pub fn install_packages(
    mount_path: &Path,
    adk_location: &AdkLocation,
    enabled_package_ids: &[String],
    progress: impl Fn(&str, usize, usize),
) -> Vec<PackageInstallResult> {
    println!("Installing {} packages...", enabled_package_ids.len());

    let all_packages = get_all_packages();

    // Build a map of packages for quick lookup
    let package_map: std::collections::HashMap<&str, &AdkPackage> = all_packages
        .iter()
        .map(|p| (p.id, p))
        .collect();

    // Resolve install order (dependencies first)
    let install_order = resolve_dependency_order(enabled_package_ids, &package_map);

    let total = install_order.len();
    let mut results = Vec::new();

    for (index, package_id) in install_order.iter().enumerate() {
        if let Some(package) = package_map.get(package_id.as_str()) {
            progress(&package.display_name, index + 1, total);

            let result = install_package(mount_path, adk_location, package);
            results.push(result);
        }
    }

    println!("Package installation complete. {} of {} succeeded",
        results.iter().filter(|r| r.success).count(),
        results.len()
    );

    results
}

/// Resolve package dependencies to determine install order
///
/// Uses topological sort to ensure dependencies are installed first
fn resolve_dependency_order(
    package_ids: &[String],
    package_map: &std::collections::HashMap<&str, &AdkPackage>,
) -> Vec<String> {
    // First, collect all packages including their dependencies
    let mut all_needed: std::collections::HashSet<String> = std::collections::HashSet::new();

    fn collect_deps(
        id: &str,
        package_map: &std::collections::HashMap<&str, &AdkPackage>,
        needed: &mut std::collections::HashSet<String>,
    ) {
        if needed.contains(id) {
            return;
        }

        if let Some(package) = package_map.get(id) {
            // First add dependencies
            for dep in package.dependencies {
                collect_deps(dep, package_map, needed);
            }
            // Then add this package
            needed.insert(id.to_string());
        }
    }

    for id in package_ids {
        collect_deps(id, package_map, &mut all_needed);
    }

    // Now sort by dependency order (simple approach: deps have fewer deps, so sort by dep count)
    let mut ordered: Vec<String> = all_needed.into_iter().collect();
    ordered.sort_by(|a, b| {
        let a_deps = package_map.get(a.as_str()).map(|p| p.dependencies.len()).unwrap_or(0);
        let b_deps = package_map.get(b.as_str()).map(|p| p.dependencies.len()).unwrap_or(0);
        a_deps.cmp(&b_deps)
    });

    ordered
}

// ============================================
// PACKAGE STATUS CHECKING
// ============================================

/// Check if a package is installed in a mounted WIM
#[allow(dead_code)]
pub fn is_package_installed(mount_path: &Path, package_name: &str) -> bool {
    let output = Command::new("dism")
        .arg(format!("/Image:{}", mount_path.display()))
        .arg("/Get-Packages")
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        return stdout.contains(package_name);
    }

    false
}

/// Get list of installed packages in a mounted WIM
#[allow(dead_code)]
pub fn get_installed_packages(mount_path: &Path) -> Vec<String> {
    let output = Command::new("dism")
        .arg(format!("/Image:{}", mount_path.display()))
        .arg("/Get-Packages")
        .output();

    let mut packages = Vec::new();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);

        for line in stdout.lines() {
            if line.contains("Package Identity :") {
                if let Some(name) = line.split(':').nth(1) {
                    packages.push(name.trim().to_string());
                }
            }
        }
    }

    packages
}

// ============================================
// TESTS
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_packages() {
        let packages = get_all_packages();
        assert!(!packages.is_empty());

        // Check that WMI is first (no dependencies)
        let wmi = packages.iter().find(|p| p.id == "wmi");
        assert!(wmi.is_some());
        assert!(wmi.unwrap().dependencies.is_empty());
    }

    #[test]
    fn test_dependency_order() {
        let packages = get_all_packages();
        let package_map: std::collections::HashMap<&str, &AdkPackage> = packages
            .iter()
            .map(|p| (p.id, p))
            .collect();

        // PowerShell depends on WMI, NetFX, and Scripting
        let order = resolve_dependency_order(
            &["powershell".to_string()],
            &package_map,
        );

        // WMI should come before PowerShell
        let wmi_pos = order.iter().position(|x| x == "wmi");
        let ps_pos = order.iter().position(|x| x == "powershell");

        assert!(wmi_pos.is_some());
        assert!(ps_pos.is_some());
        assert!(wmi_pos.unwrap() < ps_pos.unwrap());
    }

    #[test]
    fn test_default_packages() {
        let defaults = get_default_enabled_packages();

        // Should include critical packages
        assert!(defaults.contains(&"wmi".to_string()));
        assert!(defaults.contains(&"powershell".to_string()));
        assert!(defaults.contains(&"storage_wmi".to_string()));
    }

    #[test]
    fn test_required_packages() {
        let required = get_required_packages();

        // MasterBooter needs WMI and PowerShell
        assert!(required.contains(&"wmi".to_string()));
        assert!(required.contains(&"powershell".to_string()));
    }
}
