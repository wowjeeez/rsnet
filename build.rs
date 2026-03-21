use std::env;
use std::path::PathBuf;

fn go_os(rust_os: &str) -> &str {
    match rust_os {
        "macos" => "darwin",
        other => other,
    }
}

fn go_arch(rust_arch: &str) -> &str {
    match rust_arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let libtailscale_dir = manifest_dir.join("libtailscale");
    let vendored_dir = manifest_dir.join("vendored");

    let rust_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let rust_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let platform_dir = vendored_dir.join(format!("{}-{}", go_os(&rust_os), go_arch(&rust_arch)));

    let bindings_out = out_dir.join("libtailscale.rs");

    println!("cargo:rerun-if-changed=build.rs");

    // check for prebuilt vendored archive (CI / crates.io path)
    let vendored_archive = platform_dir.join("libtailscale.a");
    let vendored_bindings = platform_dir.join("libtailscale.rs");

    if vendored_archive.exists() && vendored_bindings.exists() {
        println!("cargo:rustc-link-search=native={}", platform_dir.display());
        println!("cargo:rustc-link-lib=static=tailscale");
        link_system_libs(&rust_os);

        std::fs::copy(&vendored_bindings, &bindings_out).expect("failed to copy vendored bindings");
        return;
    }

    // dev path: build from go source
    println!("cargo:rerun-if-changed={}", libtailscale_dir.display());

    let archive = libtailscale_dir.join("libtailscale.a");
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
    link_system_libs(&rust_os);

    let bindings = bindgen::Builder::default()
        .header(libtailscale_dir.join("tailscale.h").to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("unable to generate bindings");

    bindings.write_to_file(&bindings_out).expect("couldn't write bindings");
}

fn link_system_libs(target_os: &str) {
    if target_os == "macos" {
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=IOKit");
    }
    println!("cargo:rustc-link-lib=resolv");
}
