// ============================================
// MasterBooter - updater.rs
// ============================================
// This module handles auto-updating MasterBooter from GitHub Releases.
//
// How it works:
// 1. On startup (background thread), we call the GitHub API to check
//    if a newer release exists.
// 2. If an update is available, a badge appears in the sidebar.
// 3. The user clicks the badge to download the new EXE.
// 4. The `self_replace` crate swaps the running EXE with the new one.
// 5. The user restarts to use the new version.
//
// We also track the EXE version between runs. When the version changes
// (i.e., after an update), we refresh the PE tool manifests from the
// embedded defaults so any new tools or updated URLs take effect.
// ============================================

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;

// ============================================
// CONSTANTS
// ============================================

/// GitHub API endpoint for the latest release of MasterBooter.
/// This returns JSON with the tag name, release notes, and download assets.
const GITHUB_API_URL: &str =
    "https://api.github.com/repos/Howweird/Masterbooter/releases/latest";

/// The filename we expect to find in the GitHub release assets.
/// This is the EXE file that users download.
const EXE_ASSET_NAME: &str = "masterbooter.exe";

/// Filename for tracking which version last ran (stored next to the EXE).
/// We use this to detect when the EXE has been updated so we can
/// refresh PE tool manifests with any new download URLs or settings.
const VERSION_FILE_NAME: &str = "masterbooter_version.json";

// ============================================
// DATA STRUCTURES
// ============================================

/// Information about a GitHub release.
/// We only include the fields we care about — serde ignores the rest.
/// The GitHub API returns many more fields, but we don't need them.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    /// The release tag, e.g. "v1.2.0"
    pub tag_name: String,

    /// Release notes written by the developer (markdown text).
    /// May be empty if no notes were provided.
    pub body: Option<String>,

    /// List of downloadable files attached to this release.
    /// We look for "masterbooter.exe" in this list.
    pub assets: Vec<GitHubAsset>,
}

/// A single downloadable file in a GitHub release.
/// Each release can have multiple assets (e.g., EXE, ZIP, checksums).
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubAsset {
    /// The filename, e.g. "masterbooter.exe"
    pub name: String,

    /// The direct download URL for this file.
    /// This URL works without authentication for public repos.
    pub browser_download_url: String,

    /// File size in bytes (used to show "8.2 MB" in the UI)
    pub size: u64,
}

/// The result of checking GitHub for updates.
/// This struct is passed back to the UI thread to update the interface.
/// On any error, `update_available` is false and `error` has a message.
#[derive(Debug, Clone)]
pub struct UpdateCheckResult {
    /// Is a newer version available on GitHub?
    pub update_available: bool,

    /// The latest version string (e.g. "1.2.0"), without the "v" prefix
    pub latest_version: String,

    /// The version of the currently running EXE (e.g. "0.1.0")
    pub current_version: String,

    /// Release notes from GitHub (may be empty)
    pub release_notes: String,

    /// Download URL for the new EXE (empty if no update)
    pub download_url: String,

    /// Size of the new EXE in bytes (0 if no update)
    pub download_size: u64,

    /// Error message if the check failed (empty on success)
    pub error: String,
}

/// Tracks which version of MasterBooter last ran.
/// Stored as `masterbooter_version.json` next to the EXE.
/// When the EXE updates, the version changes, and we know to
/// refresh PE tool manifests on the next startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    /// The version string of the last EXE that ran (e.g. "0.1.0")
    pub last_run_version: String,
}

// ============================================
// UPDATE CHECK
// ============================================

/// Check GitHub for a newer release of MasterBooter.
///
/// How it works:
/// 1. Call the GitHub API to get the latest release info
/// 2. Parse the version from the tag_name (e.g. "v1.2.0" -> "1.2.0")
/// 3. Compare with our current version (from Cargo.toml at compile time)
/// 4. Return the result with download URL if an update exists
///
/// This function is safe to call from a background thread.
/// It blocks while waiting for the HTTP response (usually < 1 second).
/// On any error (no internet, rate limited, etc.), it returns a result
/// with update_available = false and the error message filled in.
pub fn check_for_updates() -> UpdateCheckResult {
    // Get our current version (baked in at compile time from Cargo.toml)
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    // Helper: create a "failed" result with an error message
    let make_error = |msg: String| UpdateCheckResult {
        update_available: false,
        latest_version: String::new(),
        current_version: current_version.clone(),
        release_notes: String::new(),
        download_url: String::new(),
        download_size: 0,
        error: msg,
    };

    // Build HTTP client (same pattern as tools.rs download_tool)
    let client = match reqwest::blocking::Client::builder()
        .user_agent("MasterBooter/1.0")
        .timeout(std::time::Duration::from_secs(10)) // 10 second timeout for the API call
        .build()
    {
        Ok(c) => c,
        Err(e) => return make_error(format!("Failed to create HTTP client: {}", e)),
    };

    // Query the GitHub API for the latest release
    let response = match client
        .get(GITHUB_API_URL)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
    {
        Ok(r) => r,
        Err(e) => return make_error(format!("Could not reach GitHub: {}", e)),
    };

    // Check for HTTP errors (404 = no releases yet, 403 = rate limited, etc.)
    if !response.status().is_success() {
        return make_error(format!("GitHub API returned status {}", response.status()));
    }

    // Read the response body as text, then parse it as JSON.
    // We do this instead of response.json() to avoid needing the
    // reqwest "json" feature (which adds ~3 MB to the binary).
    let body_text = match response.text() {
        Ok(t) => t,
        Err(e) => return make_error(format!("Failed to read response: {}", e)),
    };

    let release: GitHubRelease = match serde_json::from_str(&body_text) {
        Ok(r) => r,
        Err(e) => return make_error(format!("Failed to parse release info: {}", e)),
    };

    // Strip the "v" prefix from the tag (e.g. "v1.2.0" -> "1.2.0")
    let latest_version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name)
        .to_string();

    // Find the masterbooter.exe asset in the release
    // (case-insensitive match in case the filename has different casing)
    let exe_asset = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(EXE_ASSET_NAME));

    let (download_url, download_size) = match exe_asset {
        Some(asset) => (asset.browser_download_url.clone(), asset.size),
        None => (String::new(), 0),
    };

    // Compare versions to see if the latest is newer than ours
    let update_available = is_newer_version(&current_version, &latest_version);

    UpdateCheckResult {
        update_available,
        latest_version,
        current_version,
        release_notes: release.body.unwrap_or_default(),
        download_url,
        download_size,
        error: String::new(),
    }
}

// ============================================
// VERSION COMPARISON
// ============================================

/// Compare two version strings (e.g. "0.1.0" vs "1.2.0").
/// Returns true if `latest` is strictly newer than `current`.
///
/// Uses simple numeric comparison of major.minor.patch.
/// Non-numeric parts are treated as 0 (safe fallback).
///
/// Examples:
///   is_newer_version("0.1.0", "0.2.0") => true
///   is_newer_version("1.0.0", "1.0.0") => false
///   is_newer_version("2.0.0", "1.0.0") => false
fn is_newer_version(current: &str, latest: &str) -> bool {
    // Parse a version string like "1.2.3" into (1, 2, 3)
    let parse = |s: &str| -> (u32, u32, u32) {
        let parts: Vec<u32> = s.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        (
            parts.first().copied().unwrap_or(0), // major
            parts.get(1).copied().unwrap_or(0),  // minor
            parts.get(2).copied().unwrap_or(0),  // patch
        )
    };

    let current_tuple = parse(current);
    let latest_tuple = parse(latest);

    // Rust tuples compare element by element: (1,2,3) > (1,2,0) is true
    latest_tuple > current_tuple
}

// ============================================
// DOWNLOAD AND SELF-REPLACE
// ============================================

/// Download the new EXE from GitHub and replace the running one.
///
/// How self-replacement works on Windows:
/// 1. Download the new EXE to a temporary file next to the running EXE
/// 2. The self_replace crate moves the running EXE aside (renames it)
/// 3. The new EXE is copied into the original filename
/// 4. The old EXE is scheduled for deletion when the process exits
/// 5. User must restart MasterBooter to use the new version
///
/// This function blocks during download. Call it from a background thread!
/// The progress_callback receives values 0-100 for download progress.
///
/// Returns a success message on completion, or an error if something went wrong.
pub fn download_and_replace_exe(
    download_url: &str,
    progress_callback: impl Fn(u32),
) -> Result<String, String> {
    println!("Starting EXE update download from: {}", download_url);
    progress_callback(0);

    // Build HTTP client (same pattern as tools.rs)
    let client = reqwest::blocking::Client::builder()
        .user_agent("MasterBooter/1.0")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(300)) // 5 min timeout for download
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Determine where to save the temp file (next to the running EXE)
    let app_dir = crate::tools::get_app_directory();
    let temp_path = app_dir.join("masterbooter_update.tmp");

    // Send the HTTP request
    let response = client
        .get(download_url)
        .send()
        .map_err(|e| format!("Failed to connect to download server: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    // Get total file size for progress tracking (may be 0 if server doesn't report it)
    let total_size = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;

    // Create the temp file and download in 8KB chunks (same as tools.rs)
    let mut file = std::fs::File::create(&temp_path)
        .map_err(|e| format!("Failed to create temp file for update: {}", e))?;

    let mut reader = response;
    let mut buffer = [0u8; 8192]; // 8KB buffer — same as tools.rs

    loop {
        // Read a chunk from the network
        let bytes_read = reader
            .read(&mut buffer)
            .map_err(|e| format!("Error reading download data: {}", e))?;

        // If we got 0 bytes, the download is complete
        if bytes_read == 0 {
            break;
        }

        // Write the chunk to the temp file
        file.write_all(&buffer[..bytes_read])
            .map_err(|e| format!("Error writing update file: {}", e))?;

        // Update progress (0-90% for download, 90-100% for replace)
        downloaded += bytes_read as u64;
        if total_size > 0 {
            let percent = ((downloaded * 90) / total_size) as u32;
            progress_callback(percent.min(90)); // Cap at 90% during download
        }
    }

    // Make sure everything is written to disk
    file.flush()
        .map_err(|e| format!("Error flushing update file: {}", e))?;
    drop(file); // Close the file handle before replacing

    println!(
        "Download complete ({} bytes). Performing self-replace...",
        downloaded
    );
    progress_callback(95);

    // Use self_replace to swap the running EXE with the downloaded one.
    // This is the magic step that handles Windows EXE locking:
    // - Moves the running EXE to a temp name
    // - Copies the new file to the original name
    // - Schedules cleanup of the old file
    self_replace::self_replace(&temp_path).map_err(|e| {
        format!(
            "Failed to replace EXE: {}. Try closing other instances of MasterBooter and retry.",
            e
        )
    })?;

    // Clean up the temp file (self_replace copies it, so the temp can be deleted)
    let _ = std::fs::remove_file(&temp_path);

    progress_callback(100);
    println!("Self-replace successful! Restart to use the new version.");

    Ok("Update installed! Restart MasterBooter to use the new version.".to_string())
}

// ============================================
// FILE SIZE FORMATTING
// ============================================

/// Format a byte count as a human-readable size string.
///
/// Examples:
///   format_size(9_000_000) => "8.6 MB"
///   format_size(512_000)   => "500 KB"
///   format_size(1_500_000_000) => "1.4 GB"
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} bytes", bytes)
    }
}

// ============================================
// VERSION TRACKING
// ============================================
// We store which version last ran in a JSON file next to the EXE.
// When the version changes (after an update), we know to refresh
// the PE tool manifests — because the new EXE might have updated
// download URLs, new tools, or other manifest changes.

/// Check if the EXE version has changed since the last run.
/// Returns true if this is a new version (or the very first run).
///
/// How it works:
/// 1. Read masterbooter_version.json from next to the EXE
/// 2. Compare the stored version against the current CARGO_PKG_VERSION
/// 3. If different (or file missing), return true
pub fn check_version_change() -> bool {
    let version_path = get_version_file_path();
    let current = env!("CARGO_PKG_VERSION");

    // Try to read the version file
    if version_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&version_path) {
            if let Ok(info) = serde_json::from_str::<VersionInfo>(&content) {
                if info.last_run_version == current {
                    // Same version as last time — no change
                    return false;
                }
                println!(
                    "Version changed: {} -> {}",
                    info.last_run_version, current
                );
                return true;
            }
        }
    }

    // File doesn't exist or couldn't be parsed — treat as first run
    println!("First run or version file missing — will refresh PE tool manifests");
    true
}

/// Save the current version to the version tracking file.
/// Called on every startup (creates the file on first run,
/// or updates it after an EXE update).
pub fn save_current_version() {
    let version_path = get_version_file_path();
    let info = VersionInfo {
        last_run_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    // Write the JSON file
    match serde_json::to_string_pretty(&info) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&version_path, json) {
                eprintln!("Warning: Could not save version file: {}", e);
            }
        }
        Err(e) => eprintln!("Warning: Could not serialize version info: {}", e),
    }
}

/// Get the path to the version tracking file (next to the EXE).
fn get_version_file_path() -> PathBuf {
    crate::tools::get_app_directory().join(VERSION_FILE_NAME)
}

// ============================================
// PE TOOL MANIFEST REFRESH
// ============================================

/// Refresh PE tool manifests from the embedded defaults.
/// Called when a version change is detected (after an EXE update).
///
/// This overwrites the tool.toml files in pe_tools/ with the latest
/// embedded defaults from the new EXE. This is important because
/// tool download URLs or settings may have changed in the new version.
///
/// Note: Downloaded tool binaries are NOT deleted — only the
/// manifest files (tool.toml) are refreshed.
pub fn refresh_pe_tool_manifests() {
    println!("Refreshing PE tool manifests from embedded defaults...");
    crate::tools::pe_tools::refresh_default_manifests();
    println!("PE tool manifests refreshed.");
}

// ============================================
// TESTS
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        // Newer versions
        assert!(is_newer_version("0.1.0", "0.2.0"));
        assert!(is_newer_version("0.1.0", "1.0.0"));
        assert!(is_newer_version("1.0.0", "1.0.1"));
        assert!(is_newer_version("0.9.9", "1.0.0"));

        // Same version
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));

        // Older versions
        assert!(!is_newer_version("1.0.0", "0.9.0"));
        assert!(!is_newer_version("2.0.0", "1.0.0"));
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 bytes");
        assert_eq!(format_size(500), "500 bytes");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1_048_576), "1.0 MB");
        assert_eq!(format_size(9_000_000), "8.6 MB");
        assert_eq!(format_size(1_073_741_824), "1.0 GB");
    }
}
