//! This build script automates the frontend build process and macOS CoreML compilation.
//! It detects changes in the `sb-dashboard` directory and triggers `npm install` and `npm run build`
//! to ensure the frontend assets are up-to-date and available for embedding in the Rust binary.
//!
//! On macOS, it also compiles Objective-C CoreML helper functions.

use std::process::Command;

fn main() {
    // Compile Objective-C helpers on macOS
    #[cfg(target_os = "macos")]
    compile_coreml_helpers();

    println!("cargo:rerun-if-changed=../sb-dashboard/package.json");
    println!("cargo:rerun-if-changed=../sb-dashboard/package-lock.json");
    println!("cargo:rerun-if-changed=../sb-dashboard/src");
    println!("cargo:rerun-if-changed=../sb-dashboard/public");
    println!("cargo:rerun-if-changed=../sb-dashboard/index.html");

    let dashboard_dir = "../sb-dashboard";

    // Check if npm is installed
    if Command::new("npm").arg("--version").output().is_err() {
        println!("cargo:warning=npm not found. Skipping frontend build. Assets might be missing.");
        return;
    }

    println!("cargo:warning=Building frontend assets...");

    // Install dependencies
    let status = Command::new("npm")
        .arg("install")
        .current_dir(dashboard_dir)
        .status();

    if let Ok(status) = status {
        if !status.success() {
            println!("cargo:warning=npm install failed");
        }
    } else {
        println!("cargo:warning=Failed to execute npm install");
    }

    // Build
    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(dashboard_dir)
        .status();

    if let Ok(status) = status {
        if !status.success() {
            println!("cargo:warning=npm run build failed");
        }
    } else {
        println!("cargo:warning=Failed to execute npm run build");
    }
}

#[cfg(target_os = "macos")]
fn compile_coreml_helpers() {
    // Compile CoreMLHelper.m Objective-C source
    cc::Build::new()
        .file("src/detection/CoreMLHelper.m")
        .flag("-fobjc-arc") // Enable Automatic Reference Counting
        .flag("-fmodules") // Enable Clang modules
        .compile("coreml_helper");

    // Link required frameworks
    println!("cargo:rustc-link-framework=CoreML");
    println!("cargo:rustc-link-framework=CoreVideo");
    println!("cargo:rustc-link-framework=Foundation");

    // Recompile if the helper files change
    println!("cargo:rerun-if-changed=src/detection/CoreMLHelper.h");
    println!("cargo:rerun-if-changed=src/detection/CoreMLHelper.m");
}
