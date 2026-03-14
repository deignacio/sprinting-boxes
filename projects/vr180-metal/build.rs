use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=shaders/reproject.metal");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let air = out_dir.join("reproject.air");
    let metallib = out_dir.join("default.metallib");

    let shader_src = PathBuf::from("shaders/reproject.metal");

    // Compile .metal → .air
    let status = Command::new("xcrun")
        .args([
            "-sdk", "macosx",
            "metal", "-c",
            shader_src.to_str().unwrap(),
            "-o", air.to_str().unwrap(),
        ])
        .status()
        .unwrap_or_else(|e| {
            eprintln!("ERROR: xcrun metal failed: {e}");
            eprintln!("       Install the Metal Toolchain with:");
            eprintln!("         sudo xcodebuild -runFirstLaunch");
            eprintln!("         xcodebuild -downloadComponent MetalToolchain");
            panic!("Metal toolchain not installed");
        });

    if !status.success() {
        eprintln!("ERROR: Metal shader compilation failed.");
        eprintln!("       Install the Metal Toolchain with:");
        eprintln!("         sudo xcodebuild -runFirstLaunch");
        eprintln!("         xcodebuild -downloadComponent MetalToolchain");
        panic!("Metal shader compilation failed");
    }

    // Link .air → .metallib
    let status = Command::new("xcrun")
        .args([
            "-sdk", "macosx",
            "metallib",
            air.to_str().unwrap(),
            "-o", metallib.to_str().unwrap(),
        ])
        .status()
        .expect("xcrun metallib not found");

    assert!(status.success(), "metallib linking failed");

    println!("cargo:rustc-env=METALLIB_PATH={}", metallib.display());
}
