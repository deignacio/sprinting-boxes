//! This build script automates the frontend build process and macOS Metal/CoreML compilation.

use std::process::Command;

fn main() {
    // Compile Metal shaders on macOS
    #[cfg(target_os = "macos")]
    compile_metal_shaders();

    // Compile Objective-C helpers on macOS
    #[cfg(target_os = "macos")]
    compile_metal_bridge();

    // Compile CoreML helpers on macOS
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
fn compile_metal_shaders() {
    use std::path::PathBuf;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let metal_src = "src/metal_detect.metal";

    // Compile Metal shader source to AIR (Intermediate Representation)
    let air_file = out_dir.join("metal_detect.air");
    let metallib_file = out_dir.join("metal_detect.metallib");

    // xcrun metal: compile Metal source to AIR
    let metal_status = Command::new("xcrun")
        .args(&[
            "-sdk", "macosx",
            "metal",
            "-c",
            metal_src,
            "-o",
            air_file.to_str().unwrap(),
        ])
        .status()
        .expect("Failed to execute metal compiler");

    if !metal_status.success() {
        panic!("Metal shader compilation failed");
    }

    // xcrun metallib: link AIR to metallib
    let metallib_status = Command::new("xcrun")
        .args(&[
            "-sdk", "macosx",
            "metallib",
            air_file.to_str().unwrap(),
            "-o",
            metallib_file.to_str().unwrap(),
        ])
        .status()
        .expect("Failed to execute metallib");

    if !metallib_status.success() {
        panic!("Metal library linking failed");
    }

    // Tell Cargo where to find the compiled metallib
    println!("cargo:rustc-env=METAL_LIBRARY={}", metallib_file.display());
    println!("cargo:rerun-if-changed={}", metal_src);
}

#[cfg(target_os = "macos")]
fn compile_metal_bridge() {
    // Compile Objective-C Metal bridge
    cc::Build::new()
        .file("src/metal_bridge.m")
        .flag("-fmodules")
        .flag("-fobjc-arc")
        .compile("metal_bridge");

    // Link Metal framework
    println!("cargo:rustc-link-framework=Metal");
    println!("cargo:rustc-link-framework=Foundation");

    println!("cargo:rerun-if-changed=src/metal_bridge.m");
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
