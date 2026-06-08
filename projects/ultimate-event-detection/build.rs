fn main() {
    #[cfg(all(target_os = "macos", feature = "metal"))]
    compile_metal();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn compile_metal() {
    use std::path::PathBuf;
    use std::process::Command;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let metal_src = "src/metal/metal_detect.metal";

    let air_file = out_dir.join("metal_detect.air");
    let metallib_file = out_dir.join("metal_detect.metallib");

    let status = Command::new("xcrun")
        .args(["-sdk", "macosx", "metal", "-c", metal_src, "-o", air_file.to_str().unwrap()])
        .status()
        .expect("Failed to execute metal compiler");
    if !status.success() {
        panic!("Metal shader compilation failed");
    }

    let status = Command::new("xcrun")
        .args(["-sdk", "macosx", "metallib", air_file.to_str().unwrap(), "-o", metallib_file.to_str().unwrap()])
        .status()
        .expect("Failed to execute metallib");
    if !status.success() {
        panic!("Metal library linking failed");
    }

    println!("cargo:rustc-env=METAL_LIBRARY={}", metallib_file.display());
    println!("cargo:rerun-if-changed={}", metal_src);

    cc::Build::new()
        .file("src/metal/metal_bridge.m")
        .flag("-fmodules")
        .flag("-fobjc-arc")
        .compile("metal_bridge");

    println!("cargo:rustc-link-framework=Metal");
    println!("cargo:rustc-link-framework=Foundation");
    println!("cargo:rerun-if-changed=src/metal/metal_bridge.m");
}
