// ============================================
// MasterBooter - main.rs
// ============================================
// This is the entry point of the application.
// When you run `cargo run`, this file's `main()` function is called first.
//
// The program flow is:
// 1. main() starts
// 2. Create the UI window from the Slint file
// 3. Set up callbacks (what happens when buttons are clicked)
// 4. Run the UI event loop (keeps the window open)
// ============================================

// Include the compiled Slint UI code
// This macro reads the generated code from build.rs
slint::include_modules!();

use std::path::Path;
use slint::Model;  // Needed for .row_count() and .row_data() on ComboBox models

// Our modules
mod tools;
mod winpe;
mod adk_packages;  // ADK package management for WinPE
mod pe_fixes;      // PE fixes and workarounds
mod deploy;        // Windows deployment module
mod updater;       // Auto-update from GitHub releases

// ============================================
// MAIN FUNCTION
// ============================================
// This is where the program starts executing.
// In Rust, `fn main()` is always the entry point.

fn main() -> Result<(), slint::PlatformError> {
    // Print startup message to console (helpful for debugging)
    println!("============================================");
    println!("MasterBooter v{}", env!("CARGO_PKG_VERSION"));
    println!("============================================");

    // Log key paths for debugging
    println!("EXE: {:?}", std::env::current_exe().unwrap_or_default());
    println!("App directory: {:?}", tools::get_app_directory());
    println!("Backup tools: {:?}", tools::get_backup_tools_path());
    println!("PE tools: {:?}", tools::pe_tools::get_pe_tools_folder());

    // Detect environment (Live Windows vs WinPE)
    let is_winpe = detect_winpe_environment();
    if is_winpe {
        println!("Running in WinPE environment");
    } else {
        println!("Running in Live Windows environment");
    }

    // Create the main window from the Slint UI definition
    // MainWindow is defined in src/ui/main.slint
    let ui = MainWindow::new()?;

    // ============================================
    // SET UP UI STATE
    // ============================================

    // Tell the UI whether we're in WinPE (it might show different options)
    ui.set_is_winpe(is_winpe);

    // Set the version string
    ui.set_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());

    // ============================================
    // SET UP CALLBACKS
    // ============================================
    // Callbacks connect UI buttons to Rust functions.
    // When a button is clicked in the UI, the corresponding callback runs.

    // Clone the UI handle for use in callbacks
    // (Rust ownership rules require this)
    let ui_handle = ui.as_weak();

    // Callback: Mode changed (user clicked a sidebar button)
    // Auto-detect dependencies when WinPE Builder is selected
    ui.on_mode_changed({
        let ui = ui_handle.clone();
        move |mode| {
            println!("Mode changed to: {}", mode);
            if let Some(ui) = ui.upgrade() {
                ui.set_status_text(format!("{} mode selected", mode).into());

                // Auto-detect when WinPE Builder is selected
                if mode == "WinPE Builder" {
                    // Show detecting status immediately
                    ui.set_pe_detecting(true);
                    ui.set_deps_status("Detecting dependencies...".into());
                    ui.set_status_text("Detecting WinRE, ADK, and dependencies...".into());

                    // Run detection (this happens synchronously, but it's fast)
                    let winre_info = winpe::detect_winre();
                    let adk_info = winpe::detect_adk();
                    let deps = winpe::check_pe_build_dependencies();

                    // Update WinRE status
                    ui.set_winre_found(winre_info.found);
                    if winre_info.found {
                        ui.set_winre_path(winre_info.path.to_string_lossy().to_string().into());
                        ui.set_winre_size(winre_info.size_display.into());
                    }

                    // Update ADK status
                    ui.set_adk_found(deps.adk_installed);
                    if deps.adk_installed {
                        ui.set_adk_version(adk_info.version.into());
                        ui.set_adk_path(deps.adk_path.clone().into());
                    }

                    // Update other dependencies
                    ui.set_winpe_addon_found(deps.winpe_addon_installed);
                    ui.set_winpe_addon_path(deps.winpe_addon_path.clone().into());
                    ui.set_oscdimg_found(deps.oscdimg_available);
                    ui.set_oscdimg_path(deps.oscdimg_path.clone().into());
                    ui.set_seven_zip_found(deps.seven_zip_available);
                    ui.set_seven_zip_path(deps.seven_zip_path.clone().into());
                    ui.set_dism_found(deps.dism_available);
                    ui.set_powershell_found(deps.powershell_available);
                    ui.set_disk_space_ok(deps.disk_space_ok);
                    ui.set_disk_space_gb(deps.disk_space_gb as f32);
                    ui.set_all_deps_satisfied(deps.all_satisfied);

                    // Build status message
                    let status_msg = if deps.all_satisfied {
                        "All dependencies satisfied. Ready to build!".to_string()
                    } else {
                        let missing_count = deps.errors.len();
                        format!("{} missing dependencies - click 'Install Dependencies' to fix", missing_count)
                    };
                    ui.set_deps_status(status_msg.clone().into());
                    ui.set_status_text(status_msg.into());

                    // Set default output path if not already set
                    let current_output: String = ui.get_pe_output_path().to_string();
                    if current_output.is_empty() {
                        let default_path = winpe::get_default_output_path();
                        ui.set_pe_output_path(default_path.to_string_lossy().to_string().into());
                    }

                    // Scan PE tools to update status dots (green/orange)
                    update_pe_tool_status(&ui, 0);

                    // Detection complete
                    ui.set_pe_detecting(false);
                }
            }
        }
    });

    // Callback: Settings button clicked — triggers manual update check
    // This checks GitHub for a newer release and shows the result to the user.
    // Unlike the startup check, errors are shown here (user explicitly asked).
    ui.on_settings_clicked({
        let ui = ui_handle.clone();
        move || {
            println!("Settings clicked — checking for updates");
            if let Some(ui) = ui.upgrade() {
                ui.set_update_checking(true);
                ui.set_status_text("Checking for updates...".into());
            }

            // Run the update check on a background thread so the UI doesn't freeze
            let ui_for_check = ui.clone();
            std::thread::spawn(move || {
                let result = updater::check_for_updates();

                // Send results back to the UI thread
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_for_check.upgrade() {
                        ui.set_update_checking(false);

                        if result.update_available {
                            // Update found! Show the badge and info
                            ui.set_update_available(true);
                            ui.set_update_latest_version(
                                format!("v{}", result.latest_version).into(),
                            );
                            ui.set_update_release_notes(result.release_notes.into());
                            ui.set_update_download_url(result.download_url.into());
                            ui.set_update_size_display(
                                updater::format_size(result.download_size).into(),
                            );
                            ui.set_status_text(
                                format!(
                                    "Update available: v{} ({}) — click the badge in the sidebar to download",
                                    result.latest_version,
                                    updater::format_size(result.download_size)
                                )
                                .into(),
                            );
                        } else if !result.error.is_empty() {
                            // Check failed — show the error (manual check = user wants to know)
                            ui.set_update_error(result.error.clone().into());
                            ui.set_status_text(
                                format!("Update check failed: {}", result.error).into(),
                            );
                        } else {
                            // Already up to date
                            ui.set_status_text(
                                format!("You're up to date! (v{})", result.current_version).into(),
                            );
                        }
                    }
                });
            });
        }
    });

    // Callback: Download and install update from GitHub
    // Downloads the new EXE, replaces the running one, and prompts to restart.
    ui.on_download_update({
        let ui = ui_handle.clone();
        move || {
            println!("Download update clicked");

            // Get the download URL from the UI property
            let download_url = if let Some(ui) = ui.upgrade() {
                let url = ui.get_update_download_url().to_string();
                if url.is_empty() {
                    ui.set_status_text(
                        "No download URL available. Try checking for updates again.".into(),
                    );
                    return;
                }
                // Show download starting in the UI
                ui.set_update_download_progress(0);
                ui.set_status_text("Downloading update...".into());
                url
            } else {
                return;
            };

            // Download on a background thread so the UI stays responsive
            let ui_for_progress = ui.clone();
            let ui_for_done = ui.clone();

            std::thread::spawn(move || {
                // The progress callback sends updates back to the UI thread
                let result =
                    updater::download_and_replace_exe(&download_url, |progress| {
                        let ui_p = ui_for_progress.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_p.upgrade() {
                                ui.set_update_download_progress(progress as i32);
                            }
                        });
                    });

                // Send the final result back to the UI
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_for_done.upgrade() {
                        ui.set_update_download_progress(-1); // Reset progress

                        match result {
                            Ok(message) => {
                                // Success! The EXE has been replaced on disk.
                                ui.set_update_installed(true);
                                ui.set_update_available(false); // Hide the badge
                                ui.set_status_text(message.into());
                            }
                            Err(e) => {
                                // Download or replace failed
                                ui.set_update_error(format!("{}", e).into());
                                ui.set_status_text(
                                    format!(
                                        "Update failed: {}. Try downloading manually from GitHub.",
                                        e
                                    )
                                    .into(),
                                );
                            }
                        }
                    }
                });
            });
        }
    });

    // Callback: Dismiss update notification
    // Hides the update badge without downloading. User can check again from Settings.
    ui.on_dismiss_update({
        let ui = ui_handle.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_update_available(false);
                ui.set_status_text(
                    "Update dismissed. Click Settings to check again.".into(),
                );
            }
        }
    });

    // Callback: Tool card clicked - show launcher popup
    ui.on_tool_clicked({
        let ui = ui_handle.clone();
        move |tool_id| {
            println!("Tool clicked: {}", tool_id);
            if let Some(ui) = ui.upgrade() {
                // Get tool info
                if let Some(tool) = tools::get_tool_by_id(&tool_id) {
                    let installed = tools::is_tool_installed(tool);
                    let version = tools::get_installed_version(tool);
                    let folder = tools::get_tool_path(tool).to_string_lossy().to_string();

                    // Update popup properties
                    ui.set_popup_tool_id(tool_id.clone());
                    ui.set_popup_tool_name(tool.display_name.into());
                    ui.set_popup_tool_description(tool.description.into());
                    ui.set_popup_tool_installed(installed);
                    ui.set_popup_tool_status(
                        if installed {
                            version.unwrap_or_else(|| "Installed".to_string()).into()
                        } else {
                            "Not installed".into()
                        }
                    );
                    ui.set_popup_tool_folder(folder.into());
                    ui.set_popup_download_progress(-1);

                    // Show popup
                    ui.set_show_tool_popup(true);
                    ui.set_status_text(format!("{} - Tool Launcher", tool.display_name).into());
                }
            }
        }
    });

    // Callback: Launch tool button clicked
    ui.on_tool_launch_clicked({
        let ui = ui_handle.clone();
        move |tool_id| {
            println!("Launch clicked: {}", tool_id);
            if let Some(tool) = tools::get_tool_by_id(&tool_id) {
                match tools::launch_tool(tool) {
                    Ok(_) => {
                        println!("Launched {}", tool.display_name);
                        if let Some(ui) = ui.upgrade() {
                            ui.set_show_tool_popup(false);
                            ui.set_status_text(format!("Launched {}", tool.display_name).into());
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to launch: {}", e);
                        if let Some(ui) = ui.upgrade() {
                            ui.set_status_text(format!("Error: {}", e).into());
                        }
                    }
                }
            }
        }
    });

    // Callback: Download tool button clicked
    // NOTE: We capture the tool_id so that progress updates only apply to the
    // popup that started the download. If the user closes the popup and opens
    // a different tool, the background download still finishes (status bar
    // updates), but progress won't bleed into the wrong popup.
    ui.on_tool_download_clicked({
        let ui = ui_handle.clone();
        move |tool_id| {
            println!("Download clicked: {}", tool_id);
            if let Some(tool) = tools::get_tool_by_id(&tool_id) {
                if let Some(ui) = ui.upgrade() {
                    ui.set_status_text(format!("Downloading {}...", tool.display_name).into());
                    ui.set_popup_download_progress(0);
                }

                // Capture which tool started this download — used to guard UI updates
                let started_tool_id: String = tool_id.to_string();

                // Clone tool for the closure
                let tool_clone = tool.clone();
                let ui_for_download = ui.clone();
                let ui_for_progress = ui.clone();

                // Run download in a separate thread
                let progress_tool_id = started_tool_id.clone();
                std::thread::spawn(move || {
                    let result = tools::download_tool(&tool_clone, |progress| {
                        // Update progress in UI from the download thread
                        let ui_progress = ui_for_progress.clone();
                        let tid = progress_tool_id.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_progress.upgrade() {
                                // Only update popup progress if the popup is still
                                // showing the tool that started this download
                                if ui.get_popup_tool_id().as_str() == tid {
                                    ui.set_popup_download_progress(progress as i32);
                                }
                            }
                        });
                    });

                    // Update UI after download completes
                    let completion_tool_id = started_tool_id;
                    slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_for_download.upgrade() {
                                // Only touch popup state if the popup still shows this tool
                                let popup_matches =
                                    ui.get_popup_tool_id().as_str() == completion_tool_id;

                                if popup_matches {
                                    ui.set_popup_download_progress(-1);
                                }

                                match result {
                                    Ok(_) => {
                                        // Refresh popup status only if it's still our tool
                                        if popup_matches {
                                            let installed = tools::is_tool_installed(&tool_clone);
                                            let version = tools::get_installed_version(&tool_clone);
                                            ui.set_popup_tool_installed(installed);
                                            ui.set_popup_tool_status(
                                                version.unwrap_or_else(|| "Installed".to_string()).into()
                                            );
                                        }
                                        // Always update status bar so user sees completion
                                        ui.set_status_text(
                                            format!("{} downloaded successfully", tool_clone.display_name).into()
                                        );
                                    }
                                    Err(e) => {
                                        ui.set_status_text(format!("Download failed: {}", e).into());
                                    }
                                }
                        }
                    }).ok();
                });
            }
        }
    });

    // Callback: Open tool folder button clicked
    ui.on_tool_open_folder_clicked({
        move |tool_id| {
            println!("Open folder clicked: {}", tool_id);
            if let Some(tool) = tools::get_tool_by_id(&tool_id) {
                if let Err(e) = tools::open_tool_folder(tool) {
                    eprintln!("Failed to open folder: {}", e);
                }
            }
        }
    });

    // Callback: Download All backup tools
    // Downloads every tool sequentially in a background thread.
    // Updates a counter ("1/5", "2/5", ...) so the user can see progress.
    ui.on_download_all_clicked({
        let ui = ui_handle.clone();
        move || {
            println!("Download All clicked");

            // Mark the button as active immediately
            if let Some(ui) = ui.upgrade() {
                ui.set_download_all_active(true);
                ui.set_download_all_progress("0/5".into());
                ui.set_status_text("Downloading all backup tools...".into());
            }

            // Clone UI handle for the background thread
            let ui_for_thread = ui.clone();

            // Spawn one background thread that downloads tools one-by-one
            std::thread::spawn(move || {
                let all_tools = tools::get_all_tools();
                let total = all_tools.len();
                let mut success_count = 0;
                let mut fail_count = 0;

                for (index, tool) in all_tools.iter().enumerate() {
                    let tool_name = tool.display_name.to_string();
                    let counter = format!("{}/{}", index + 1, total);

                    // Update counter in UI before starting this tool
                    let ui_counter = ui_for_thread.clone();
                    let counter_clone = counter.clone();
                    let name_clone = tool_name.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_counter.upgrade() {
                            ui.set_download_all_progress(counter_clone.into());
                            ui.set_status_text(
                                format!("Downloading {} ({})...", name_clone, counter).into()
                            );
                        }
                    });

                    // Skip tools that are already installed
                    if tools::is_tool_installed(tool) {
                        println!("  {} already installed, skipping", tool.display_name);
                        success_count += 1;
                        continue;
                    }

                    // Download this tool (progress per-tool is not shown in the
                    // button — we just show the counter — but we still pass a
                    // no-op callback so the download function works normally)
                    let tool_owned = (*tool).clone();
                    match tools::download_tool(&tool_owned, |_percent| {
                        // Individual tool progress intentionally ignored here;
                        // the button shows "Downloading 2/5..." instead
                    }) {
                        Ok(_) => {
                            println!("  {} downloaded OK", tool.display_name);
                            success_count += 1;
                        }
                        Err(e) => {
                            eprintln!("  {} download failed: {}", tool.display_name, e);
                            fail_count += 1;
                        }
                    }
                }

                // All done — update UI on the main thread
                let ui_final = ui_for_thread.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_final.upgrade() {
                        ui.set_download_all_active(false);
                        ui.set_download_all_progress("".into());

                        if fail_count == 0 {
                            ui.set_status_text(
                                format!("All {} tools downloaded successfully", total).into()
                            );
                        } else {
                            ui.set_status_text(
                                format!("{} downloaded, {} failed", success_count, fail_count).into()
                            );
                        }
                    }
                });
            });
        }
    });

    // ============================================
    // PRODUCT KEY BACKUP CALLBACKS
    // ============================================
    // These handle detecting, copying, and saving the Windows product key
    // from the current machine (live Windows). The saved key can then be
    // loaded in the Deploy section (even from WinPE after reboot).

    // Callback: Detect Key — runs PowerShell to find OEM + installed keys
    ui.on_backup_detect_key({
        let ui = ui_handle.clone();
        move || {
            println!("Backup: Detect Windows product key");

            // Show detecting state immediately
            if let Some(ui) = ui.upgrade() {
                ui.set_backup_key_detecting(true);
                ui.set_status_text("Detecting Windows product key...".into());
            }

            // Run detection in background thread (PowerShell can take a few seconds)
            let ui_bg = ui.clone();
            std::thread::spawn(move || {
                let result = deploy::detect_windows_keys();

                // Update UI with results on the main thread
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_bg.upgrade() {
                        ui.set_backup_key_detecting(false);
                        match result {
                            Ok(info) => {
                                ui.set_backup_oem_key(info.oem_key.clone().into());
                                ui.set_backup_installed_key(info.installed_key.clone().into());
                                ui.set_backup_key_edition(info.edition.clone().into());
                                ui.set_backup_key_status(info.status.clone().into());
                                // Build a summary message
                                let found = if !info.oem_key.is_empty() && !info.installed_key.is_empty() {
                                    "Found OEM key and installed key"
                                } else if !info.installed_key.is_empty() {
                                    "Found installed key"
                                } else if !info.oem_key.is_empty() {
                                    "Found OEM key"
                                } else {
                                    "No product keys detected"
                                };
                                ui.set_status_text(found.into());
                            }
                            Err(e) => {
                                ui.set_status_text(format!("Key detection failed: {}", e).into());
                            }
                        }
                    }
                });
            });
        }
    });

    // Callback: Copy Key — copies a key string to the clipboard
    ui.on_backup_copy_key({
        let ui = ui_handle.clone();
        move |key| {
            let key_str = key.to_string();
            println!("Backup: Copy key to clipboard");
            match arboard::Clipboard::new() {
                Ok(mut clipboard) => {
                    match clipboard.set_text(&key_str) {
                        Ok(()) => {
                            if let Some(ui) = ui.upgrade() {
                                ui.set_status_text("Key copied to clipboard".into());
                            }
                        }
                        Err(e) => {
                            if let Some(ui) = ui.upgrade() {
                                ui.set_status_text(format!("Failed to copy: {}", e).into());
                            }
                        }
                    }
                }
                Err(e) => {
                    if let Some(ui) = ui.upgrade() {
                        ui.set_status_text(format!("Clipboard unavailable: {}", e).into());
                    }
                }
            }
        }
    });

    // ============================================
    // MULTI-KEY MANAGEMENT CALLBACKS
    // ============================================
    // saved_keys.json stores an array of WindowsKeyInfo entries — one per machine.
    // The Deploy page has a ComboBox to select which machine's key to load.

    /// Helper: refresh the saved keys ComboBox in the UI.
    /// Reads saved_keys.json and updates the ComboBox model with "HOSTNAME (date)" labels.
    fn refresh_saved_keys_ui(ui: &MainWindow) {
        let keys = deploy::load_saved_keys();
        let labels = deploy::format_saved_key_labels(&keys);

        // Convert to a Slint string model for the ComboBox
        let model = std::rc::Rc::new(slint::VecModel::from(
            labels.iter().map(|s| slint::SharedString::from(s.as_str())).collect::<Vec<_>>()
        ));
        ui.set_deploy_saved_key_labels(model.into());

        // Reset selection to first item if any keys exist, or -1 if empty
        if keys.is_empty() {
            ui.set_deploy_saved_key_index(-1);
        } else if ui.get_deploy_saved_key_index() < 0 || ui.get_deploy_saved_key_index() >= keys.len() as i32 {
            ui.set_deploy_saved_key_index(0);
        }
    }

    // On startup, populate the saved keys ComboBox
    if let Some(ui) = ui_handle.upgrade() {
        refresh_saved_keys_ui(&ui);
    }

    // Callback: Save Key — writes detected keys to saved_keys.json next to EXE
    // Now supports multiple keys: appends/updates by hostname, then refreshes the ComboBox.
    ui.on_backup_save_key({
        let ui = ui_handle.clone();
        move || {
            println!("Backup: Save key to file");
            if let Some(ui) = ui.upgrade() {
                // Build the key info struct from the UI properties
                let info = deploy::WindowsKeyInfo {
                    oem_key: ui.get_backup_oem_key().to_string(),
                    installed_key: ui.get_backup_installed_key().to_string(),
                    edition: ui.get_backup_key_edition().to_string(),
                    status: ui.get_backup_key_status().to_string(),
                    hostname: {
                        // Quick hostname detection
                        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Unknown".to_string())
                    },
                    date: {
                        // Get current date
                        let now = std::time::SystemTime::now();
                        let duration = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                        let secs = duration.as_secs();
                        let days = secs / 86400;
                        let years = 1970 + (days / 365);
                        let remaining_days = days % 365;
                        let month = remaining_days / 30 + 1;
                        let day = remaining_days % 30 + 1;
                        format!("{}-{:02}-{:02}", years, month.min(12), day.min(31))
                    },
                };

                match deploy::save_keys_to_file(&info) {
                    Ok(()) => {
                        ui.set_backup_key_saved_info(
                            format!("Saved to saved_keys.json (from {} on {})",
                                info.hostname, info.date).into());
                        ui.set_status_text("Product key saved to file".into());
                        // Refresh the Deploy page ComboBox so the new key appears immediately
                        refresh_saved_keys_ui(&ui);
                    }
                    Err(e) => {
                        ui.set_status_text(format!("Failed to save key: {}", e).into());
                    }
                }
            }
        }
    });

    // Callback: Load Saved Key At Index — reads the key at the given ComboBox index
    // and fills the deploy product key field.
    ui.on_deploy_load_saved_key_at({
        let ui = ui_handle.clone();
        move |index: i32| {
            println!("Deploy: Load saved key at index {}", index);
            if let Some(ui) = ui.upgrade() {
                let keys = deploy::load_saved_keys();

                if index < 0 || index as usize >= keys.len() {
                    ui.set_status_text("No saved key selected. Use Backup page to detect and save first.".into());
                    return;
                }

                let info = &keys[index as usize];

                // Prefer the installed key (this is the active key the user is using).
                // Fall back to OEM key if no installed key was detected.
                let key_to_use = if !info.installed_key.is_empty() {
                    info.installed_key.clone()
                } else if !info.oem_key.is_empty() {
                    info.oem_key.clone()
                } else {
                    String::new()
                };

                if !key_to_use.is_empty() {
                    ui.set_deploy_product_key(key_to_use.into());
                    ui.set_status_text(format!("Loaded key from {} (backed up {})",
                        info.hostname, info.date).into());
                } else {
                    ui.set_status_text("Selected key entry has no product key stored".into());
                }
            }
        }
    });

    // Callback: Delete Saved Key — removes the selected key entry from saved_keys.json
    ui.on_deploy_delete_saved_key({
        let ui = ui_handle.clone();
        move |index: i32| {
            println!("Deploy: Delete saved key at index {}", index);
            if let Some(ui) = ui.upgrade() {
                let keys = deploy::load_saved_keys();

                if index < 0 || index as usize >= keys.len() {
                    ui.set_status_text("No saved key selected to delete".into());
                    return;
                }

                let hostname = keys[index as usize].hostname.clone();

                match deploy::delete_saved_key(&hostname) {
                    Ok(true) => {
                        ui.set_status_text(format!("Deleted saved key for '{}'", hostname).into());
                        // Refresh the ComboBox to reflect the deletion
                        refresh_saved_keys_ui(&ui);
                    }
                    Ok(false) => {
                        ui.set_status_text(format!("Key for '{}' not found", hostname).into());
                    }
                    Err(e) => {
                        ui.set_status_text(format!("Failed to delete key: {}", e).into());
                    }
                }
            }
        }
    });

    // Callback: Refresh Saved Keys — re-reads saved_keys.json and updates the ComboBox
    ui.on_deploy_refresh_saved_keys({
        let ui = ui_handle.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                refresh_saved_keys_ui(&ui);
            }
        }
    });

    // ============================================
    // WINPE BUILDER CALLBACKS
    // ============================================

    // Callback: Detect WinRE button clicked
    // Also runs comprehensive dependency check for PE building
    ui.on_pe_detect_winre({
        let ui = ui_handle.clone();
        move || {
            println!("Detecting WinRE and ADK...");

            // Show immediate feedback - set detecting state
            if let Some(ui) = ui.upgrade() {
                ui.set_pe_detecting(true);
                ui.set_status_text("Detecting WinRE, ADK, and dependencies...".into());
                ui.set_deps_status("Detecting...".into());
            }

            // Detect WinRE
            let winre_info = winpe::detect_winre();

            // Detect ADK
            let adk_info = winpe::detect_adk();

            // Run comprehensive dependency check
            println!("Running dependency check...");
            let deps = winpe::check_pe_build_dependencies();

            // Print dependency check results to console
            println!("=== Dependency Check Results ===");
            println!("ADK Installed: {} ({})", deps.adk_installed, deps.adk_path);
            println!("WinPE Add-on: {} ({})", deps.winpe_addon_installed, deps.winpe_addon_path);
            println!("oscdimg: {} ({})", deps.oscdimg_available, deps.oscdimg_path);
            println!("7-Zip: {} ({})", deps.seven_zip_available, deps.seven_zip_path);
            println!("DISM: {}", deps.dism_available);
            println!("PowerShell: {}", deps.powershell_available);
            println!("Disk Space OK: {} ({:.1} GB)", deps.disk_space_ok, deps.disk_space_gb);
            println!("All Satisfied: {}", deps.all_satisfied);
            if !deps.errors.is_empty() {
                println!("Errors: {:?}", deps.errors);
            }
            if !deps.warnings.is_empty() {
                println!("Warnings: {:?}", deps.warnings);
            }
            println!("================================");

            // Update UI
            if let Some(ui) = ui.upgrade() {
                // Update WinRE status
                ui.set_winre_found(winre_info.found);
                if winre_info.found {
                    ui.set_winre_path(winre_info.path.to_string_lossy().to_string().into());
                    ui.set_winre_size(winre_info.size_display.into());
                } else {
                    ui.set_winre_path("".into());
                    ui.set_winre_size("".into());
                }

                // Update ADK status (from dependency check - more comprehensive)
                ui.set_adk_found(deps.adk_installed);
                if deps.adk_installed {
                    ui.set_adk_version(adk_info.version.into());
                    ui.set_adk_path(deps.adk_path.clone().into());
                } else {
                    ui.set_adk_version("".into());
                    ui.set_adk_path("".into());
                }

                // Update WinPE Add-on status
                ui.set_winpe_addon_found(deps.winpe_addon_installed);
                ui.set_winpe_addon_path(deps.winpe_addon_path.clone().into());

                // Update other dependencies
                ui.set_oscdimg_found(deps.oscdimg_available);
                ui.set_oscdimg_path(deps.oscdimg_path.clone().into());
                ui.set_seven_zip_found(deps.seven_zip_available);
                ui.set_seven_zip_path(deps.seven_zip_path.clone().into());
                ui.set_dism_found(deps.dism_available);
                ui.set_powershell_found(deps.powershell_available);
                ui.set_disk_space_ok(deps.disk_space_ok);
                ui.set_disk_space_gb(deps.disk_space_gb as f32);
                ui.set_all_deps_satisfied(deps.all_satisfied);

                // Build status message
                let status_msg = if deps.all_satisfied {
                    if winre_info.found {
                        "All dependencies satisfied. Ready to build!".to_string()
                    } else {
                        "All dependencies satisfied. Select a Windows ISO to build PE.".to_string()
                    }
                } else {
                    // Show first error
                    if !deps.errors.is_empty() {
                        deps.errors[0].clone()
                    } else {
                        "Missing dependencies - cannot build PE".to_string()
                    }
                };
                ui.set_deps_status(status_msg.clone().into());
                ui.set_status_text(status_msg.into());

                // Set default output path if not already set
                let current_output: String = ui.get_pe_output_path().to_string();
                if current_output.is_empty() {
                    let default_path = winpe::get_default_output_path();
                    ui.set_pe_output_path(default_path.to_string_lossy().to_string().into());
                }

                // Scan PE tools to update status dots (green/orange)
                update_pe_tool_status(&ui, 0);

                // Detection complete
                ui.set_pe_detecting(false);
            }
        }
    });

    // Callback: Install Dependencies button clicked
    // Attempts to install ALL missing dependencies (7-Zip, ADK, WinPE Add-on)
    // Uses winget when available, falls back to opening browser for manual download
    ui.on_pe_install_adk({
        let ui = ui_handle.clone();
        move || {
            println!("Install Dependencies clicked");

            // Clone UI handle for thread
            let ui_for_install = ui.clone();

            if let Some(ui) = ui.upgrade() {
                ui.set_status_text("Starting dependency installation...".into());
                ui.set_deps_status("Installing dependencies...".into());
                ui.set_pe_detecting(true);  // Show busy indicator
            }

            // Run installation in a separate thread to avoid blocking UI
            std::thread::spawn(move || {
                // Update UI: Installing 7-Zip
                let ui_7zip = ui_for_install.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_7zip.upgrade() {
                        ui.set_status_text("Installing 7-Zip...".into());
                        ui.set_deps_status("Step 1/3: Installing 7-Zip...".into());
                    }
                });

                // Install 7-Zip
                let seven_zip_result = winpe::install_7zip();
                println!("7-Zip result: {:?}", seven_zip_result);
                let seven_zip_ok = seven_zip_result.success;
                let seven_zip_method = seven_zip_result.method.clone();

                // Update UI: Installing ADK (this can take a while)
                let ui_adk = ui_for_install.clone();
                let seven_zip_status = if seven_zip_ok { "OK" } else { &seven_zip_method };
                let adk_status_msg = format!("7-Zip: {} | Installing Windows ADK (please wait)...", seven_zip_status);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_adk.upgrade() {
                        ui.set_status_text(adk_status_msg.into());
                        ui.set_deps_status("Step 2/3: Installing Windows ADK (this takes time)...".into());
                    }
                });

                // Install ADK (includes waiting for installation to complete)
                let adk_result = winpe::install_adk();
                println!("ADK result: {:?}", adk_result);
                let adk_ok = adk_result.success;
                let adk_method = adk_result.method.clone();

                // Update UI: Installing WinPE Add-on (with retries)
                let ui_winpe = ui_for_install.clone();
                let seven_zip_status2 = if seven_zip_ok { "OK" } else { &seven_zip_method };
                let adk_status = if adk_ok { "OK" } else { &adk_method };
                let winpe_status_msg = format!("7-Zip: {} | ADK: {} | Installing WinPE Add-on (with retries)...", seven_zip_status2, adk_status);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_winpe.upgrade() {
                        ui.set_status_text(winpe_status_msg.into());
                        ui.set_deps_status("Step 3/3: Installing WinPE Add-on (may retry if ADK not ready)...".into());
                    }
                });

                // Install WinPE Add-on (includes retries if ADK not ready)
                let winpe_result = winpe::install_winpe_addon();
                println!("WinPE Add-on result: {:?}", winpe_result);
                let winpe_ok = winpe_result.success;
                let winpe_method = winpe_result.method.clone();

                // Final UI update
                let ui_final = ui_for_install.clone();
                let all_success = seven_zip_ok && adk_ok && winpe_ok;

                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_final.upgrade() {
                        ui.set_pe_detecting(false);

                        if all_success {
                            ui.set_status_text("All dependencies installed successfully!".into());
                            ui.set_deps_status("Installation complete - click Detect to verify".into());
                            ui.set_all_deps_satisfied(true);
                        } else {
                            // Build summary of what worked and what didn't
                            let seven_zip_str = if seven_zip_ok { "OK".to_string() } else { seven_zip_method };
                            let adk_str = if adk_ok { "OK".to_string() } else { adk_method };
                            let winpe_str = if winpe_ok { "OK".to_string() } else { winpe_method };

                            let status = format!("7-Zip: {} | ADK: {} | WinPE: {} | Click Detect to verify",
                                seven_zip_str, adk_str, winpe_str);
                            ui.set_status_text(status.into());
                            ui.set_deps_status("Some components may need manual installation".into());
                        }
                    }
                });
            });
        }
    });

    // Callback: Browse for ISO source
    // Used for both ISO_PE and ISO_RE modes - stores path in iso_path (separate from winre_path)
    ui.on_pe_browse_source({
        let ui = ui_handle.clone();
        move || {
            println!("Browse for ISO source clicked");

            // Open file picker for ISO
            if let Some(iso_path) = winpe::pick_iso_file() {
                println!("Selected ISO: {}", iso_path.display());

                if let Some(ui) = ui.upgrade() {
                    // Analyze the ISO to verify it's valid
                    match winpe::analyze_iso(&iso_path) {
                        Ok(info) => {
                            if info.has_boot_wim {
                                // Valid Windows ISO with boot.wim
                                // Store in iso_path (NOT winre_path - those are separate now)
                                ui.set_iso_path(iso_path.to_string_lossy().to_string().into());
                                ui.set_iso_selected(true);
                                ui.set_iso_size(info.size_display.into());
                                ui.set_status_text(format!(
                                    "Windows ISO selected: {} (boot.wim found)",
                                    iso_path.file_name().unwrap_or_default().to_string_lossy()
                                ).into());
                            } else {
                                ui.set_status_text("Invalid Windows ISO - no boot.wim found".into());
                            }
                        }
                        Err(e) => {
                            ui.set_status_text(format!("Error analyzing ISO: {}", e).into());
                        }
                    }
                }
            } else {
                println!("ISO selection cancelled");
            }
        }
    });

    // Callback: Browse for output path
    ui.on_pe_browse_output({
        let ui = ui_handle.clone();
        move || {
            println!("Browse for output path clicked");

            // Open save file dialog
            if let Some(output_path) = winpe::pick_output_path() {
                println!("Selected output: {}", output_path.display());

                if let Some(ui) = ui.upgrade() {
                    ui.set_pe_output_path(output_path.to_string_lossy().to_string().into());
                    ui.set_status_text(format!("Output path: {}", output_path.display()).into());
                }
            } else {
                println!("Output selection cancelled");
                // Set default if nothing selected
                if let Some(ui) = ui.upgrade() {
                    let current: String = ui.get_pe_output_path().to_string();
                    if current.is_empty() {
                        let default_path = winpe::get_default_output_path();
                        ui.set_pe_output_path(default_path.to_string_lossy().to_string().into());
                    }
                }
            }
        }
    });

    // Callback: Build button clicked (works for all modes: Local RE, ISO PE, ISO RE)
    ui.on_pe_build({
        let ui = ui_handle.clone();
        move || {
            println!("Build ISO clicked");

            if let Some(ui) = ui.upgrade() {
                // Get current settings
                let source_type = ui.get_pe_source().to_string();
                let output_path_str: String = ui.get_pe_output_path().to_string();
                // Drivers, tools, and network are always included.
                // Disabling any of these produces broken PE images, so there's
                // no UI toggle — they're hardcoded to true.
                let include_drivers = true;
                let include_tools = true;

                // Get the appropriate source path based on source type
                let winre_path_str: String = ui.get_winre_path().to_string();
                let iso_path_str: String = ui.get_iso_path().to_string();

                // ============================================
                // READ ADK PACKAGE TOGGLES FROM UI
                // ============================================
                // Get master toggles for packages and fixes
                let install_packages = ui.get_pe_install_packages();
                let apply_fixes = ui.get_pe_apply_fixes();

                // Read individual package toggles
                // Each package maps to an ID in the adk_packages module
                let pkg_wmi = ui.get_pe_pkg_wmi();
                let pkg_netfx = ui.get_pe_pkg_netfx();
                let pkg_scripting = ui.get_pe_pkg_scripting();
                let pkg_powershell = ui.get_pe_pkg_powershell();
                let pkg_dism_cmdlets = ui.get_pe_pkg_dism_cmdlets();
                let pkg_secureboot_cmdlets = ui.get_pe_pkg_secureboot_cmdlets();
                let pkg_storage_wmi = ui.get_pe_pkg_storage_wmi();
                let pkg_enhanced_storage = ui.get_pe_pkg_enhanced_storage();
                let pkg_fmapi = ui.get_pe_pkg_fmapi();
                let pkg_dot3svc = ui.get_pe_pkg_dot3svc();
                let pkg_secure_startup = ui.get_pe_pkg_secure_startup();
                let pkg_hta = ui.get_pe_pkg_hta();
                let pkg_winrecfg = ui.get_pe_pkg_winrecfg();
                let pkg_font_support = ui.get_pe_pkg_font_support();
                let pkg_platform_id = ui.get_pe_pkg_platform_id();
                let pkg_wds_tools = ui.get_pe_pkg_wds_tools();

                // New packages (added for expanded PE builder)
                let pkg_wifi = ui.get_pe_pkg_wifi();
                let pkg_pppoe = ui.get_pe_pkg_pppoe();
                let pkg_rndis = ui.get_pe_pkg_rndis();
                let pkg_hsp_driver = ui.get_pe_pkg_hsp_driver();
                let pkg_rejuv = ui.get_pe_pkg_rejuv();
                let pkg_srt = ui.get_pe_pkg_srt();
                let pkg_setup = ui.get_pe_pkg_setup();
                let pkg_setup_client = ui.get_pe_pkg_setup_client();
                let pkg_setup_server = ui.get_pe_pkg_setup_server();
                let pkg_legacy_setup = ui.get_pe_pkg_legacy_setup();
                let pkg_mdac = ui.get_pe_pkg_mdac();
                let pkg_fonts_legacy = ui.get_pe_pkg_fonts_legacy();
                let pkg_fonts_japanese = ui.get_pe_pkg_fonts_japanese();
                let pkg_fonts_korean = ui.get_pe_pkg_fonts_korean();
                let pkg_fonts_chinese_simplified = ui.get_pe_pkg_fonts_chinese_simplified();
                let pkg_fonts_chinese_traditional = ui.get_pe_pkg_fonts_chinese_traditional();
                let pkg_fonts_chinese_hk = ui.get_pe_pkg_fonts_chinese_hk();
                let pkg_gaming_peripherals = ui.get_pe_pkg_gaming_peripherals();

                // Read new output options
                let output_type = ui.get_pe_output_type().to_string();
                let use_uefi_2023_ca = ui.get_pe_use_uefi_2023_ca();
                let backup_original = ui.get_pe_backup_original();
                let default_shell = ui.get_pe_default_shell().to_string();

                // ============================================
                // READ PE FIX TOGGLES FROM UI
                // ============================================
                let fix_dpi_scaling = ui.get_pe_fix_dpi_scaling();
                let fix_wallpaper_host = ui.get_pe_fix_wallpaper_host();
                let fix_font_fix = ui.get_pe_fix_font_fix();
                let fix_crash_dialogs = ui.get_pe_fix_crash_dialogs();
                let fix_long_paths = ui.get_pe_fix_long_paths();

                // ============================================
                // READ PE TOOL TOGGLES FROM UI
                // ============================================
                // These control which individual PE tools get injected
                let tool_winxshell = ui.get_pe_tool_winxshell();
                let tool_explorer = ui.get_pe_tool_explorer();
                let tool_penetwork = ui.get_pe_tool_penetwork();
                let tool_crystaldisk = ui.get_pe_tool_crystaldisk();
                let tool_7zip = ui.get_pe_tool_7zip();
                let tool_autoruns = ui.get_pe_tool_autoruns();
                // New tools from pcassistsoftware.co.uk
                let tool_diskcheck = ui.get_pe_tool_diskcheck();
                let tool_dismtool = ui.get_pe_tool_dismtool();
                let tool_webbrowser = ui.get_pe_tool_webbrowser();
                let tool_eventviewer = ui.get_pe_tool_eventviewer();
                let tool_installedsw = ui.get_pe_tool_installedsw();
                let tool_fileexplorer = ui.get_pe_tool_fileexplorer();

                println!("Source Type: {}", source_type);
                println!("Output: {}", output_path_str);
                println!("Drivers: {}, Tools: {}", include_drivers, include_tools);
                println!("Install Packages: {}, Apply Fixes: {}", install_packages, apply_fixes);

                // Validate configuration
                if output_path_str.is_empty() {
                    ui.set_status_text("Please select an output path first".into());
                    return;
                }

                // Determine source path based on selected mode
                let source_path_str = match source_type.as_str() {
                    "LocalRE" => {
                        if winre_path_str.is_empty() {
                            ui.set_status_text("Local WinRE not detected. Click 'Detect' first or select an ISO.".into());
                            return;
                        }
                        winre_path_str.clone()
                    }
                    "ISO_PE" | "ISO_RE" => {
                        if iso_path_str.is_empty() {
                            ui.set_status_text("No Windows ISO selected. Click 'ISO → WinPE' or 'ISO → WinRE' to select one.".into());
                            return;
                        }
                        iso_path_str.clone()
                    }
                    _ => {
                        ui.set_status_text("Please select a source type first".into());
                        return;
                    }
                };

                println!("Using source: {}", source_path_str);
                let source_path = std::path::PathBuf::from(&source_path_str);

                if !source_path.exists() {
                    ui.set_status_text("Source file not found. Select a valid ISO or detect WinRE.".into());
                    return;
                }

                // ============================================
                // UPDATE PE TOOLS CONFIG FROM UI CHECKBOXES
                // ============================================
                // Save the individual tool selections to the config file
                // so the build process knows which tools to include
                if include_tools {
                    println!("Updating PE tools configuration from UI...");
                    // Map UI checkbox names to tool names in pe_tools folder
                    // These names MUST match the "name" field in each tool.toml
                    let tool_selections = [
                        ("WinXShell", tool_winxshell),
                        ("Explorer++", tool_explorer),
                        ("PENetwork", tool_penetwork),
                        ("CrystalDiskInfo", tool_crystaldisk),
                        ("7-Zip", tool_7zip),
                        ("Autoruns", tool_autoruns),
                        ("Disk Check", tool_diskcheck),
                        ("DISM Tool", tool_dismtool),
                        ("Web Browser", tool_webbrowser),
                        ("Event Viewer", tool_eventviewer),
                        ("Installed Software", tool_installedsw),
                        ("File Explorer", tool_fileexplorer),
                    ];

                    for (tool_name, enabled) in &tool_selections {
                        if let Err(e) = tools::pe_tools::set_pe_tool_enabled(tool_name, *enabled) {
                            println!("Warning: Failed to update {} setting: {}", tool_name, e);
                        } else {
                            println!("  {} = {}", tool_name, if *enabled { "enabled" } else { "disabled" });
                        }
                    }
                }

                // Start the build
                ui.set_pe_building(true);
                ui.set_pe_build_progress(0);
                ui.set_pe_build_status("Starting build...".into());

                // ============================================
                // BUILD ENABLED PACKAGES LIST FROM UI TOGGLES
                // ============================================
                // Map each UI toggle to its package ID
                let mut enabled_packages: Vec<String> = Vec::new();
                if pkg_wmi { enabled_packages.push("wmi".to_string()); }
                if pkg_netfx { enabled_packages.push("netfx".to_string()); }
                if pkg_scripting { enabled_packages.push("scripting".to_string()); }
                if pkg_powershell { enabled_packages.push("powershell".to_string()); }
                if pkg_dism_cmdlets { enabled_packages.push("dism_cmdlets".to_string()); }
                if pkg_secureboot_cmdlets { enabled_packages.push("secureboot_cmdlets".to_string()); }
                if pkg_storage_wmi { enabled_packages.push("storage_wmi".to_string()); }
                if pkg_enhanced_storage { enabled_packages.push("enhanced_storage".to_string()); }
                if pkg_fmapi { enabled_packages.push("fmapi".to_string()); }
                if pkg_dot3svc { enabled_packages.push("dot3svc".to_string()); }
                if pkg_secure_startup { enabled_packages.push("secure_startup".to_string()); }
                if pkg_hta { enabled_packages.push("hta".to_string()); }
                if pkg_winrecfg { enabled_packages.push("winrecfg".to_string()); }
                if pkg_font_support { enabled_packages.push("font_support".to_string()); }
                if pkg_platform_id { enabled_packages.push("platform_id".to_string()); }
                if pkg_wds_tools { enabled_packages.push("wds_tools".to_string()); }

                // New packages
                // NOTE: pkg_wifi controls inject_wifi_support() in winpe.rs, not an ADK package
                // (WinPE-WiFi-Package doesn't exist as a standalone ADK .cab)
                if pkg_pppoe { enabled_packages.push("pppoe".to_string()); }
                if pkg_rndis { enabled_packages.push("rndis".to_string()); }
                if pkg_hsp_driver { enabled_packages.push("hsp_driver".to_string()); }
                if pkg_rejuv { enabled_packages.push("rejuv".to_string()); }
                if pkg_srt { enabled_packages.push("srt".to_string()); }
                if pkg_setup { enabled_packages.push("setup".to_string()); }
                if pkg_setup_client { enabled_packages.push("setup_client".to_string()); }
                if pkg_setup_server { enabled_packages.push("setup_server".to_string()); }
                if pkg_legacy_setup { enabled_packages.push("legacy_setup".to_string()); }
                if pkg_mdac { enabled_packages.push("mdac".to_string()); }
                if pkg_fonts_legacy { enabled_packages.push("fonts_legacy".to_string()); }
                if pkg_fonts_japanese { enabled_packages.push("fonts_japanese".to_string()); }
                if pkg_fonts_korean { enabled_packages.push("fonts_korean".to_string()); }
                if pkg_fonts_chinese_simplified { enabled_packages.push("fonts_chinese_simplified".to_string()); }
                if pkg_fonts_chinese_traditional { enabled_packages.push("fonts_chinese_traditional".to_string()); }
                if pkg_fonts_chinese_hk { enabled_packages.push("fonts_chinese_hk".to_string()); }
                if pkg_gaming_peripherals { enabled_packages.push("gaming_peripherals".to_string()); }

                // ============================================
                // BUILD ENABLED FIXES LIST FROM UI TOGGLES
                // ============================================
                // Map each UI toggle to its fix ID
                let mut enabled_fixes: Vec<String> = Vec::new();
                if fix_dpi_scaling { enabled_fixes.push("dpi_scaling".to_string()); }
                if fix_wallpaper_host { enabled_fixes.push("wallpaper_host".to_string()); }
                if fix_font_fix { enabled_fixes.push("font_fix".to_string()); }
                if fix_crash_dialogs { enabled_fixes.push("disable_crash_dialogs".to_string()); }
                if fix_long_paths { enabled_fixes.push("enable_long_paths".to_string()); }

                println!("Enabled packages: {:?}", enabled_packages);
                println!("Enabled fixes: {:?}", enabled_fixes);

                // ============================================
                // BUILD CONFIGURATION
                // ============================================
                // Create the configuration with all options from the UI
                let config = winpe::PeBuildConfig {
                    source_path,
                    output_path: std::path::PathBuf::from(&output_path_str),
                    architecture: "amd64".to_string(),
                    volume_label: "MASTERBOOTER".to_string(),

                    // New output options
                    output_type,
                    use_uefi_2023_ca,
                    backup_original,
                    default_shell,

                    include_drivers,
                    include_tools,
                    // Driver paths are auto-detected during build in winpe.rs STEP 4:
                    // - WiFi_Drivers.7z in pe_tools/ (extracted and injected via DISM)
                    // - User-provided Drivers/ folder next to the EXE
                    driver_paths: Vec::new(),
                    // WiFi support: extracts WLAN files from ISO's install.wim into PE
                    // Network support is always included — WiFi depends only on the package toggle
                    enable_wifi: pkg_wifi,
                    install_packages,
                    enabled_packages,
                    apply_fixes,
                    enabled_fixes,
                    fix_options: pe_fixes::FixOptions::default(),
                    dry_run: false,
                };

                // Clone UI handle for the build thread
                let ui_for_build = ui.as_weak();

                // Clone UI handle for progress updates
                let ui_for_progress = ui_for_build.clone();

                // Run build in a separate thread
                std::thread::spawn(move || {
                    // Disable Quick Edit mode on the console window.
                    // When Quick Edit is enabled (Windows default), clicking anywhere in
                    // the console window selects text and PAUSES the entire process.
                    // The user has to press Enter to resume — this causes the build to
                    // appear "stuck" at whatever progress % was showing when they clicked.
                    // We disable it here so accidental clicks don't freeze the build.
                    #[cfg(target_os = "windows")]
                    {
                        // Windows API constants for console mode
                        const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
                        const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
                        const STD_INPUT_HANDLE: u32 = 0xFFFFFFF6; // (DWORD)-10

                        extern "system" {
                            fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
                            fn GetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, lpMode: *mut u32) -> i32;
                            fn SetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, dwMode: u32) -> i32;
                        }

                        unsafe {
                            // Get the console input handle directly from Windows
                            let handle = GetStdHandle(STD_INPUT_HANDLE);
                            if !handle.is_null() {
                                let mut mode: u32 = 0;
                                if GetConsoleMode(handle, &mut mode) != 0 {
                                    // Remove Quick Edit, add Extended Flags (required for the change to take effect)
                                    mode &= !ENABLE_QUICK_EDIT_MODE;
                                    mode |= ENABLE_EXTENDED_FLAGS;
                                    let _ = SetConsoleMode(handle, mode);
                                    println!("Console Quick Edit mode disabled (prevents accidental build pauses)");
                                }
                            }
                        }
                    }

                    let result = winpe::build_pe_iso(&config, move |progress, status| {
                        // Update progress in UI
                        let ui_progress = ui_for_progress.clone();
                        let status_owned = status.to_string();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_progress.upgrade() {
                                ui.set_pe_build_progress(progress);
                                ui.set_pe_build_status(status_owned.into());
                            }
                        });
                    });

                    // Update UI after build completes
                    let ui_final = ui_for_build.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_final.upgrade() {
                            ui.set_pe_building(false);
                            ui.set_pe_build_progress(0);
                            if result.success {
                                ui.set_pe_build_status("Build complete!".into());
                                ui.set_status_text("WinPE ISO built successfully".into());
                            } else {
                                ui.set_pe_build_status("".into());
                                ui.set_status_text(result.message.into());
                            }
                        }
                    });
                });
            }
        }
    });

    // Callback: Open output folder button clicked
    ui.on_pe_open_output_folder({
        let ui = ui_handle.clone();
        move || {
            println!("Open output folder clicked");
            if let Some(ui) = ui.upgrade() {
                let output_path_str: String = ui.get_pe_output_path().to_string();
                if output_path_str.is_empty() {
                    // Open Documents folder as default
                    let default_path = winpe::get_default_output_path();
                    if let Err(e) = winpe::open_folder(&default_path) {
                        ui.set_status_text(format!("Error: {}", e).into());
                    }
                } else {
                    let path = std::path::Path::new(&output_path_str);
                    if let Err(e) = winpe::open_folder(path) {
                        ui.set_status_text(format!("Error: {}", e).into());
                    }
                }
            }
        }
    });

    // ============================================
    // PE TOOLS: Download All callback
    // ============================================
    // Downloads all enabled PE tools in a background thread.
    // Updates a counter ("1/6", "2/6", ...) and refreshes status dots when done.
    ui.on_pe_download_all_tools({
        let ui = ui_handle.clone();
        move || {
            println!("Download All PE Tools clicked");

            // ============================================
            // READ UI CHECKBOX STATES BEFORE SPAWNING THREAD
            // ============================================
            // We MUST read the UI state on the main thread (Slint requirement).
            // The discover_pe_tools() function reads "enabled" from pe_tools_config.json,
            // but the user controls which tools to download via the UI checkboxes.
            // So we read the checkbox states here and override the enabled flags.
            let ui_enabled: std::collections::HashMap<String, bool> = if let Some(ui) = ui.upgrade() {
                ui.set_pe_tools_download_active(true);
                ui.set_pe_tools_download_progress("0/0".into());
                ui.set_status_text("Downloading PE tools...".into());

                // Build a map of tool name -> enabled from UI checkboxes
                let mut map = std::collections::HashMap::new();
                map.insert("WinXShell".to_string(), ui.get_pe_tool_winxshell());
                map.insert("Explorer++".to_string(), ui.get_pe_tool_explorer());
                map.insert("PENetwork".to_string(), ui.get_pe_tool_penetwork());
                map.insert("CrystalDiskInfo".to_string(), ui.get_pe_tool_crystaldisk());
                map.insert("7-Zip".to_string(), ui.get_pe_tool_7zip());
                map.insert("Autoruns".to_string(), ui.get_pe_tool_autoruns());
                map.insert("Disk Check".to_string(), ui.get_pe_tool_diskcheck());
                map.insert("DISM Tool".to_string(), ui.get_pe_tool_dismtool());
                map.insert("Web Browser".to_string(), ui.get_pe_tool_webbrowser());
                map.insert("Event Viewer".to_string(), ui.get_pe_tool_eventviewer());
                map.insert("Installed Software".to_string(), ui.get_pe_tool_installedsw());
                map.insert("File Explorer".to_string(), ui.get_pe_tool_fileexplorer());
                map
            } else {
                return;
            };

            let enabled_count = ui_enabled.values().filter(|&&v| v).count();

            // Clone UI handle for the background thread
            let ui_for_thread = ui.clone();

            // Spawn background thread to download tools
            std::thread::spawn(move || {
                // Discover all PE tools on disk
                let mut tools = tools::pe_tools::discover_pe_tools();

                // Override the "enabled" flag with the UI checkbox states
                // This ensures only tools the user has checked get downloaded
                for tool in &mut tools {
                    if let Some(&ui_state) = ui_enabled.get(&tool.name) {
                        tool.enabled = ui_state;
                    }
                }

                println!("Download All: {} tools checked in UI", enabled_count);
                for tool in &tools {
                    println!("  {} - enabled: {}, present: {}, url: {}",
                        tool.name, tool.enabled, tool.is_present,
                        if tool.download_url.is_empty() { "(none)" } else { &tool.download_url });
                }

                // Download enabled tools that are not yet present
                let results = tools::pe_tools::download_enabled_pe_tools(
                    &tools,
                    |name, current, total, _pct| {
                        // Update the counter and status bar for each tool
                        let ui_progress = ui_for_thread.clone();
                        let counter = format!("{}/{}", current, total);
                        let status_msg = format!("Downloading PE tool {}/{}: {}", current, total, name);
                        let counter_clone = counter.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_progress.upgrade() {
                                ui.set_pe_tools_download_progress(counter_clone.into());
                                ui.set_status_text(status_msg.into());
                            }
                        });
                    },
                );

                // Count successes and failures
                let success_count = results.iter().filter(|r| r.success).count();
                let fail_count = results.iter().filter(|r| !r.success).count();
                let skipped = enabled_count.saturating_sub(success_count + fail_count);

                // After downloading, re-discover tools (to get updated is_present flags)
                // and copy all present tools to the GitHub staging folder.
                // This gives the user files to upload as GitHub release assets (fallback source).
                if success_count > 0 {
                    let refreshed_tools = tools::pe_tools::discover_pe_tools();
                    let (copied, copy_failed) = tools::pe_tools::copy_tools_to_github_staging(&refreshed_tools);
                    println!("GitHub staging: {} tools copied, {} failed", copied, copy_failed);
                }

                // All done — update UI on the main thread
                let ui_final = ui_for_thread.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_final.upgrade() {
                        // Turn off the download-active indicator
                        ui.set_pe_tools_download_active(false);
                        ui.set_pe_tools_download_progress("".into());

                        // Show result in the status bar
                        if fail_count == 0 && success_count > 0 {
                            let msg = if skipped > 0 {
                                format!("{} downloaded, {} already present", success_count, skipped)
                            } else {
                                format!("All {} PE tools downloaded successfully", success_count)
                            };
                            ui.set_status_text(msg.into());
                        } else if success_count == 0 && fail_count == 0 {
                            ui.set_status_text(
                                "All checked PE tools already downloaded".into()
                            );
                        } else {
                            ui.set_status_text(
                                format!("{} downloaded, {} failed, {} already present",
                                    success_count, fail_count, skipped).into()
                            );
                        }

                        // Refresh the status dots and summary after downloading
                        update_pe_tool_status(&ui, enabled_count);
                    }
                });
            });
        }
    });

    // ============================================
    // WINDOWS DEPLOY CALLBACKS
    // ============================================

    // Callback: Browse for Windows image (WIM/ESD/ISO)
    ui.on_deploy_browse_image({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Browse for image clicked");
            // Open file picker on the main thread (rfd works on main thread)
            if let Some(path) = deploy::pick_image_file() {
                if let Some(ui) = ui.upgrade() {
                    let filename = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    ui.set_deploy_wim_path(path.to_string_lossy().to_string().into());
                    ui.set_deploy_wim_name(filename.clone().into());
                    // Reset editions dropdown when new image is selected
                    let empty_model: std::rc::Rc<slint::VecModel<slint::SharedString>> =
                        std::rc::Rc::new(slint::VecModel::default());
                    ui.set_deploy_edition_list(empty_model.into());
                    ui.set_deploy_selected_edition_name("".into());
                    ui.set_status_text(format!("Image selected: {}", filename).into());
                }
            }
        }
    });

    // Callback: Refresh editions (parse WIM with DISM in background thread)
    ui.on_deploy_refresh_editions({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Refresh editions clicked");

            // Read the WIM path from UI on the main thread
            let wim_path_str = if let Some(ui) = ui.upgrade() {
                let p: String = ui.get_deploy_wim_path().to_string();
                if p.is_empty() {
                    ui.set_status_text("Please select an image file first".into());
                    return;
                }
                ui.set_deploy_detecting(true);
                ui.set_status_text("Scanning for Windows editions...".into());
                p
            } else {
                return;
            };

            // Run DISM in a background thread (it's slow)
            // If user selected an ISO, it will be mounted automatically to find the WIM inside
            let ui_worker = ui.clone();
            std::thread::spawn(move || {
                let image_path = std::path::Path::new(&wim_path_str);
                let result = deploy::parse_wim_editions(image_path);

                // Update UI back on the main thread
                let ui_final = ui_worker.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_final.upgrade() {
                        ui.set_deploy_detecting(false);
                        match result {
                            Ok((editions, resolved_wim_path)) => {
                                // If an ISO was mounted, update the wim_path to point at
                                // the actual install.wim inside the mounted ISO
                                ui.set_deploy_wim_path(
                                    resolved_wim_path.to_string_lossy().to_string().into()
                                );
                                // Build ComboBox model with just edition names (no size)
                                let names: Vec<slint::SharedString> = editions
                                    .iter()
                                    .map(|e| slint::SharedString::from(e.name.as_str()))
                                    .collect();
                                let model = std::rc::Rc::new(slint::VecModel::from(names));
                                ui.set_deploy_edition_list(model.into());
                                // Auto-select the first edition
                                if !editions.is_empty() {
                                    ui.set_deploy_selected_edition_name(
                                        editions[0].name.clone().into()
                                    );
                                }
                                ui.set_status_text(
                                    format!("Found {} edition(s)", editions.len()).into(),
                                );
                            }
                            Err(e) => {
                                ui.set_status_text(
                                    format!("Failed to read editions: {}", e).into(),
                                );
                            }
                        }
                    }
                });
            });
        }
    });

    // Callback: Refresh disks (detect in background thread)
    ui.on_deploy_refresh_disks({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Refresh disks clicked");

            if let Some(ui_ref) = ui.upgrade() {
                ui_ref.set_deploy_detecting(true);
                ui_ref.set_status_text("Detecting available disks...".into());
            }

            let ui_worker = ui.clone();
            std::thread::spawn(move || {
                let result = deploy::detect_disks();

                let ui_final = ui_worker.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_final.upgrade() {
                        ui.set_deploy_detecting(false);
                        match result {
                            Ok(disks) => {
                                // Build ComboBox model with disk display strings
                                let names: Vec<slint::SharedString> = disks
                                    .iter()
                                    .map(|d| slint::SharedString::from(d.display_string().as_str()))
                                    .collect();
                                let model = std::rc::Rc::new(slint::VecModel::from(names));
                                ui.set_deploy_disk_list(model.into());
                                // Auto-select the first disk
                                if !disks.is_empty() {
                                    ui.set_deploy_selected_disk_name(
                                        disks[0].display_string().into()
                                    );
                                }
                                ui.set_status_text(
                                    format!("Found {} disk(s)", disks.len()).into(),
                                );
                            }
                            Err(e) => {
                                ui.set_status_text(
                                    format!("Disk detection failed: {}", e).into(),
                                );
                            }
                        }
                    }
                });
            });
        }
    });

    // Callback: Start deployment (the main event!)
    ui.on_deploy_start({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Start deployment clicked");

            // ============================================
            // READ ALL UI STATE ON THE MAIN THREAD
            // ============================================
            // Slint properties can only be read on the UI thread
            let config = if let Some(ui) = ui.upgrade() {
                // Read the selected edition name directly from the ComboBox
                let edition_name: String = ui.get_deploy_selected_edition_name().to_string();

                // Figure out the edition index (1-based for DISM) by finding
                // which position in the dropdown list matches the selected name
                let edition_list = ui.get_deploy_edition_list();
                let edition_index = {
                    let mut idx = 0u32;
                    for i in 0..edition_list.row_count() {
                        if edition_list.row_data(i).map_or(false, |v| v.as_str() == edition_name) {
                            idx = (i + 1) as u32; // DISM uses 1-based index
                            break;
                        }
                    }
                    idx
                };

                // Parse disk number from the ComboBox selection
                // The display string starts with "Disk N:" — extract N
                let disk_id = if ui.get_deploy_let_windows_choose() {
                    -1i32
                } else {
                    let selected_disk: String = ui.get_deploy_selected_disk_name().to_string();
                    // Parse "Disk 0: Samsung SSD (500 GB, GPT)" → extract "0"
                    if selected_disk.starts_with("Disk ") {
                        selected_disk
                            .trim_start_matches("Disk ")
                            .split(':')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .parse::<i32>()
                            .unwrap_or(-1)
                    } else {
                        -1i32
                    }
                };

                // Parse boot mode
                let boot_mode_str: String = ui.get_deploy_boot_mode().to_string();
                let boot_mode = if boot_mode_str == "BIOS" {
                    deploy::BootMode::BIOS
                } else {
                    deploy::BootMode::UEFI
                };

                let config = deploy::DeployConfig {
                    wim_path: std::path::PathBuf::from(ui.get_deploy_wim_path().to_string()),
                    edition: edition_name,
                    edition_index,
                    computer_name: ui.get_deploy_computer_name().to_string(),
                    timezone: ui.get_deploy_timezone().to_string(),
                    language: ui.get_deploy_language().to_string(),
                    boot_mode,
                    disk_id,
                    bypass_win11: ui.get_deploy_bypass_win11(),
                    user_name: ui.get_deploy_user_name().to_string(),
                    user_password: ui.get_deploy_user_password().to_string(),
                    user_display_name: ui.get_deploy_user_display_name().to_string(),
                    user_is_admin: ui.get_deploy_user_is_admin(),
                    enable_autologon: ui.get_deploy_enable_autologon(),
                    skip_oobe: ui.get_deploy_skip_oobe(),
                    skip_eula: ui.get_deploy_skip_eula(),
                    skip_network: ui.get_deploy_skip_network(),
                    product_key: ui.get_deploy_product_key().to_string(),
                    organization: ui.get_deploy_organization().to_string(),
                    owner_name: ui.get_deploy_owner_name().to_string(),
                    disable_telemetry: ui.get_deploy_disable_telemetry(),
                    disable_location: ui.get_deploy_disable_location(),
                    disable_ads: ui.get_deploy_disable_ads(),
                    disable_suggested_apps: ui.get_deploy_disable_suggested_apps(),
                    disable_bing_search: ui.get_deploy_disable_bing_search(),
                    disable_smartscreen: ui.get_deploy_disable_smartscreen(),
                    enable_rdp: ui.get_deploy_enable_rdp(),
                    disable_uac: ui.get_deploy_disable_uac(),
                    disable_defender: ui.get_deploy_disable_defender(),
                    disable_firewall: ui.get_deploy_disable_firewall(),
                    disable_vbs: ui.get_deploy_disable_vbs(),
                    disable_bitlocker: ui.get_deploy_disable_bitlocker(),
                    disable_fast_startup: ui.get_deploy_disable_fast_startup(),
                    high_performance: ui.get_deploy_high_performance(),
                    disable_system_restore: ui.get_deploy_disable_system_restore(),
                    show_file_extensions: ui.get_deploy_show_file_extensions(),
                    show_hidden_files: ui.get_deploy_show_hidden_files(),
                    classic_context_menu: ui.get_deploy_classic_context_menu(),
                    taskbar_search_mode: ui.get_deploy_taskbar_search_mode() as u8,
                    hide_task_view: ui.get_deploy_hide_task_view(),
                    hide_widgets: ui.get_deploy_hide_widgets(),
                    taskbar_left_align: ui.get_deploy_taskbar_left_align(),
                    disable_cortana: ui.get_deploy_disable_cortana(),
                    disable_onedrive: ui.get_deploy_disable_onedrive(),
                    disable_teams: ui.get_deploy_disable_teams(),
                    disable_copilot: ui.get_deploy_disable_copilot(),
                    disable_widgets_service: ui.get_deploy_disable_widgets_service(),
                    join_domain: ui.get_deploy_join_domain(),
                    domain_name: ui.get_deploy_domain_name().to_string(),
                    domain_username: ui.get_deploy_domain_username().to_string(),
                    domain_password: ui.get_deploy_domain_password().to_string(),
                    workgroup: ui.get_deploy_workgroup().to_string(),
                    prevent_device_encryption: ui.get_deploy_disable_bitlocker(), // Same as bitlocker toggle
                };

                // Validate
                if config.wim_path.as_os_str().is_empty() {
                    ui.set_status_text("Please select a Windows image first".into());
                    return;
                }
                if config.edition.is_empty() {
                    ui.set_status_text("Please scan and select a Windows edition first".into());
                    return;
                }

                // Start building
                ui.set_deploy_building(true);
                ui.set_deploy_build_progress(0);
                ui.set_deploy_build_status("Starting deployment...".into());

                config
            } else {
                return;
            };

            // Run deployment in background thread
            let ui_for_progress = ui.clone();
            let ui_for_build = ui.clone();

            std::thread::spawn(move || {
                let result = deploy::execute(&config, move |progress, status| {
                    let ui_p = ui_for_progress.clone();
                    let s = status.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_p.upgrade() {
                            ui.set_deploy_build_progress(progress);
                            ui.set_deploy_build_status(s.into());
                        }
                    });
                });

                // Update UI after deployment completes
                let ui_final = ui_for_build.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_final.upgrade() {
                        ui.set_deploy_building(false);
                        ui.set_deploy_build_progress(0);
                        if result.success {
                            ui.set_deploy_build_status("Deployment complete!".into());
                            ui.set_status_text("Windows deployment completed successfully".into());
                        } else {
                            ui.set_deploy_build_status("".into());
                            ui.set_status_text(result.message.into());
                        }
                    }
                });
            });
        }
    });

    // Callback: Preview XML (generate autounattend.xml and show in status)
    ui.on_deploy_preview_xml({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Preview XML clicked");
            if let Some(ui) = ui.upgrade() {
                // Build a config from current UI state (same as deploy-start but no execution)
                let edition_name: String = ui.get_deploy_selected_edition_name().to_string();
                let edition_name = if edition_name.is_empty() {
                    "Windows 11 Pro".to_string() // Placeholder for preview
                } else {
                    edition_name
                };

                let boot_mode_str: String = ui.get_deploy_boot_mode().to_string();
                let config = deploy::DeployConfig {
                    edition: edition_name,
                    boot_mode: if boot_mode_str == "BIOS" { deploy::BootMode::BIOS } else { deploy::BootMode::UEFI },
                    computer_name: ui.get_deploy_computer_name().to_string(),
                    user_name: ui.get_deploy_user_name().to_string(),
                    ..deploy::DeployConfig::default()
                };

                let xml = deploy::generate_autounattend(&config);
                // Save the preview XML next to the EXE (e.g., on the USB drive)
                // instead of the temp folder. This way it's easy to find and stays
                // with the deployment toolkit. Falls back to temp dir if we can't
                // determine the EXE location (shouldn't happen in practice).
                let preview_path = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_else(std::env::temp_dir)
                    .join("autounattend_preview.xml");
                if let Ok(_) = std::fs::write(&preview_path, &xml) {
                    ui.set_status_text(format!("XML preview saved to: {}", preview_path.display()).into());
                    // Try to open it in Notepad so the user can review
                    let _ = std::process::Command::new("notepad.exe")
                        .arg(&preview_path)
                        .spawn();
                } else {
                    ui.set_status_text(format!("Generated {} bytes of XML", xml.len()).into());
                }
            }
        }
    });

    // Callback: Save profile — saves current UI settings as a named .json profile
    ui.on_deploy_save_profile({
        let ui = ui_handle.clone();
        move |name| {
            let name_str = name.to_string();
            println!("Deploy: Save profile '{}'", name_str);
            if let Some(ui) = ui.upgrade() {
                if name_str.trim().is_empty() {
                    ui.set_status_text("Please enter a profile name".into());
                    return;
                }
                // Build config from all UI toggle/text states
                let config = deploy::DeployConfig {
                    computer_name: ui.get_deploy_computer_name().to_string(),
                    timezone: ui.get_deploy_timezone().to_string(),
                    language: ui.get_deploy_language().to_string(),
                    user_name: ui.get_deploy_user_name().to_string(),
                    user_password: ui.get_deploy_user_password().to_string(),
                    user_display_name: ui.get_deploy_user_display_name().to_string(),
                    user_is_admin: ui.get_deploy_user_is_admin(),
                    enable_autologon: ui.get_deploy_enable_autologon(),
                    skip_oobe: ui.get_deploy_skip_oobe(),
                    skip_eula: ui.get_deploy_skip_eula(),
                    skip_network: ui.get_deploy_skip_network(),
                    bypass_win11: ui.get_deploy_bypass_win11(),
                    disable_telemetry: ui.get_deploy_disable_telemetry(),
                    disable_location: ui.get_deploy_disable_location(),
                    disable_ads: ui.get_deploy_disable_ads(),
                    disable_suggested_apps: ui.get_deploy_disable_suggested_apps(),
                    disable_bing_search: ui.get_deploy_disable_bing_search(),
                    disable_smartscreen: ui.get_deploy_disable_smartscreen(),
                    enable_rdp: ui.get_deploy_enable_rdp(),
                    disable_uac: ui.get_deploy_disable_uac(),
                    disable_defender: ui.get_deploy_disable_defender(),
                    disable_firewall: ui.get_deploy_disable_firewall(),
                    disable_vbs: ui.get_deploy_disable_vbs(),
                    disable_bitlocker: ui.get_deploy_disable_bitlocker(),
                    disable_fast_startup: ui.get_deploy_disable_fast_startup(),
                    high_performance: ui.get_deploy_high_performance(),
                    disable_system_restore: ui.get_deploy_disable_system_restore(),
                    show_file_extensions: ui.get_deploy_show_file_extensions(),
                    show_hidden_files: ui.get_deploy_show_hidden_files(),
                    classic_context_menu: ui.get_deploy_classic_context_menu(),
                    taskbar_search_mode: ui.get_deploy_taskbar_search_mode() as u8,
                    hide_task_view: ui.get_deploy_hide_task_view(),
                    hide_widgets: ui.get_deploy_hide_widgets(),
                    taskbar_left_align: ui.get_deploy_taskbar_left_align(),
                    disable_cortana: ui.get_deploy_disable_cortana(),
                    disable_onedrive: ui.get_deploy_disable_onedrive(),
                    disable_teams: ui.get_deploy_disable_teams(),
                    disable_copilot: ui.get_deploy_disable_copilot(),
                    disable_widgets_service: ui.get_deploy_disable_widgets_service(),
                    join_domain: ui.get_deploy_join_domain(),
                    domain_name: ui.get_deploy_domain_name().to_string(),
                    domain_username: ui.get_deploy_domain_username().to_string(),
                    domain_password: ui.get_deploy_domain_password().to_string(),
                    workgroup: ui.get_deploy_workgroup().to_string(),
                    prevent_device_encryption: ui.get_deploy_disable_bitlocker(),
                    ..deploy::DeployConfig::default()
                };

                match deploy::save_profile(&name_str, &config) {
                    Ok(()) => {
                        ui.set_status_text(format!("Profile '{}' saved", name_str).into());
                        ui.set_deploy_active_profile(name_str.clone().into());
                        // Refresh the ComboBox dropdown list with all saved profiles
                        let profiles = deploy::list_profiles();
                        let model = std::rc::Rc::new(slint::VecModel::from(
                            profiles.iter().map(|s| slint::SharedString::from(s.as_str())).collect::<Vec<_>>()
                        ));
                        ui.set_deploy_profile_list(model.into());
                    }
                    Err(e) => {
                        ui.set_status_text(format!("Failed to save profile: {}", e).into());
                    }
                }
            }
        }
    });

    // Callback: Select profile from ComboBox — auto-loads the selected profile
    ui.on_deploy_select_profile({
        let ui = ui_handle.clone();
        move |name| {
            let name_str = name.to_string();
            println!("Deploy: Auto-loading profile '{}'", name_str);
            if let Some(ui) = ui.upgrade() {
                if name_str.trim().is_empty() {
                    return;
                }
                // Load the profile and apply all settings to the UI
                match deploy::load_profile(&name_str) {
                    Ok(config) => {
                        // Apply every saved setting back to the UI
                        ui.set_deploy_computer_name(config.computer_name.into());
                        ui.set_deploy_timezone(config.timezone.into());
                        ui.set_deploy_language(config.language.into());
                        ui.set_deploy_user_name(config.user_name.into());
                        ui.set_deploy_user_password(config.user_password.into());
                        ui.set_deploy_user_display_name(config.user_display_name.into());
                        ui.set_deploy_user_is_admin(config.user_is_admin);
                        ui.set_deploy_enable_autologon(config.enable_autologon);
                        ui.set_deploy_skip_oobe(config.skip_oobe);
                        ui.set_deploy_skip_eula(config.skip_eula);
                        ui.set_deploy_skip_network(config.skip_network);
                        ui.set_deploy_bypass_win11(config.bypass_win11);
                        ui.set_deploy_disable_telemetry(config.disable_telemetry);
                        ui.set_deploy_disable_location(config.disable_location);
                        ui.set_deploy_disable_ads(config.disable_ads);
                        ui.set_deploy_disable_suggested_apps(config.disable_suggested_apps);
                        ui.set_deploy_disable_bing_search(config.disable_bing_search);
                        ui.set_deploy_disable_smartscreen(config.disable_smartscreen);
                        ui.set_deploy_enable_rdp(config.enable_rdp);
                        ui.set_deploy_disable_uac(config.disable_uac);
                        ui.set_deploy_disable_defender(config.disable_defender);
                        ui.set_deploy_disable_firewall(config.disable_firewall);
                        ui.set_deploy_disable_vbs(config.disable_vbs);
                        ui.set_deploy_disable_bitlocker(config.disable_bitlocker);
                        ui.set_deploy_disable_fast_startup(config.disable_fast_startup);
                        ui.set_deploy_high_performance(config.high_performance);
                        ui.set_deploy_disable_system_restore(config.disable_system_restore);
                        ui.set_deploy_show_file_extensions(config.show_file_extensions);
                        ui.set_deploy_show_hidden_files(config.show_hidden_files);
                        ui.set_deploy_classic_context_menu(config.classic_context_menu);
                        ui.set_deploy_taskbar_search_mode(config.taskbar_search_mode as i32);
                        ui.set_deploy_hide_task_view(config.hide_task_view);
                        ui.set_deploy_hide_widgets(config.hide_widgets);
                        ui.set_deploy_taskbar_left_align(config.taskbar_left_align);
                        ui.set_deploy_disable_cortana(config.disable_cortana);
                        ui.set_deploy_disable_onedrive(config.disable_onedrive);
                        ui.set_deploy_disable_teams(config.disable_teams);
                        ui.set_deploy_disable_copilot(config.disable_copilot);
                        ui.set_deploy_disable_widgets_service(config.disable_widgets_service);
                        ui.set_deploy_join_domain(config.join_domain);
                        ui.set_deploy_domain_name(config.domain_name.into());
                        ui.set_deploy_domain_username(config.domain_username.into());
                        ui.set_deploy_domain_password(config.domain_password.into());
                        ui.set_deploy_workgroup(config.workgroup.into());
                        let boot_str = match config.boot_mode {
                            deploy::BootMode::UEFI => "UEFI",
                            deploy::BootMode::BIOS => "BIOS",
                        };
                        ui.set_deploy_boot_mode(boot_str.into());

                        ui.set_deploy_active_profile(name_str.clone().into());
                        ui.set_status_text(format!("Profile '{}' loaded", name_str).into());
                    }
                    Err(e) => {
                        ui.set_status_text(format!("Failed to load profile: {}", e).into());
                    }
                }
            }
        }
    });

    // Callback: Import profile — opens file explorer to pick a .json profile from disk
    ui.on_deploy_import_profile({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Import profile from file");
            if let Some(ui) = ui.upgrade() {
                // Open file picker for .json profiles
                if let Some(path) = deploy::pick_profile_file() {
                    match deploy::load_profile_from_path(&path) {
                        Ok(config) => {
                            // Get the profile name from the filename (without .json)
                            let profile_name = path.file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| "Imported".to_string());

                            // Save a copy into our profiles folder so it shows in the dropdown
                            let _ = deploy::save_profile(&profile_name, &config);

                            // Apply all settings to the UI (same as select-profile)
                            ui.set_deploy_computer_name(config.computer_name.into());
                            ui.set_deploy_timezone(config.timezone.into());
                            ui.set_deploy_language(config.language.into());
                            ui.set_deploy_user_name(config.user_name.into());
                            ui.set_deploy_user_password(config.user_password.into());
                            ui.set_deploy_user_display_name(config.user_display_name.into());
                            ui.set_deploy_user_is_admin(config.user_is_admin);
                            ui.set_deploy_enable_autologon(config.enable_autologon);
                            ui.set_deploy_skip_oobe(config.skip_oobe);
                            ui.set_deploy_skip_eula(config.skip_eula);
                            ui.set_deploy_skip_network(config.skip_network);
                            ui.set_deploy_bypass_win11(config.bypass_win11);
                            ui.set_deploy_disable_telemetry(config.disable_telemetry);
                            ui.set_deploy_disable_location(config.disable_location);
                            ui.set_deploy_disable_ads(config.disable_ads);
                            ui.set_deploy_disable_suggested_apps(config.disable_suggested_apps);
                            ui.set_deploy_disable_bing_search(config.disable_bing_search);
                            ui.set_deploy_disable_smartscreen(config.disable_smartscreen);
                            ui.set_deploy_enable_rdp(config.enable_rdp);
                            ui.set_deploy_disable_uac(config.disable_uac);
                            ui.set_deploy_disable_defender(config.disable_defender);
                            ui.set_deploy_disable_firewall(config.disable_firewall);
                            ui.set_deploy_disable_vbs(config.disable_vbs);
                            ui.set_deploy_disable_bitlocker(config.disable_bitlocker);
                            ui.set_deploy_disable_fast_startup(config.disable_fast_startup);
                            ui.set_deploy_high_performance(config.high_performance);
                            ui.set_deploy_disable_system_restore(config.disable_system_restore);
                            ui.set_deploy_show_file_extensions(config.show_file_extensions);
                            ui.set_deploy_show_hidden_files(config.show_hidden_files);
                            ui.set_deploy_classic_context_menu(config.classic_context_menu);
                            ui.set_deploy_taskbar_search_mode(config.taskbar_search_mode as i32);
                            ui.set_deploy_hide_task_view(config.hide_task_view);
                            ui.set_deploy_hide_widgets(config.hide_widgets);
                            ui.set_deploy_taskbar_left_align(config.taskbar_left_align);
                            ui.set_deploy_disable_cortana(config.disable_cortana);
                            ui.set_deploy_disable_onedrive(config.disable_onedrive);
                            ui.set_deploy_disable_teams(config.disable_teams);
                            ui.set_deploy_disable_copilot(config.disable_copilot);
                            ui.set_deploy_disable_widgets_service(config.disable_widgets_service);
                            ui.set_deploy_join_domain(config.join_domain);
                            ui.set_deploy_domain_name(config.domain_name.into());
                            ui.set_deploy_domain_username(config.domain_username.into());
                            ui.set_deploy_domain_password(config.domain_password.into());
                            ui.set_deploy_workgroup(config.workgroup.into());
                            let boot_str = match config.boot_mode {
                                deploy::BootMode::UEFI => "UEFI",
                                deploy::BootMode::BIOS => "BIOS",
                            };
                            ui.set_deploy_boot_mode(boot_str.into());

                            // Refresh the dropdown and set active profile
                            ui.set_deploy_active_profile(profile_name.clone().into());
                            let profiles = deploy::list_profiles();
                            let model = std::rc::Rc::new(slint::VecModel::from(
                                profiles.iter().map(|s| slint::SharedString::from(s.as_str())).collect::<Vec<_>>()
                            ));
                            ui.set_deploy_profile_list(model.into());

                            ui.set_status_text(format!("Imported profile '{}'", profile_name).into());
                        }
                        Err(e) => {
                            ui.set_status_text(format!("Failed to import profile: {}", e).into());
                        }
                    }
                }
                // If user cancelled the file picker, do nothing
            }
        }
    });

    // Callback: Delete profile — removes the selected profile and refreshes dropdown
    ui.on_deploy_delete_profile({
        let ui = ui_handle.clone();
        move |name| {
            let name_str = name.to_string();
            println!("Deploy: Delete profile '{}'", name_str);
            if let Some(ui) = ui.upgrade() {
                if name_str.trim().is_empty() {
                    ui.set_status_text("No profile selected to delete".into());
                    return;
                }
                match deploy::delete_profile(&name_str) {
                    Ok(()) => {
                        ui.set_status_text(format!("Profile '{}' deleted", name_str).into());
                        // Clear active profile and refresh dropdown
                        ui.set_deploy_active_profile("".into());
                        let profiles = deploy::list_profiles();
                        let model = std::rc::Rc::new(slint::VecModel::from(
                            profiles.iter().map(|s| slint::SharedString::from(s.as_str())).collect::<Vec<_>>()
                        ));
                        ui.set_deploy_profile_list(model.into());
                    }
                    Err(e) => {
                        ui.set_status_text(format!("Failed to delete profile: {}", e).into());
                    }
                }
            }
        }
    });

    // Callback: Refresh profiles — rebuilds the ComboBox dropdown list
    ui.on_deploy_refresh_profiles({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Refresh profile list");
            if let Some(ui) = ui.upgrade() {
                let profiles = deploy::list_profiles();
                let model = std::rc::Rc::new(slint::VecModel::from(
                    profiles.iter().map(|s| slint::SharedString::from(s.as_str())).collect::<Vec<_>>()
                ));
                ui.set_deploy_profile_list(model.into());
            }
        }
    });

    // ============================================
    // DEPLOY: SCRIPT MANAGEMENT CALLBACKS
    // ============================================
    // These callbacks handle adding/removing FirstLogon scripts that get
    // copied to the target Windows installation after setup completes.
    // Scripts run on first user logon via:
    //   - Automated mode: autounattend.xml <FirstLogonCommands>
    //   - Normal mode: RunOnce registry key injected into offline hive

    // Callback: Add a FirstLogon script — opens file picker, copies to FirstLogon/ folder
    ui.on_deploy_add_firstlogon_script({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Add FirstLogon script");
            // Open a file picker to select a script file (.ps1, .bat, .cmd, .exe, .reg)
            if let Some(path) = deploy::pick_script_file() {
                match deploy::add_script("FirstLogon", &path) {
                    Ok(()) => {
                        println!("Deploy: Added FirstLogon script: {:?}", path);
                        // Refresh the script list in the UI
                        if let Some(ui) = ui.upgrade() {
                            let scripts = deploy::list_scripts("FirstLogon");
                            ui.set_deploy_firstlogon_scripts(scripts.join(";").into());
                            ui.set_status_text(format!("Added script: {}",
                                path.file_name().unwrap_or_default().to_string_lossy()).into());
                        }
                    }
                    Err(e) => {
                        if let Some(ui) = ui.upgrade() {
                            ui.set_status_text(format!("Failed to add script: {}", e).into());
                        }
                    }
                }
            }
        }
    });

    // Callback: Remove a FirstLogon script by name
    ui.on_deploy_remove_firstlogon_script({
        let ui = ui_handle.clone();
        move |name| {
            let filename = name.to_string();
            println!("Deploy: Remove FirstLogon script: {}", filename);
            match deploy::remove_script("FirstLogon", &filename) {
                Ok(()) => {
                    if let Some(ui) = ui.upgrade() {
                        let scripts = deploy::list_scripts("FirstLogon");
                        ui.set_deploy_firstlogon_scripts(scripts.join(";").into());
                        ui.set_status_text(format!("Removed script: {}", filename).into());
                    }
                }
                Err(e) => {
                    if let Some(ui) = ui.upgrade() {
                        ui.set_status_text(format!("Failed to remove script: {}", e).into());
                    }
                }
            }
        }
    });

    // Callback: Refresh the FirstLogon script list — rescans the FirstLogon/ folder
    ui.on_deploy_refresh_scripts({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Refresh script list");
            if let Some(ui) = ui.upgrade() {
                let firstlogon = deploy::list_scripts("FirstLogon");
                ui.set_deploy_firstlogon_scripts(firstlogon.join(";").into());
            }
        }
    });

    // ============================================
    // DEPLOY: NORMAL INSTALL CALLBACK
    // ============================================
    // Normal Install launches setup.exe interactively (no answer file).
    // The user answers all prompts themselves.

    ui.on_deploy_start_normal({
        let ui = ui_handle.clone();
        move || {
            println!("Deploy: Start Normal Install clicked");

            if let Some(ui) = ui.upgrade() {
                // Validate: make sure a WIM/ISO path is set
                let wim_path = ui.get_deploy_wim_path().to_string();
                if wim_path.is_empty() {
                    ui.set_status_text("Please select a Windows image first".into());
                    return;
                }

                // Start the progress indicators
                ui.set_deploy_building(true);
                ui.set_deploy_build_progress(0);
                ui.set_deploy_build_status("Starting Normal Install...".into());
            }

            // Run normal install in a background thread
            let ui_for_progress = ui.clone();
            let ui_for_done = ui.clone();

            std::thread::spawn(move || {
                // Execute the normal install pipeline:
                // Step 1: Find setup.exe on available drives
                // Step 2: Validate setup.exe
                // Step 3: Launch setup.exe interactively (wait for user to finish)
                // Step 4: Copy post-install scripts to target drive
                // Step 5: Reboot
                let result = deploy::normal_execute(move |progress, status| {
                    let ui_p = ui_for_progress.clone();
                    let s = status.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_p.upgrade() {
                            ui.set_deploy_build_progress(progress);
                            ui.set_deploy_build_status(s.into());
                        }
                    });
                });

                // Update UI when normal install completes
                let ui_final = ui_for_done.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_final.upgrade() {
                        ui.set_deploy_building(false);
                        ui.set_deploy_build_progress(0);
                        if result.success {
                            ui.set_deploy_build_status("Normal Install complete!".into());
                            ui.set_status_text("Windows installation completed successfully".into());
                        } else {
                            ui.set_deploy_build_status("".into());
                            ui.set_status_text(result.message.into());
                        }
                    }
                });
            });
        }
    });

    // ============================================
    // RUN THE APPLICATION
    // ============================================
    // This starts the event loop - the window stays open and responds to clicks.
    // The program stays here until the user closes the window.

    // Load the saved profile list into the ComboBox dropdown on startup
    {
        let profiles = deploy::list_profiles();
        let model = std::rc::Rc::new(slint::VecModel::from(
            profiles.iter().map(|s| slint::SharedString::from(s.as_str())).collect::<Vec<_>>()
        ));
        ui.set_deploy_profile_list(model.into());
    }

    // Load the FirstLogon script list on startup so the UI shows any previously added scripts
    {
        let firstlogon = deploy::list_scripts("FirstLogon");
        ui.set_deploy_firstlogon_scripts(firstlogon.join(";").into());
    }

    // Check for saved product keys on startup (from a previous session)
    // If saved_keys.json exists next to the EXE, show the saved info in the UI.
    // With multi-key support, we show how many keys are saved.
    {
        let saved_keys = deploy::load_saved_keys();
        if !saved_keys.is_empty() {
            let last = &saved_keys[saved_keys.len() - 1]; // Most recent entry
            ui.set_backup_key_saved_info(
                slint::SharedString::from(format!("{} key(s) saved (latest: {} on {})",
                    saved_keys.len(), last.hostname, last.date)));
        }
    }

    // ============================================
    // AUTO-UPDATE CHECK ON STARTUP
    // ============================================
    // Check for updates in the background (non-blocking).
    // Skip in WinPE — no reliable internet and we're focused on deployment.
    if !is_winpe {
        // Step 1: Check if the EXE version changed since last run.
        // This happens after an auto-update — the new EXE has a different
        // version, so we refresh PE tool manifests from embedded defaults.
        if updater::check_version_change() {
            println!("Version change detected — refreshing PE tool manifests");
            updater::refresh_pe_tool_manifests();
        }
        // Always save current version (creates file on first run)
        updater::save_current_version();

        // Step 2: Check GitHub for a newer release (background thread).
        // This runs silently — no error messages shown to the user on startup.
        let ui_for_update = ui.as_weak();
        std::thread::spawn(move || {
            println!("Checking for updates...");
            let result = updater::check_for_updates();

            // Send results back to the UI thread
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_for_update.upgrade() {
                    if result.update_available {
                        // Update found! Show the sidebar badge
                        println!(
                            "Update available: {} -> {}",
                            result.current_version, result.latest_version
                        );
                        ui.set_update_available(true);
                        ui.set_update_latest_version(
                            format!("v{}", result.latest_version).into(),
                        );
                        ui.set_update_release_notes(result.release_notes.into());
                        ui.set_update_download_url(result.download_url.into());
                        ui.set_update_size_display(
                            updater::format_size(result.download_size).into(),
                        );
                        ui.set_status_text(
                            format!(
                                "Update available: v{} ({}) — click the badge in the sidebar to download",
                                result.latest_version,
                                updater::format_size(result.download_size)
                            )
                            .into(),
                        );
                    } else if !result.error.is_empty() {
                        // Silently log errors on startup — don't bother the user
                        println!("Update check failed (non-blocking): {}", result.error);
                    } else {
                        println!("No update available (current: {})", result.current_version);
                    }
                }
            });
        });
    }

    println!("Starting UI...");
    ui.run()
}

// ============================================
// HELPER FUNCTIONS
// ============================================

/// Detect if we're running in a WinPE environment
/// WinPE is identified by:
/// - Running from X: drive
/// - Presence of specific WinPE marker files
/// - Registry entries
fn detect_winpe_environment() -> bool {
    // Check for common WinPE indicators

    // 1. Check if X:\Windows exists (typical WinPE system drive)
    if Path::new("X:\\Windows").exists() {
        return true;
    }

    // 2. Check for WinPE marker files
    let markers = [
        "X:\\Windows\\System32\\winpeshl.ini",
        "X:\\Windows\\System32\\startnet.cmd",
    ];

    for marker in &markers {
        if Path::new(marker).exists() {
            return true;
        }
    }

    // 3. Check for MasterBooter USB marker (future feature)
    // This lets users indicate they want "PE mode" behavior even in live Windows
    if Path::new(".\\MasterBooter.pe.marker").exists() {
        return true;
    }

    false
}

/// Update the PE tool status dots and summary in the UI.
/// Scans the pe_tools folder to see which tools are downloaded (present on disk),
/// then sets each pe-tool-*-present property and updates the pe-tools-summary text.
///
/// # Arguments
/// * `ui` - Reference to the main window
/// * `enabled_count` - Number of enabled tools (used for summary text).
///                     Pass 0 to auto-count from discovered tools.
fn update_pe_tool_status(ui: &MainWindow, enabled_count: usize) {
    // Discover all PE tools on disk
    let discovered = tools::pe_tools::discover_pe_tools();

    // Count how many are present (downloaded) and how many are enabled
    let present_count = discovered.iter().filter(|t| t.is_present).count();
    let total_count = discovered.len();
    let actual_enabled = if enabled_count > 0 { enabled_count }
        else { discovered.iter().filter(|t| t.enabled).count() };

    // Match each discovered tool to its UI property by name
    // (tool names come from tool.toml manifests in pe_tools/)
    for tool in &discovered {
        let is_present = tool.is_present;
        // Match tool name to the corresponding UI property
        match tool.name.as_str() {
            "WinXShell" => ui.set_pe_tool_winxshell_present(is_present),
            "Explorer++" => ui.set_pe_tool_explorer_present(is_present),
            "PENetwork" => ui.set_pe_tool_penetwork_present(is_present),
            "CrystalDiskInfo" => ui.set_pe_tool_crystaldisk_present(is_present),
            "7-Zip" => ui.set_pe_tool_7zip_present(is_present),
            "Autoruns" => ui.set_pe_tool_autoruns_present(is_present),
            "Disk Check" => ui.set_pe_tool_diskcheck_present(is_present),
            "DISM Tool" => ui.set_pe_tool_dismtool_present(is_present),
            "Web Browser" => ui.set_pe_tool_webbrowser_present(is_present),
            "Event Viewer" => ui.set_pe_tool_eventviewer_present(is_present),
            "Installed Software" => ui.set_pe_tool_installedsw_present(is_present),
            "File Explorer" => ui.set_pe_tool_fileexplorer_present(is_present),
            _ => {
                // Custom/unknown tools — no UI dot for these yet
                println!("  Unknown PE tool for status dot: {}", tool.name);
            }
        }
    }

    // Update the summary text (e.g. "5 of 7 downloaded")
    let summary = format!("{} of {} downloaded", present_count, total_count);
    ui.set_pe_tools_summary(summary.into());

    println!("PE tool status updated: {}/{} present, {}/{} enabled",
        present_count, total_count, actual_enabled, total_count);
}

/// Find the MasterBooter tools folder
/// Returns the path to the 'tools' folder next to the executable
#[allow(dead_code)]
fn get_tools_folder() -> std::path::PathBuf {
    // Get the folder where the EXE is located
    let exe_path = std::env::current_exe().unwrap_or_default();
    let exe_folder = exe_path.parent().unwrap_or(Path::new("."));

    // The tools folder is next to the EXE
    exe_folder.join("tools")
}
