use std::env;
use std::path::PathBuf;

const RELEASE_URL: &str = "https://github.com/wowjeeez/rsnet/releases/download";
const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    let goos = go_os(&rust_os);
    let goarch = go_arch(&rust_arch);
    let platform_dir = vendored_dir.join(format!("{goos}-{goarch}"));

    let bindings_out = out_dir.join("libtailscale.rs");
    let archive_out = out_dir.join("libtailscale.a");

    println!("cargo:rerun-if-changed=build.rs");

    // 1: check for local vendored archive (CI path)
    let vendored_archive = platform_dir.join("libtailscale.a");
    let vendored_bindings = platform_dir.join("libtailscale.rs");

    if vendored_archive.exists() && vendored_bindings.exists() {
        println!("cargo:rustc-link-search=native={}", platform_dir.display());
        println!("cargo:rustc-link-lib=static=tailscale");
        link_system_libs(&rust_os);
        std::fs::copy(&vendored_bindings, &bindings_out).expect("failed to copy vendored bindings");
        return;
    }

    // 2: check if already downloaded to OUT_DIR (cached between builds)
    if archive_out.exists() && bindings_out.exists() {
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=tailscale");
        link_system_libs(&rust_os);
        return;
    }

    // 3: try downloading prebuilt from github releases (crates.io path)
    let archive_url = format!(
        "{RELEASE_URL}/v{VERSION}/libtailscale-{goos}-{goarch}.a"
    );
    if try_download(&archive_url, &archive_out) {
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=tailscale");
        link_system_libs(&rust_os);

        // generate bindings from the header shipped in the crate
        let header = manifest_dir.join("libtailscale/tailscale.h");
        if header.exists() {
            let bindings = bindgen::Builder::default()
                .header(header.to_str().unwrap())
                .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
                .generate()
                .expect("unable to generate bindings");
            bindings.write_to_file(&bindings_out).expect("couldn't write bindings");
        } else {
            panic!("downloaded archive but libtailscale/tailscale.h not found");
        }
        return;
    }

    // 4: build from go source (dev path)
    println!("cargo:rerun-if-changed={}", libtailscale_dir.display());

    let go_archive = libtailscale_dir.join("libtailscale.a");
    let status = std::process::Command::new("go")
        .args(["build", "-buildmode=c-archive", "-o"])
        .arg(&go_archive)
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

fn try_download(url: &str, dest: &PathBuf) -> bool {
    // follow redirects with curl (github releases redirect to S3)
    let status = std::process::Command::new("curl")
        .args(["-fSL", "--retry", "3", "--retry-delay", "2", "-o"])
        .arg(dest)
        .arg(url)
        .status();

    match status {
        Ok(s) if s.success() && dest.exists() && dest.metadata().map(|m| m.len() > 0).unwrap_or(false) => {
            eprintln!("downloaded {url}");
            true
        }
        _ => {
            eprintln!("failed to download {url}, falling back to go build");
            let _ = std::fs::remove_file(dest);
            false
        }
    }
}

fn link_system_libs(target_os: &str) {
    if target_os == "macos" {
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=IOKit");
    }
    println!("cargo:rustc-link-lib=resolv");
}
