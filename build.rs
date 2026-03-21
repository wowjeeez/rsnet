use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let libtailscale_dir = manifest_dir.join("libtailscale");
    let vendored_dir = manifest_dir.join("vendored");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let platform_dir = vendored_dir.join(format!("{target_os}-{target_arch}"));

    println!("cargo:rerun-if-changed=build.rs");

    let lib_ext = if target_os == "windows" { "lib" } else { "a" };

    // check for prebuilt vendored archive (CI / crates.io path)
    let vendored_archive = platform_dir.join(format!("libtailscale.{lib_ext}"));
    let vendored_bindings = platform_dir.join("libtailscale.rs");

    if vendored_archive.exists() && vendored_bindings.exists() {
        println!("cargo:rustc-link-search=native={}", platform_dir.display());
        println!("cargo:rustc-link-lib=static=tailscale");
        link_system_libs(&target_os);

        let out_path = manifest_dir.join("src/vendor/libtailscale.rs");
        std::fs::copy(&vendored_bindings, &out_path).expect("failed to copy vendored bindings");
        return;
    }

    // dev path: build from go source
    println!("cargo:rerun-if-changed={}", libtailscale_dir.display());

    let archive = libtailscale_dir.join(format!("libtailscale.{lib_ext}"));
    let status = std::process::Command::new("go")
        .args(["build", "-buildmode=c-archive", "-o"])
        .arg(&archive)
        .arg(".")
        .current_dir(&libtailscale_dir)
        .status()
        .expect("failed to run go build — is Go installed?");

    if !status.success() {
        panic!("go build failed with status: {}", status);
    }

    println!("cargo:rustc-link-search=native={}", libtailscale_dir.display());
    println!("cargo:rustc-link-lib=static=tailscale");
    link_system_libs(&target_os);

    let bindings = bindgen::Builder::default()
        .header(libtailscale_dir.join("tailscale.h").to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("unable to generate bindings");

    let out_path = manifest_dir.join("src/vendor/libtailscale.rs");
    bindings.write_to_file(&out_path).expect("couldn't write bindings");
}

fn link_system_libs(target_os: &str) {
    match target_os {
        "macos" => {
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
            println!("cargo:rustc-link-lib=framework=Security");
            println!("cargo:rustc-link-lib=framework=IOKit");
            println!("cargo:rustc-link-lib=resolv");
        }
        "windows" => {
            println!("cargo:rustc-link-lib=ws2_32");
            println!("cargo:rustc-link-lib=iphlpapi");
            println!("cargo:rustc-link-lib=ole32");
            println!("cargo:rustc-link-lib=userenv");
            println!("cargo:rustc-link-lib=ntdll");
        }
        _ => {
            // linux and others
            println!("cargo:rustc-link-lib=resolv");
        }
    }
}
