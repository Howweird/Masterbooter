// ============================================
// tools.rs - Manages bundled third-party tools
// ============================================
//
// This module handles:
//   - Tool definitions (name, URL, download type)
//   - Checking if tools are installed
//   - Downloading tools from official sources
//   - Launching tools
//
// PORTABLE DESIGN:
// Tools are stored in subfolders NEXT TO the MasterBooter.exe:
//
//   USB Drive/
//   ├── masterbooter.exe
//   ├── backup_tools/           # Tools for Backup/Restore (live Windows)
//   │   ├── fabs/
//   │   │   └── AutoBackup7Pro.exe
//   │   ├── profwiz/
//   │   │   └── Profwiz.exe
//   │   └── disk2vhd/
//   │       └── disk2vhd64.exe
//   └── pe_tools/               # Tools bundled INTO WinPE images
//       ├── shell/
//       │   └── winxshell/
//       ├── network/
//       │   └── penetwork/
//       └── disk/
//           └── crystaldiskinfo/
// ============================================

use std::fs::{self, File};
use std::io::{self, Write, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Result, Context};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ============================================
// DOWNLOAD TYPES
// ============================================

/// How a tool is downloaded and processed
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DownloadType {
    /// Direct EXE - just save the file
    DirectExe,
    /// ZIP archive - extract EXE files only
    Zip,
    /// MSI installer - extract using msiexec
    Msi,
    /// Self-extracting EXE (like Inno Setup)
    SelfExtractingExe,
}

// ============================================
// TOOL DEFINITION
// ============================================

/// Information about a bundled tool
#[derive(Debug, Clone)]
pub struct BundledTool {
    /// Unique ID (e.g., "fabs", "profwiz")
    pub id: &'static str,
    /// Display name for UI
    pub display_name: &'static str,
    /// Executable filename
    pub executable_name: &'static str,
    /// Description for UI
    pub description: &'static str,
    /// Download URL (official source)
    pub download_url: &'static str,
    /// How to process the download
    pub download_type: DownloadType,
}

// ============================================
// TOOL DEFINITIONS
// ============================================
// All tools download from their official sources.
// This ensures users get legitimate, up-to-date versions.

/// Fab's AutoBackup 7 Pro - User profile backup
pub const FABS_AUTOBACKUP: BundledTool = BundledTool {
    id: "fabs",
    display_name: "Fab's AutoBackup 7 Pro",
    executable_name: "AutoBackup7Pro.exe",
    description: "Professional user profile backup and restore tool. Activate with your own license.",
    download_url: "https://download.fpnet.fr/trial/AutoBackup7Pro.exe",
    download_type: DownloadType::SelfExtractingExe,
};

/// ProfWiz - User Profile Wizard (profile migration)
pub const PROFWIZ: BundledTool = BundledTool {
    id: "profwiz",
    display_name: "User Profile Wizard (Personal Edition)",
    executable_name: "Profwiz.exe",
    description: "Migrate user profiles between domains or computers. Free for personal use.",
    download_url: "https://www.forensit.com/Downloads/Profwiz.msi",
    download_type: DownloadType::Msi,
};

/// Transwiz - Profile Transfer
pub const TRANSWIZ: BundledTool = BundledTool {
    id: "transwiz",
    display_name: "Transwiz (Profile Transfer)",
    executable_name: "Transwiz.exe",
    description: "Transfer user profiles to a new computer. Backup profiles to a file and restore on another PC.",
    download_url: "https://www.forensit.com/Downloads/Transwiz.msi",
    download_type: DownloadType::Msi,
};

/// Disk2VHD - Microsoft Sysinternals disk imaging
pub const DISK2VHD: BundledTool = BundledTool {
    id: "disk2vhd",
    display_name: "Disk2VHD",
    executable_name: "disk2vhd64.exe",
    description: "Create VHD/VHDX disk images from physical disks.",
    download_url: "https://download.sysinternals.com/files/Disk2vhd.zip",
    download_type: DownloadType::Zip,
};

/// HDD Raw Copy Tool - Sector-by-sector disk copy
pub const HDD_RAW_COPY: BundledTool = BundledTool {
    id: "hddrawcopy",
    display_name: "HDD Raw Copy Tool",
    executable_name: "HDDRawCopy1.20Portable.exe",
    description: "Sector-by-sector raw disk copy. Creates exact clones including hidden partitions.",
    download_url: "https://hddguru.com/software/HDD-Raw-Copy-Tool/HDDRawCopy1.20Portable.exe",
    download_type: DownloadType::DirectExe,
};

// ============================================
// SYSTEM PREP TOOLS
// ============================================
// Tools for preparing a system for image capture (sysprep).

/// SysprepPreparator — wizard-based tool for preparing Windows for imaging.
/// Runs pre-sysprep checks (pending updates, Store apps, drivers, domain),
/// performs system cleanup, then launches sysprep.exe with configurable options.
/// GitHub: https://github.com/CodingWonders/SysprepPreparator
pub const SYSPREP_PREPARATOR: BundledTool = BundledTool {
    id: "sysprepprep",
    display_name: "Sysprep Preparator",
    executable_name: "SysprepPreparator.exe",
    description: "Wizard-based tool to prepare Windows for imaging. Runs compatibility checks, cleanup, and sysprep.",
    download_url: "https://github.com/CodingWonders/SysprepPreparator/releases/download/DT_25122/SysprepPreparator.zip",
    download_type: DownloadType::Zip,
};

/// Get a tool by its ID.
/// Looks up both backup tools and system prep tools.
pub fn get_tool_by_id(id: &str) -> Option<&'static BundledTool> {
    match id {
        "fabs" => Some(&FABS_AUTOBACKUP),
        "profwiz" => Some(&PROFWIZ),
        "transwiz" => Some(&TRANSWIZ),
        "disk2vhd" => Some(&DISK2VHD),
        "hddrawcopy" => Some(&HDD_RAW_COPY),
        "sysprepprep" => Some(&SYSPREP_PREPARATOR),
        _ => None,
    }
}

/// Get ALL backup tools as a list.
/// Used by the "Download All" feature to iterate every tool.
/// Note: System prep tools are NOT included here — they have their own section.
pub fn get_all_tools() -> Vec<&'static BundledTool> {
    vec![
        &FABS_AUTOBACKUP,
        &PROFWIZ,
        &TRANSWIZ,
        &DISK2VHD,
        &HDD_RAW_COPY,
    ]
}

// ============================================
// PATH HELPERS
// ============================================

/// Get the directory where masterbooter.exe is located.
///
/// Uses std::env::current_exe() to find the EXE's actual location.
/// This ensures tools are always stored NEXT TO the EXE, even when
/// the current working directory is different (e.g., running from
/// a shortcut or a different drive).
pub fn get_app_directory() -> PathBuf {
    if let Ok(exe_path) = std::env::current_exe() {
        // Canonicalize to resolve any symlinks/junctions, then get parent
        let resolved = exe_path.canonicalize().unwrap_or(exe_path);
        if let Some(parent) = resolved.parent() {
            // Strip \\?\ prefix that canonicalize adds on Windows
            let parent_str = parent.to_string_lossy();
            if parent_str.starts_with(r"\\?\") {
                return PathBuf::from(&parent_str[4..]);
            }
            return parent.to_path_buf();
        }
    }
    // Last resort: use current directory (shouldn't normally happen)
    println!("Warning: Could not determine EXE directory, using current directory");
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Get the backup tools directory (next to the EXE).
///
/// Backup tools (Fab's AutoBackup, Disk2VHD, etc.) are stored in
/// backup_tools/ next to the EXE. These run in live Windows for the
/// Backup/Restore page and are also accessible from the USB drive in PE.
pub fn get_backup_tools_path() -> PathBuf {
    get_app_directory().join("backup_tools")
}

/// Alias for backward compatibility - same as get_backup_tools_path()
pub fn get_tools_base_path() -> PathBuf {
    get_backup_tools_path()
}

/// Get a specific tool's folder path
pub fn get_tool_path(tool: &BundledTool) -> PathBuf {
    get_tools_base_path().join(tool.id)
}

/// Get the full path to a tool's executable
pub fn get_executable_path(tool: &BundledTool) -> PathBuf {
    get_tool_path(tool).join(tool.executable_name)
}

// ============================================
// TOOL STATUS
// ============================================

/// Check if a tool is installed (EXE exists)
pub fn is_tool_installed(tool: &BundledTool) -> bool {
    get_executable_path(tool).exists()
}

/// Get installed version (from version.txt or file info)
pub fn get_installed_version(tool: &BundledTool) -> Option<String> {
    let version_file = get_tool_path(tool).join("version.txt");

    if version_file.exists() {
        if let Ok(version) = fs::read_to_string(&version_file) {
            return Some(version.trim().to_string());
        }
    }

    // If tool is installed but no version file, return "Installed"
    if is_tool_installed(tool) {
        return Some("Installed".to_string());
    }

    None
}

// ============================================
// TOOL LAUNCHING
// ============================================

/// Launch a tool
pub fn launch_tool(tool: &BundledTool) -> Result<()> {
    let exe_path = get_executable_path(tool);

    if !exe_path.exists() {
        anyhow::bail!("Tool not installed: {}", tool.display_name);
    }

    // Launch the tool
    #[cfg(windows)]
    {
        Command::new(&exe_path)
            .current_dir(get_tool_path(tool))
            .spawn()
            .context(format!("Failed to launch {}", tool.display_name))?;
    }

    #[cfg(not(windows))]
    {
        anyhow::bail!("Tool launching only supported on Windows");
    }

    Ok(())
}

/// Open the tool's folder in File Explorer
pub fn open_tool_folder(tool: &BundledTool) -> Result<()> {
    let folder_path = get_tool_path(tool);

    // Create folder if it doesn't exist
    fs::create_dir_all(&folder_path)?;

    #[cfg(windows)]
    {
        Command::new("explorer.exe")
            .arg(&folder_path)
            .spawn()
            .context("Failed to open folder")?;
    }

    Ok(())
}

// ============================================
// TOOL DOWNLOADING
// ============================================

/// Download a tool from its official URL
/// Returns Ok(()) on success, Err on failure
pub fn download_tool(tool: &BundledTool, progress_callback: impl Fn(u32)) -> Result<()> {
    let dest_path = get_tool_path(tool);
    println!("App directory: {:?}", get_app_directory());
    println!("Tool destination: {:?}", dest_path);
    fs::create_dir_all(&dest_path)?;

    // Determine temp filename based on download type
    let temp_filename = match tool.download_type {
        DownloadType::DirectExe => "download.exe",
        DownloadType::Zip => "download.zip",
        DownloadType::Msi => "download.msi",
        DownloadType::SelfExtractingExe => "download.exe",
    };
    let temp_path = dest_path.join(temp_filename);

    // Download the file
    println!("Downloading {} from {}...", tool.display_name, tool.download_url);
    progress_callback(0);

    let client = reqwest::blocking::Client::builder()
        .user_agent("MasterBooter/1.0")
        .redirect(reqwest::redirect::Policy::limited(10))  // Follow up to 10 redirects
        .build()?;

    println!("Fetching URL: {}", tool.download_url);

    let response = client
        .get(tool.download_url)
        .send()
        .context("Failed to connect to download server")?;

    println!("Response status: {}", response.status());
    println!("Final URL: {}", response.url());
    println!("Content-Type: {:?}", response.headers().get("content-type"));

    if !response.status().is_success() {
        anyhow::bail!("Download failed with status: {}", response.status());
    }

    let total_size = response.content_length().unwrap_or(0);
    println!("Content-Length: {} bytes", total_size);
    let mut downloaded: u64 = 0;

    // Write to temp file
    let mut file = File::create(&temp_path)?;
    let mut reader = response;
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        file.write_all(&buffer[..bytes_read])?;
        downloaded += bytes_read as u64;

        if total_size > 0 {
            let percent = ((downloaded * 100) / total_size) as u32;
            progress_callback(percent);
        }
    }

    // IMPORTANT: Explicitly flush and close the file before processing
    file.flush()?;
    drop(file);

    progress_callback(100);

    // Verify the downloaded file
    let file_size = fs::metadata(&temp_path)?.len();
    println!("Download complete. File size: {} bytes", file_size);

    // Debug: show first few bytes of the file
    {
        let mut debug_file = File::open(&temp_path)?;
        let mut header = [0u8; 8];
        debug_file.read_exact(&mut header)?;
        println!("File header (first 8 bytes): {:02X?}", header);
        // debug_file is dropped here at end of block
    }

    println!("Processing...");

    // Process based on download type
    let result = match tool.download_type {
        DownloadType::DirectExe => process_direct_exe(&temp_path, &dest_path, tool.executable_name),
        DownloadType::Zip => process_zip_file(&temp_path, &dest_path),
        DownloadType::Msi => process_msi_file(&temp_path, &dest_path),
        DownloadType::SelfExtractingExe => process_self_extracting(&temp_path, &dest_path),
    };

    // Clean up temp file if it still exists
    let _ = fs::remove_file(&temp_path);

    result
}

/// Process a direct EXE download - just rename it
fn process_direct_exe(temp_path: &Path, dest_path: &Path, exe_name: &str) -> Result<()> {
    let dest_exe = dest_path.join(exe_name);

    // Remove old EXE if exists
    let _ = fs::remove_file(&dest_exe);

    // Move temp file to final location
    fs::rename(temp_path, &dest_exe)?;

    println!("Installed to: {:?}", dest_exe);
    Ok(())
}

/// Process a ZIP file - extract only EXE files
fn process_zip_file(zip_path: &Path, dest_path: &Path) -> Result<()> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // First pass: check if the ZIP contains DLLs alongside EXEs.
    // If it does, this is a complete application (like SysprepPreparator)
    // and we need to extract EVERYTHING — not just the EXE.
    let mut has_dll = false;
    let mut has_exe = false;
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let lower = entry.name().to_lowercase();
        if lower.ends_with(".dll") { has_dll = true; }
        if lower.ends_with(".exe") { has_exe = true; }
    }

    // Re-open archive (iterator consumed above)
    let file2 = File::open(zip_path)?;
    let mut archive2 = zip::ZipArchive::new(file2)?;

    let extract_all = has_dll && has_exe; // Complete app — extract everything
    let mut extracted_any = false;

    for i in 0..archive2.len() {
        let mut entry = archive2.by_index(i)?;
        let name = entry.name().to_string();

        // Skip directory entries (they're just markers, not files)
        if entry.is_dir() {
            // Create the directory in dest_path
            let dir_path = dest_path.join(&name);
            let _ = fs::create_dir_all(&dir_path);
            continue;
        }

        // Decide whether to extract this file
        let should_extract = if extract_all {
            // Complete app: extract everything (.exe, .dll, .config, language files, etc.)
            true
        } else {
            // Simple tool: only extract EXE files
            name.to_lowercase().ends_with(".exe")
        };

        if should_extract {
            // Build the output path, preserving subfolder structure
            let dest_file = dest_path.join(&name);

            // Create parent directories if needed (e.g., Languages/en.ini)
            if let Some(parent) = dest_file.parent() {
                let _ = fs::create_dir_all(parent);
            }

            // Remove old file if exists
            let _ = fs::remove_file(&dest_file);

            // Extract the file
            let mut outfile = File::create(&dest_file)?;
            io::copy(&mut entry, &mut outfile)?;

            let display_name = Path::new(&name)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(name.clone());
            println!("Extracted: {}", display_name);
            extracted_any = true;
        }
    }

    if extracted_any {
        Ok(())
    } else {
        anyhow::bail!("No extractable files found in ZIP");
    }
}

/// Find 7-Zip executable in common locations
fn find_7zip() -> Option<PathBuf> {
    let paths = [
        PathBuf::from(r"C:\Program Files\7-Zip\7z.exe"),
        PathBuf::from(r"C:\Program Files (x86)\7-Zip\7z.exe"),
        // Check next to our exe (bundled 7z)
        get_app_directory().join("7z.exe"),
        get_tools_base_path().join("7z.exe"),
    ];

    for path in paths {
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Process an MSI file - extract using 7-Zip or fallback to msiexec
fn process_msi_file(msi_path: &Path, dest_path: &Path) -> Result<()> {
    // Create temp extraction folder
    let temp_dir = std::env::temp_dir().join(format!("MasterBooter_MSI_{}", uuid::Uuid::new_v4().simple()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir)?;

    println!("Extracting MSI to temp folder: {:?}", temp_dir);

    // Try 7-Zip first (better at extracting MSI contents)
    let extraction_success = if let Some(seven_zip) = find_7zip() {
        println!("Using 7-Zip: {:?}", seven_zip);

        // Use 7z to extract: 7z x "file.msi" -o"output" -y
        let output = Command::new(&seven_zip)
            .arg("x")
            .arg(msi_path)
            .arg(format!("-o{}", temp_dir.to_string_lossy()))
            .arg("-y")  // Yes to all prompts
            .output();

        if let Ok(out) = output {
            println!("7z exit code: {:?}", out.status.code());
            if out.status.success() {
                // MSI files often contain a nested cab file (disk1.cab) with the actual EXEs
                // We need to extract those too
                for entry in walkdir::WalkDir::new(&temp_dir).max_depth(2) {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        if path.extension().map(|e| e.eq_ignore_ascii_case("cab")).unwrap_or(false) {
                            println!("Found nested cab file: {:?}, extracting...", path);

                            let cab_output = Command::new(&seven_zip)
                                .arg("x")
                                .arg(path)
                                .arg(format!("-o{}", temp_dir.to_string_lossy()))
                                .arg("-y")
                                .output();

                            if let Ok(out) = cab_output {
                                println!("Nested cab extraction exit code: {:?}", out.status.code());
                            }
                        }
                    }
                }
                true
            } else {
                println!("7z stdout: {}", String::from_utf8_lossy(&out.stdout));
                println!("7z stderr: {}", String::from_utf8_lossy(&out.stderr));
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    // Fallback to msiexec if 7-Zip didn't work or isn't available
    if !extraction_success {
        println!("7-Zip not available or failed, using msiexec fallback...");

        // msiexec /a "file.msi" /qn TARGETDIR="output"
        // /a = Administrative install (extracts files)
        // /qn = Quiet, no UI
        let output = Command::new("msiexec")
            .arg("/a")
            .arg(msi_path)
            .arg("/qn")
            .arg(format!("TARGETDIR={}", temp_dir.to_string_lossy()))
            .output()
            .context("Failed to run msiexec")?;

        println!("msiexec exit code: {:?}", output.status.code());

        // msiexec returns 0 on success, but may also extract to a subfolder
        // Give it a moment to finish writing files
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    // Find and copy EXE files from extracted contents
    let mut found_exe = false;
    println!("Searching for EXE files in: {:?}", temp_dir);

    for entry in walkdir::WalkDir::new(&temp_dir) {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false) {
                let filename = entry.file_name().to_string_lossy().to_string();

                // Skip icon/metadata EXEs and system files
                let lowercase = filename.to_lowercase();
                if lowercase.starts_with("icon.")
                    || lowercase.starts_with("!")
                    || lowercase == "msiexec.exe"
                    || lowercase.contains("uninstall") {
                    continue;
                }

                let dest_file = dest_path.join(&filename);

                let _ = fs::remove_file(&dest_file);
                if let Ok(_) = fs::copy(path, &dest_file) {
                    println!("Extracted: {}", filename);
                    found_exe = true;
                }
            }
        }
    }

    // Clean up temp folder
    let _ = fs::remove_dir_all(&temp_dir);

    if found_exe {
        Ok(())
    } else {
        anyhow::bail!("No EXE files found in MSI. Try installing 7-Zip for better extraction.")
    }
}

/// Process a self-extracting EXE (like Inno Setup, NSIS installers)
/// Uses 7-Zip to extract installer contents directly
fn process_self_extracting(exe_path: &Path, dest_path: &Path) -> Result<()> {
    // Try 7-Zip first (can extract most installer formats)
    if let Some(seven_zip) = find_7zip() {
        println!("Extracting installer with 7-Zip...");

        let output = Command::new(&seven_zip)
            .arg("x")
            .arg(exe_path)
            .arg(format!("-o{}", dest_path.to_string_lossy()))
            .arg("-y")
            .output()
            .context("Failed to run 7-Zip")?;

        println!("7z exit code: {:?}", output.status.code());

        // Check if any EXE was extracted (other than the download.exe)
        let has_exe = fs::read_dir(dest_path)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| {
                        let path = e.path();
                        let is_exe = path.extension().map(|ext| ext == "exe").unwrap_or(false);
                        let filename = path.file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        is_exe && !filename.starts_with("download")
                    })
            })
            .unwrap_or(false);

        if has_exe {
            // Clean up the downloaded installer
            let _ = fs::remove_file(exe_path);
            println!("Extraction complete");
            return Ok(());
        }

        println!("7-Zip extraction didn't find EXE files, trying installer...");
    }

    // Fallback: Try running as Inno Setup installer
    println!("Running installer with silent extraction...");

    #[cfg(windows)]
    {
        let args = format!(
            "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR=\"{}\"",
            dest_path.to_string_lossy()
        );

        let status = Command::new(exe_path)
            .raw_arg(&args)
            .status()
            .context("Failed to run installer")?;

        println!("Installer exit code: {:?}", status.code());
    }

    // Give it a moment to write files
    std::thread::sleep(std::time::Duration::from_millis(1000));

    // Check if extraction was successful
    let has_exe = fs::read_dir(dest_path)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .any(|e| {
                    let path = e.path();
                    let is_exe = path.extension().map(|ext| ext == "exe").unwrap_or(false);
                    let filename = path.file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    is_exe && !filename.starts_with("download")
                })
        })
        .unwrap_or(false);

    // Clean up the downloaded installer
    let _ = fs::remove_file(exe_path);

    if has_exe {
        println!("Extraction complete");
        Ok(())
    } else {
        anyhow::bail!(
            "Extraction failed. This tool may require manual installation."
        )
    }
}

// ============================================
// PE TOOLS - For WinPE Builder
// ============================================
// This section handles tools that get bundled INTO the PE image,
// separate from the backup/restore tools above.

#[allow(dead_code)]
pub mod pe_tools {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    // ============================================
    // DATA STRUCTURES
    // ============================================

    /// Represents a tool that can be included in a PE build
    /// This is parsed from a tool.toml file
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PeTool {
        /// Display name of the tool
        pub name: String,

        /// Short description of what the tool does
        pub description: String,

        /// Category: shell, network, disk, security, utilities
        #[serde(default)]
        pub category: String,

        /// Version string
        #[serde(default)]
        pub version: String,

        /// Main executable file (relative to tool folder)
        pub exe: String,

        /// Is this a shell replacement? (only one can be active)
        #[serde(default)]
        pub is_shell: bool,

        /// Create a desktop shortcut in PE?
        #[serde(default = "default_true")]
        pub create_shortcut: bool,

        /// Should this tool be enabled by default?
        #[serde(default)]
        pub enabled_by_default: bool,

        /// Auto-launch at PE startup (before shell)?
        #[serde(default)]
        pub auto_launch: bool,

        /// URL to download the tool if not present (primary source — manufacturer)
        #[serde(default)]
        pub download_url: String,

        /// Fallback URL if primary download fails (GitHub mirror)
        /// This is tried automatically when the primary download_url fails
        #[serde(default)]
        pub fallback_url: String,

        // --- Runtime fields (not from TOML) ---

        /// Full path to the tool folder
        #[serde(skip)]
        pub folder_path: PathBuf,

        /// Is this tool currently enabled for the build?
        #[serde(skip)]
        pub enabled: bool,

        /// Is the tool actually present (exe exists)?
        #[serde(skip)]
        pub is_present: bool,
    }

    /// Helper function for serde default
    fn default_true() -> bool {
        true
    }

    /// Wrapper for the [tool] section in tool.toml
    #[derive(Debug, Deserialize)]
    struct ToolToml {
        tool: PeTool,
    }

    /// Configuration for which tools are enabled
    /// Saved to pe_tools_config.json
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct PeToolsConfig {
        /// Map of tool name -> enabled status
        pub enabled_tools: HashMap<String, bool>,

        /// Custom tools added by user (paths to tool.toml files)
        #[serde(default)]
        pub custom_tools: Vec<PathBuf>,
    }

    // ============================================
    // TOOL DISCOVERY
    // ============================================

    /// Get the path to the PE tools folder
    /// These are tools that get bundled INTO WinPE images (WinPE Builder page)
    pub fn get_pe_tools_folder() -> PathBuf {
        // Use the same robust app directory detection as backup tools
        let app_dir = super::get_app_directory();
        let pe_tools_path = app_dir.join("pe_tools");
        if pe_tools_path.exists() {
            return pe_tools_path;
        }

        // Fall back to current directory (for development)
        let cwd_pe_tools = PathBuf::from("pe_tools");
        if cwd_pe_tools.exists() {
            return cwd_pe_tools;
        }

        // Default: create next to EXE (will be created on first use)
        pe_tools_path
    }

    /// Create the default pe_tools folder structure with tool.toml manifests.
    /// Called automatically when pe_tools/ doesn't exist next to the EXE.
    /// This lets the EXE be truly portable — just drop it on a USB and go.
    fn create_default_pe_tools(tools_folder: &Path) {
        println!("Creating default PE tools folder: {}", tools_folder.display());

        // Each entry: (relative_path, toml_content)
        // These define what tools MasterBooter knows about and where to download them.
        let manifests: &[(&str, &str)] = &[
            ("shell/WinXShell/tool.toml", r#"[tool]
name = "WinXShell"
description = "Desktop shell with taskbar, start menu, and system tray for WinPE"
category = "shell"
version = "2.5"
exe = "WinXShell_x64.exe"
is_shell = true
create_shortcut = false
enabled_by_default = true
auto_launch = false
download_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/WinXShell.zip"
"#),
            ("shell/Explorer++/tool.toml", r#"[tool]
name = "Explorer++"
description = "Lightweight file manager with tabbed browsing"
category = "shell"
version = "1.4.0"
exe = "Explorer++.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://github.com/derceg/explorerplusplus/releases/download/version-1.4.0/explorerpp_x64.zip"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/Explorer++.zip"
"#),
            ("shell/FileExplorer/tool.toml", r#"[tool]
name = "File Explorer"
description = "Lightweight dual-pane file explorer with search for WinPE and live Windows"
category = "shell"
version = "1.0"
exe = "File Explorer (PE).exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://www.pcassistsoftware.co.uk/downloads/File_Explorer_PE.7z"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/FileExplorer.zip"
"#),
            ("network/PENetwork/tool.toml", r#"[tool]
name = "PENetwork"
description = "Network configuration tool for WinPE - IP, DNS, WiFi, shares"
category = "network"
version = "0.59.B12"
exe = "PENetwork.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = true
download_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/PENetwork.zip"
"#),
            ("network/WebBrowser/tool.toml", r#"[tool]
name = "Web Browser"
description = "Compact portable web browser for WinPE and live Windows"
category = "network"
version = "1.0"
exe = "Web Browser (PE).exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://www.pcassistsoftware.co.uk/downloads/Web_Browser_PE.7z"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/WebBrowser.zip"
"#),
            ("disk/CrystalDiskInfo/tool.toml", r#"[tool]
name = "CrystalDiskInfo"
description = "Disk health monitoring and SMART data"
category = "disk"
version = "9.7.2"
exe = "DiskInfo64.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
download_url = "https://downloads.sourceforge.net/project/crystaldiskinfo/9.7.2/CrystalDiskInfo9_7_2.zip"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/CrystalDiskInfo.zip"
"#),
            ("disk/DiskCheck/tool.toml", r#"[tool]
name = "Disk Check"
description = "Monitor hard drive SMART status in WinPE and live Windows"
category = "disk"
version = "1.0"
exe = "Disk Check (PE) x64.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://www.pcassistsoftware.co.uk/downloads/Disk_Check_PE.7z"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/DiskCheck.zip"
"#),
            ("system/DISMTool/tool.toml", r#"[tool]
name = "DISM Tool"
description = "GUI interface for DISM - works on running OS, mounted WIM, or offline OS"
category = "system"
version = "1.0"
exe = "DISM_Tool.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://www.pcassistsoftware.co.uk/downloads/DISM_Tool.7z"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/DISMTool.zip"
"#),
            ("utilities/7-Zip/tool.toml", r#"[tool]
name = "7-Zip"
description = "File archiver with high compression ratio"
category = "utilities"
version = "24.09"
exe = "7zFM.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/7-Zip.zip"
"#),
            ("utilities/Autoruns/tool.toml", r#"[tool]
name = "Autoruns"
description = "Sysinternals tool to view and manage auto-starting programs"
category = "utilities"
version = "14.11"
exe = "Autoruns64.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://download.sysinternals.com/files/Autoruns.zip"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/Autoruns.zip"
"#),
            ("utilities/EventViewer/tool.toml", r#"[tool]
name = "Event Viewer"
description = "Examine Windows event logs from WinPE or live Windows"
category = "utilities"
version = "1.0"
exe = "Event Viewer PE.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://www.pcassistsoftware.co.uk/downloads/Event_Viewer_PE.7z"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/EventViewer.zip"
"#),
            ("utilities/InstalledSoftware/tool.toml", r#"[tool]
name = "Installed Software"
description = "List installed software on host systems from WinPE or live Windows"
category = "utilities"
version = "1.0"
exe = "Installed Software PE.exe"
is_shell = false
create_shortcut = true
enabled_by_default = true
auto_launch = false
download_url = "https://www.pcassistsoftware.co.uk/downloads/Installed_Software_PE.7z"
fallback_url = "https://github.com/Howweird/Masterbooter-Tools/releases/download/v1.0/InstalledSoftware.zip"
"#),
        ];

        // Write each manifest file, creating parent directories as needed
        for (rel_path, content) in manifests {
            let full_path = tools_folder.join(rel_path);
            if let Some(parent) = full_path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    println!("  Warning: Failed to create {}: {}", parent.display(), e);
                    continue;
                }
            }
            if let Err(e) = fs::write(&full_path, content) {
                println!("  Warning: Failed to write {}: {}", full_path.display(), e);
            } else {
                println!("  Created {}", rel_path);
            }
        }

        // Also create the security/ category folder (empty for now)
        let _ = fs::create_dir_all(tools_folder.join("security"));

        println!("Default PE tools created ({} manifests)", manifests.len());
    }

    /// Refresh PE tool manifests from the embedded defaults.
    /// Called when the EXE version changes after an auto-update.
    ///
    /// This overwrites existing tool.toml files with the latest
    /// embedded defaults. Tool binaries (the actual .exe files
    /// that were previously downloaded) are NOT deleted — only
    /// the manifest files are refreshed.
    ///
    /// Why: When MasterBooter updates, the embedded manifests may
    /// contain new download URLs, new tools, or updated versions.
    pub fn refresh_default_manifests() {
        let tools_folder = get_pe_tools_folder();
        println!("Refreshing PE tool manifests in: {}", tools_folder.display());
        create_default_pe_tools(&tools_folder);
        println!("PE tool manifests refreshed successfully.");
    }

    /// Scan the tools folder and discover all available PE tools
    pub fn discover_pe_tools() -> Vec<PeTool> {
        let tools_folder = get_pe_tools_folder();
        let mut tools = Vec::new();

        println!("Scanning for PE tools in: {}", tools_folder.display());

        // If the pe_tools folder doesn't exist, create it with default manifests.
        // This makes the EXE portable — just copy it to a USB and it works.
        if !tools_folder.exists() {
            println!("PE tools folder not found — creating defaults...");
            create_default_pe_tools(&tools_folder);
        }

        // Load saved configuration
        let config = load_pe_tools_config();

        // Scan each category folder
        let categories = ["shell", "network", "disk", "security", "utilities", "system"];

        for category in categories {
            let category_path = tools_folder.join(category);
            if !category_path.exists() {
                continue;
            }

            // Scan each tool folder within the category
            if let Ok(entries) = fs::read_dir(&category_path) {
                for entry in entries.flatten() {
                    let tool_folder = entry.path();
                    if !tool_folder.is_dir() {
                        continue;
                    }

                    // Look for tool.toml
                    let manifest_path = tool_folder.join("tool.toml");
                    if let Some(mut tool) = parse_tool_manifest(&manifest_path) {
                        // Set runtime fields
                        tool.folder_path = tool_folder.clone();
                        tool.category = category.to_string();

                        // Check if exe exists
                        let exe_path = tool_folder.join(&tool.exe);
                        tool.is_present = exe_path.exists();

                        // Set enabled status from config or default
                        tool.enabled = config.enabled_tools
                            .get(&tool.name)
                            .copied()
                            .unwrap_or(tool.enabled_by_default);

                        println!("  Found PE tool: {} ({}) - present: {}, enabled: {}",
                            tool.name, category, tool.is_present, tool.enabled);

                        tools.push(tool);
                    }
                }
            }
        }

        // Also load any custom tools from config
        for custom_path in &config.custom_tools {
            if let Some(mut tool) = parse_tool_manifest(custom_path) {
                tool.folder_path = custom_path.parent().unwrap_or(Path::new(".")).to_path_buf();
                let exe_path = tool.folder_path.join(&tool.exe);
                tool.is_present = exe_path.exists();
                tool.enabled = config.enabled_tools
                    .get(&tool.name)
                    .copied()
                    .unwrap_or(tool.enabled_by_default);

                println!("  Found custom PE tool: {} - present: {}", tool.name, tool.is_present);
                tools.push(tool);
            }
        }

        println!("Total PE tools discovered: {}", tools.len());
        tools
    }

    /// Parse a tool.toml manifest file
    fn parse_tool_manifest(path: &Path) -> Option<PeTool> {
        if !path.exists() {
            return None;
        }

        let content = fs::read_to_string(path).ok()?;
        let toml_data: ToolToml = toml::from_str(&content).ok()?;
        Some(toml_data.tool)
    }

    // ============================================
    // CONFIGURATION MANAGEMENT
    // ============================================

    /// Get the path to the PE tools configuration file
    fn get_config_path() -> PathBuf {
        get_pe_tools_folder().parent()
            .unwrap_or(Path::new("."))
            .join("pe_tools_config.json")
    }

    /// Load the PE tools configuration from disk
    pub fn load_pe_tools_config() -> PeToolsConfig {
        let config_path = get_config_path();

        if config_path.exists() {
            if let Ok(content) = fs::read_to_string(&config_path) {
                if let Ok(config) = serde_json::from_str(&content) {
                    return config;
                }
            }
        }

        PeToolsConfig::default()
    }

    /// Save the PE tools configuration to disk
    pub fn save_pe_tools_config(config: &PeToolsConfig) -> Result<(), String> {
        let config_path = get_config_path();

        let json = serde_json::to_string_pretty(config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(&config_path, json)
            .map_err(|e| format!("Failed to write config: {}", e))?;

        Ok(())
    }

    /// Update the enabled status for a PE tool and save
    pub fn set_pe_tool_enabled(tool_name: &str, enabled: bool) -> Result<(), String> {
        let mut config = load_pe_tools_config();
        config.enabled_tools.insert(tool_name.to_string(), enabled);
        save_pe_tools_config(&config)
    }

    // (Unused pe_tools helpers removed for release: add_custom_pe_tool, get_tools_by_category,
    //  get_enabled_tools, get_enabled_shell, get_auto_launch_tools, get_shortcut_tools,
    //  tool_needs_download, get_tools_needing_download, get_tools_summary, category_display_name)

    // ============================================
    // PE TOOL DOWNLOADING
    // ============================================
    // Downloads PE tools at build time from their official sources.
    // Supports: .7z, .zip archives and self-extracting .exe installers.

    use std::io::{Read as IoRead, Write as IoWrite};

    /// Type of download/archive format
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum PeDownloadType {
        /// .7z archive - extract with 7-Zip
        SevenZip,
        /// .zip archive - extract with 7-Zip or built-in
        Zip,
        /// Self-extracting installer (like 7-Zip itself)
        SelfExtractingExe,
        /// Direct executable - just download and place
        DirectExe,
        /// Unknown format
        Unknown,
    }

    /// Detect the download type from a URL
    pub fn detect_download_type(url: &str) -> PeDownloadType {
        let url_lower = url.to_lowercase();

        if url_lower.ends_with(".7z") {
            PeDownloadType::SevenZip
        } else if url_lower.ends_with(".zip") {
            PeDownloadType::Zip
        } else if url_lower.ends_with(".exe") {
            // Check if it looks like an installer or direct exe
            // Installers typically have version numbers or "setup" in the name
            if url_lower.contains("setup") || url_lower.contains("install") {
                PeDownloadType::SelfExtractingExe
            } else {
                // Assume self-extracting for .exe downloads (like 7z2408-x64.exe)
                PeDownloadType::SelfExtractingExe
            }
        } else {
            PeDownloadType::Unknown
        }
    }

    /// Result of a PE tool download operation
    #[derive(Debug)]
    pub struct PeDownloadResult {
        pub tool_name: String,
        pub success: bool,
        pub error_message: Option<String>,
        pub files_extracted: Vec<String>,
    }

    /// Progress callback type for downloads
    /// Parameters: (tool_name, current_tool_index, total_tools, percent_complete)
    pub type DownloadProgressCallback = Box<dyn Fn(&str, usize, usize, u32) + Send>;

    /// Find 7-Zip executable for extraction
    fn find_7zip_exe() -> Option<PathBuf> {
        // Check common installation paths
        let paths = [
            PathBuf::from(r"C:\Program Files\7-Zip\7z.exe"),
            PathBuf::from(r"C:\Program Files (x86)\7-Zip\7z.exe"),
        ];

        for path in paths {
            if path.exists() {
                return Some(path);
            }
        }

        // Check if we have 7-Zip in our pe_tools (bootstrap problem - might not be there yet)
        let pe_7zip = get_pe_tools_folder().join("utilities").join("7-Zip").join("7z.exe");
        if pe_7zip.exists() {
            return Some(pe_7zip);
        }

        None
    }

    /// Download a single PE tool from its download URL
    ///
    /// # Arguments
    /// * `tool` - The PE tool to download
    /// * `progress` - Callback for progress updates (percent 0-100)
    ///
    /// # Returns
    /// Result with download result or error message
    pub fn download_pe_tool<F>(tool: &PeTool, progress: F) -> PeDownloadResult
    where
        F: Fn(u32),
    {
        let tool_name = tool.name.clone();

        // Check if download is needed
        if tool.download_url.is_empty() && tool.fallback_url.is_empty() {
            return PeDownloadResult {
                tool_name,
                success: false,
                error_message: Some("No download URL specified".to_string()),
                files_extracted: vec![],
            };
        }

        if tool.is_present {
            return PeDownloadResult {
                tool_name,
                success: true,
                error_message: None,
                files_extracted: vec![tool.exe.clone()],
            };
        }

        // Create destination folder if needed
        if let Err(e) = fs::create_dir_all(&tool.folder_path) {
            return PeDownloadResult {
                tool_name,
                success: false,
                error_message: Some(format!("Failed to create folder: {}", e)),
                files_extracted: vec![],
            };
        }

        // Build list of URLs to try: primary first, then fallback
        let mut urls_to_try: Vec<&str> = Vec::new();
        if !tool.download_url.is_empty() {
            urls_to_try.push(&tool.download_url);
        }
        if !tool.fallback_url.is_empty() {
            urls_to_try.push(&tool.fallback_url);
        }

        // Try each URL until one succeeds
        let mut last_error = String::new();
        for (url_index, url) in urls_to_try.iter().enumerate() {
            let is_fallback = url_index > 0;
            if is_fallback {
                println!("  Primary download failed, trying GitHub fallback: {}", url);
            } else {
                println!("Downloading PE tool: {} from {}", tool.name, url);
            }
            progress(0);

            // Determine download type from the URL
            let download_type = detect_download_type(url);
            println!("  Download type: {:?}", download_type);

            // Determine temp filename based on download type
            let temp_ext = match download_type {
                PeDownloadType::SevenZip => "7z",
                PeDownloadType::Zip => "zip",
                PeDownloadType::SelfExtractingExe | PeDownloadType::DirectExe => "exe",
                PeDownloadType::Unknown => "download",
            };
            let temp_path = tool.folder_path.join(format!("download.{}", temp_ext));

            // Download the file
            match download_file(url, &temp_path, &progress) {
                Ok(_) => println!("  Download complete: {} bytes",
                    fs::metadata(&temp_path).map(|m| m.len()).unwrap_or(0)),
                Err(e) => {
                    last_error = format!("Download failed from {}: {}", url, e);
                    println!("  {}", last_error);
                    let _ = fs::remove_file(&temp_path);
                    continue; // Try next URL
                }
            }

            progress(80);

            // Extract based on download type
            let extract_result = match download_type {
                PeDownloadType::SevenZip | PeDownloadType::Zip => {
                    extract_archive(&temp_path, &tool.folder_path)
                }
                PeDownloadType::SelfExtractingExe => {
                    extract_self_extracting_exe(&temp_path, &tool.folder_path)
                }
                PeDownloadType::DirectExe => {
                    // Just rename the file to the expected exe name
                    let dest_exe = tool.folder_path.join(&tool.exe);
                    fs::rename(&temp_path, &dest_exe)
                        .map(|_| vec![tool.exe.clone()])
                        .map_err(|e| e.to_string())
                }
                PeDownloadType::Unknown => {
                    Err("Unknown download format".to_string())
                }
            };

            // Clean up temp file if it still exists
            let _ = fs::remove_file(&temp_path);

            // Flatten single-subfolder archives:
            // Some archives (e.g., pcassistsoftware .7z files) extract into a subfolder
            // like DISMTool/DISM_Tool/DISM_Tool.exe instead of DISMTool/DISM_Tool.exe.
            // If the tool exe isn't found but a single subfolder exists, move its contents up.
            let expected_exe = tool.folder_path.join(&tool.exe);
            if !expected_exe.exists() {
                // Look for a single subfolder that might contain the exe
                if let Ok(entries) = fs::read_dir(&tool.folder_path) {
                    let subdirs: Vec<_> = entries.flatten()
                        .filter(|e| e.path().is_dir())
                        .collect();
                    // If there's exactly one subfolder, check if our exe is inside it
                    if subdirs.len() == 1 {
                        let subfolder = subdirs[0].path();
                        let nested_exe = subfolder.join(&tool.exe);
                        if nested_exe.exists() {
                            println!("  Flattening nested folder: {}", subfolder.display());
                            // Move all files from subfolder up to the tool folder
                            if let Ok(sub_entries) = fs::read_dir(&subfolder) {
                                for entry in sub_entries.flatten() {
                                    let src = entry.path();
                                    let dest = tool.folder_path.join(entry.file_name());
                                    if !dest.exists() {
                                        if let Err(e) = fs::rename(&src, &dest) {
                                            // rename can fail across drives, fall back to copy
                                            println!("    Rename failed ({}), trying copy...", e);
                                            if src.is_file() {
                                                let _ = fs::copy(&src, &dest);
                                                let _ = fs::remove_file(&src);
                                            }
                                        }
                                    }
                                }
                            }
                            // Remove the now-empty subfolder
                            let _ = fs::remove_dir_all(&subfolder);
                        }
                    }
                }
            }

            progress(100);

            // Check if extraction succeeded
            match extract_result {
                Ok(files) => {
                    if is_fallback {
                        println!("  GitHub fallback succeeded: {} files extracted", files.len());
                    } else {
                        println!("  Extracted {} files", files.len());
                    }
                    return PeDownloadResult {
                        tool_name,
                        success: true,
                        error_message: None,
                        files_extracted: files,
                    };
                }
                Err(e) => {
                    last_error = format!("Extraction failed: {}", e);
                    println!("  {}", last_error);
                    continue; // Try next URL
                }
            }
        }

        // All URLs failed
        PeDownloadResult {
            tool_name,
            success: false,
            error_message: Some(last_error),
            files_extracted: vec![],
        }
    }

    /// Download a file from URL to destination path
    fn download_file<F>(url: &str, dest_path: &Path, progress: &F) -> Result<(), String>
    where
        F: Fn(u32),
    {
        // Create HTTP client with redirect support
        let client = reqwest::blocking::Client::builder()
            .user_agent("MasterBooter/1.0")
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(300)) // 5 minute timeout
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        // Send request
        let response = client
            .get(url)
            .send()
            .map_err(|e| format!("Failed to connect: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()));
        }

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        // Create output file
        let mut file = std::fs::File::create(dest_path)
            .map_err(|e| format!("Failed to create file: {}", e))?;

        // Download with progress
        let mut reader = response;
        let mut buffer = [0u8; 8192];

        loop {
            let bytes_read = reader.read(&mut buffer)
                .map_err(|e| format!("Read error: {}", e))?;

            if bytes_read == 0 {
                break;
            }

            file.write_all(&buffer[..bytes_read])
                .map_err(|e| format!("Write error: {}", e))?;

            downloaded += bytes_read as u64;

            // Report progress (0-80% for download, 80-100% for extraction)
            if total_size > 0 {
                let percent = ((downloaded * 80) / total_size) as u32;
                progress(percent);
            }
        }

        file.flush().map_err(|e| format!("Flush error: {}", e))?;
        Ok(())
    }

    /// Extract a .7z or .zip archive using 7-Zip
    fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<Vec<String>, String> {
        let seven_zip = find_7zip_exe()
            .ok_or_else(|| "7-Zip not found. Please install 7-Zip from https://7-zip.org".to_string())?;

        println!("  Extracting with 7-Zip: {:?}", seven_zip);

        // Run 7z x "archive" -o"dest" -y
        let output = std::process::Command::new(&seven_zip)
            .arg("x")
            .arg(archive_path)
            .arg(format!("-o{}", dest_dir.to_string_lossy()))
            .arg("-y") // Yes to all prompts
            .arg("-bso0") // Suppress standard output
            .arg("-bsp0") // Suppress progress
            .output()
            .map_err(|e| format!("Failed to run 7-Zip: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("7-Zip extraction failed: {}", stderr));
        }

        // List extracted files
        let mut extracted_files = Vec::new();
        if let Ok(entries) = fs::read_dir(dest_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip the download file itself
                if !name.starts_with("download.") {
                    extracted_files.push(name);
                }
            }
        }

        Ok(extracted_files)
    }

    /// Extract a self-extracting exe using 7-Zip (or run silently)
    fn extract_self_extracting_exe(exe_path: &Path, dest_dir: &Path) -> Result<Vec<String>, String> {
        // Try 7-Zip first - it can extract most installer formats
        if let Some(seven_zip) = find_7zip_exe() {
            println!("  Extracting self-extracting exe with 7-Zip...");

            let _output = std::process::Command::new(&seven_zip)
                .arg("x")
                .arg(exe_path)
                .arg(format!("-o{}", dest_dir.to_string_lossy()))
                .arg("-y")
                .arg("-bso0")
                .arg("-bsp0")
                .output()
                .map_err(|e| format!("Failed to run 7-Zip: {}", e))?;

            // Check if extraction produced any files (other than download.exe)
            let mut extracted_files = Vec::new();
            if let Ok(entries) = fs::read_dir(dest_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let lower = name.to_lowercase();
                    if !lower.starts_with("download.") && lower.ends_with(".exe") {
                        extracted_files.push(name);
                    }
                }
            }

            if !extracted_files.is_empty() {
                return Ok(extracted_files);
            }

            // If 7-Zip didn't extract anything useful, the exe might need to be run
            println!("  7-Zip extraction didn't find EXEs, trying silent install...");
        }

        // Fallback: Try running the installer silently
        // Common silent install flags for different installer types
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            // Try Inno Setup style first
            let inno_result = std::process::Command::new(exe_path)
                .raw_arg(format!("/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR=\"{}\"",
                    dest_dir.to_string_lossy()))
                .status();

            if inno_result.is_ok() {
                // Wait a moment for files to be written
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
        }

        // List any extracted files
        let mut extracted_files = Vec::new();
        if let Ok(entries) = fs::read_dir(dest_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let lower = name.to_lowercase();
                if !lower.starts_with("download.") {
                    extracted_files.push(name);
                }
            }
        }

        if extracted_files.is_empty() {
            Err("No files extracted. The installer may require manual installation.".to_string())
        } else {
            Ok(extracted_files)
        }
    }

    /// Download all enabled PE tools that are missing
    ///
    /// # Arguments
    /// * `tools` - List of all PE tools (will download those that are enabled but not present)
    /// * `progress` - Callback for overall progress (tool_name, current_index, total, percent)
    ///
    /// # Returns
    /// Vector of download results for each tool
    pub fn download_enabled_pe_tools(
        tools: &[PeTool],
        progress: impl Fn(&str, usize, usize, u32),
    ) -> Vec<PeDownloadResult> {
        // Get tools that need downloading
        let tools_to_download: Vec<&PeTool> = tools.iter()
            .filter(|t| t.enabled && !t.is_present && !t.download_url.is_empty())
            .collect();

        let total = tools_to_download.len();
        let mut results = Vec::new();

        if total == 0 {
            println!("No PE tools need downloading - all present or disabled");
            return results;
        }

        println!("Downloading {} PE tools...", total);

        for (index, tool) in tools_to_download.iter().enumerate() {
            // Report which tool we're starting
            progress(&tool.name, index + 1, total, 0);

            // Download with progress callback that updates overall progress
            let result = download_pe_tool(tool, |percent| {
                progress(&tool.name, index + 1, total, percent);
            });

            results.push(result);
        }

        // Summary
        let success_count = results.iter().filter(|r| r.success).count();
        let fail_count = results.iter().filter(|r| !r.success).count();
        println!("Download complete: {} succeeded, {} failed", success_count, fail_count);

        results
    }

    /// Copy all downloaded PE tools to a GitHub staging folder.
    /// This creates the folder structure needed to upload tools as GitHub release assets.
    /// Each tool gets its own folder under the staging directory, matching the pe_tools structure.
    ///
    /// The staging folder is: C:\github\Master-Booter\pe_tools\{category}\{folder_name}\
    ///
    /// Call this after downloading tools so the user can upload them to GitHub
    /// as a fallback download source.
    pub fn copy_tools_to_github_staging(tools: &[PeTool]) -> (usize, usize) {
        let staging_base = PathBuf::from(r"C:\github\Master-Booter\pe_tools");
        let mut copied = 0;
        let mut failed = 0;

        println!("Copying PE tools to GitHub staging: {}", staging_base.display());

        for tool in tools {
            // Only copy tools that are actually present (downloaded)
            if !tool.is_present {
                continue;
            }

            // Build destination path: staging/category/folder_name/
            // e.g., C:\github\Master-Booter\pe_tools\disk\CrystalDiskInfo\
            let folder_name = tool.folder_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let dest_dir = staging_base.join(&tool.category).join(&folder_name);

            // Create destination folder
            if let Err(e) = fs::create_dir_all(&dest_dir) {
                println!("  Failed to create staging folder for {}: {}", tool.name, e);
                failed += 1;
                continue;
            }

            // Copy all files from the tool folder to the staging folder
            match copy_dir_contents(&tool.folder_path, &dest_dir) {
                Ok(count) => {
                    println!("  Copied {} ({} files) to {}", tool.name, count, dest_dir.display());
                    copied += 1;
                }
                Err(e) => {
                    println!("  Failed to copy {}: {}", tool.name, e);
                    failed += 1;
                }
            }
        }

        println!("GitHub staging complete: {} tools copied, {} failed", copied, failed);
        (copied, failed)
    }

    /// Recursively copy all files from src directory to dest directory
    fn copy_dir_contents(src: &Path, dest: &Path) -> Result<usize, String> {
        let mut count = 0;

        let entries = fs::read_dir(src)
            .map_err(|e| format!("Failed to read {}: {}", src.display(), e))?;

        for entry in entries.flatten() {
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());

            if src_path.is_dir() {
                // Recursively copy subdirectories
                fs::create_dir_all(&dest_path)
                    .map_err(|e| format!("Failed to create dir {}: {}", dest_path.display(), e))?;
                count += copy_dir_contents(&src_path, &dest_path)?;
            } else if src_path.is_file() {
                // Skip temporary download files
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("download.") {
                    continue;
                }
                // Copy the file
                fs::copy(&src_path, &dest_path)
                    .map_err(|e| format!("Failed to copy {}: {}", src_path.display(), e))?;
                count += 1;
            }
        }

        Ok(count)
    }

    // (Unused functions download_pe_tool_by_name, verify_enabled_tools removed for release)
}

// ============================================
// TESTS
// ============================================
#[cfg(test)]
mod tests {
    use super::pe_tools::*;

    /// Test downloading a PE tool (Explorer++ - small zip file)
    /// Run with: cargo test test_download_pe_tool -- --nocapture --ignored
    #[test]
    #[ignore] // Ignored by default - requires network and 7-Zip
    fn test_download_pe_tool() {
        println!("\n=== Testing PE Tool Download ===\n");

        // First, discover all PE tools
        println!("1. Discovering PE tools...");
        let tools = discover_pe_tools();
        println!("   Found {} tools", tools.len());

        for tool in &tools {
            println!("   - {} ({}) - present: {}, url: {}",
                tool.name, tool.category, tool.is_present,
                if tool.download_url.is_empty() { "none" } else { "yes" });
        }

        // Find Explorer++ (it's a small zip, good for testing)
        let test_tool = tools.iter()
            .find(|t| t.name == "Explorer++")
            .expect("Explorer++ tool not found in pe_tools");

        println!("\n2. Testing download of: {}", test_tool.name);
        println!("   URL: {}", test_tool.download_url);
        println!("   Expected exe: {}", test_tool.exe);
        println!("   Destination: {:?}", test_tool.folder_path);

        // Download with progress
        println!("\n3. Downloading...");
        let result = download_pe_tool(test_tool, |percent| {
            if percent % 20 == 0 || percent == 100 {
                println!("   Progress: {}%", percent);
            }
        });

        // Check result
        println!("\n4. Result:");
        println!("   Success: {}", result.success);
        if let Some(err) = &result.error_message {
            println!("   Error: {}", err);
        }
        println!("   Files extracted: {:?}", result.files_extracted);

        // Verify the exe exists
        let exe_path = test_tool.folder_path.join(&test_tool.exe);
        println!("\n5. Verifying exe exists at: {:?}", exe_path);
        let exists = exe_path.exists();
        println!("   Exists: {}", exists);

        assert!(result.success, "Download should succeed");
        assert!(exists, "Executable should exist after download");

        println!("\n=== Test Passed! ===\n");
    }

    /// Test downloading multiple tools
    /// Run with: cargo test test_download_multiple -- --nocapture --ignored
    #[test]
    #[ignore]
    fn test_download_multiple() {
        println!("\n=== Testing Multiple PE Tool Downloads ===\n");

        // Discover tools
        let mut tools = discover_pe_tools();

        // Enable tools we want to test downloading
        for tool in &mut tools {
            // Test CrystalDiskInfo download with new URL
            tool.enabled = tool.name == "CrystalDiskInfo";
        }

        println!("Enabled tools:");
        for tool in tools.iter().filter(|t| t.enabled) {
            println!("  - {} (present: {})", tool.name, tool.is_present);
        }

        // Download all enabled
        println!("\nDownloading...");
        let results = download_enabled_pe_tools(&tools, |name, current, total, percent| {
            println!("  [{}/{}] {} - {}%", current, total, name, percent);
        });

        // Summary
        println!("\nResults:");
        for result in &results {
            let status = if result.success { "OK" } else { "FAILED" };
            println!("  {} - {}", result.tool_name, status);
            if let Some(err) = &result.error_message {
                println!("    Error: {}", err);
            }
        }

        let success_count = results.iter().filter(|r| r.success).count();
        println!("\n{}/{} downloads succeeded", success_count, results.len());
    }
}
