// ============================================
// MasterBooter - build.rs
// ============================================
// This file runs BEFORE the main program is compiled.
// It does two things:
// 1. Compile the Slint UI files (.slint) into Rust code
// 2. Embed the Windows icon into the EXE (so it shows in File Explorer/taskbar)
//
// You don't need to modify this file unless you:
// - Rename the main .slint file
// - Change the icon file
// ============================================

fn main() {
    // Step 1: Compile the main Slint UI file
    // This converts src/ui/main.slint into Rust code that main.rs can use
    //
    // If compilation fails, you'll see an error message telling you:
    // - Which line in the .slint file has the problem
    // - What the error is (missing semicolon, unknown property, etc.)
    if let Err(e) = slint_build::compile("src/ui/main.slint") {
        // Print a helpful error message
        eprintln!("============================================");
        eprintln!("ERROR: Failed to compile Slint UI");
        eprintln!("============================================");
        eprintln!("{}", e);
        eprintln!("");
        eprintln!("Make sure src/ui/main.slint exists and has valid syntax.");
        eprintln!("Check the Slint documentation: https://slint.dev/docs/");
        eprintln!("============================================");

        // Exit with error code so the build fails
        std::process::exit(1);
    }

    // Step 2: Embed the Windows icon into the EXE
    // This makes the icon show up in:
    // - File Explorer (when you browse to the EXE)
    // - Windows taskbar (when the app is running)
    // - Alt+Tab switcher
    // Only runs on Windows targets (skipped on other platforms)
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to embed Windows icon: {}", e);
            // Don't fail the build — the app works fine without an icon
        }
    }
}
