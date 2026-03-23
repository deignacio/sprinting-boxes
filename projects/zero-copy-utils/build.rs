use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=shaders/process.metal");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let air = out_dir.join("process.air");
    let metallib = out_dir.join("default.metallib");

    let shader_src = PathBuf::from("shaders/process.metal");

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
            eprintln!("       Install the Metal toolchain with: sudo xcodebuild -runFirstLaunch");
            panic!("Metal toolchain not installed");
        });

    assert!(status.success(), "Metal shader compilation failed");

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
