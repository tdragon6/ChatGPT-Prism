use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../native");
    println!("cargo:rerun-if-changed=../scripts/build-native.mjs");
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-env-changed=PRISM_WINDOWS_CSC");
    println!("cargo:rerun-if-env-changed=PRISM_WINDOWS_NETFX");
    println!("cargo:rerun-if-env-changed=PRISM_WINDOWS_DOTNET");
    let target = env::var("TARGET").unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let path = output.join("window-host");
    let status = Command::new("node")
        .args(["../scripts/build-native.mjs", &target])
        .arg(&path)
        .status()
        .expect("run the native helper build");
    assert!(status.success(), "failed to build the native window helper");
    tauri_build::build();
}
