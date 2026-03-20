use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let libtailscale_dir = manifest_dir.join("libtailscale");
    let archive = libtailscale_dir.join("libtailscale.a");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", libtailscale_dir.display());

    let status = Command::new("go")
        .args(["build", "-buildmode=c-archive", "-o"])
        .arg(&archive)
        .arg(".")
        .current_dir(&libtailscale_dir)
        .status()
        .expect("failed to run go build — is Go installed?");

    if !status.success() {
        panic!("go build failed with status: {}", status);
    }

    // IMPORTANT: use "tailscale" not "libtailscale" because the linker prepends "lib"
    // so it looks for "libtailscale.a" which is what we produce
    println!("cargo:rustc-link-search=native={}", libtailscale_dir.display());
    println!("cargo:rustc-link-lib=static=tailscale");

    // On macOS, libtailscale needs these frameworks
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=IOKit");
    }
    println!("cargo:rustc-link-lib=resolv");

    let bindings = bindgen::Builder::default()
        .header(libtailscale_dir.join("tailscale.h").to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("unable to generate bindings");

    let out_path = manifest_dir.join("src/vendor/libtailscale.rs");
    bindings.write_to_file(&out_path).expect("couldn't write bindings");
}
