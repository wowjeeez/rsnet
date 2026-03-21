use std::env;
use std::path::PathBuf;

const RELEASE_URL: &str = "https://github.com/wowjeeez/rsnet/releases/download";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn go_os(rust_os: &str) -> &str {
    match rust_os { "macos" => "darwin", other => other }
}

fn go_arch(rust_arch: &str) -> &str {
    match rust_arch { "aarch64" => "arm64", "x86_64" => "amd64", other => other }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let bindings_out = out_dir.join("libtailscale.rs");

    if env::var("DOCS_RS").is_ok() {
        std::fs::write(&bindings_out, STUB_BINDINGS).unwrap();
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let libtailscale_dir = manifest_dir.join("libtailscale");
    let rust_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let rust_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let goos = go_os(&rust_os);
    let goarch = go_arch(&rust_arch);
    let header = libtailscale_dir.join("tailscale.h");

    println!("cargo:rerun-if-changed=build.rs");

    // ci path: vendored/ has the archive with tailscale.c already baked in
    let vendored = manifest_dir.join(format!("vendored/{goos}-{goarch}"));
    if vendored.join("libtailscale.a").exists() && vendored.join("libtailscale.rs").exists() {
        println!("cargo:rustc-link-search=native={}", vendored.display());
        println!("cargo:rustc-link-lib=static=tailscale");
        link_system_libs(&rust_os);
        std::fs::copy(vendored.join("libtailscale.rs"), &bindings_out).unwrap();
        return;
    }

    // crates.io path: no go source, download archive from github release
    let has_go_source = libtailscale_dir.join("tailscale.go").exists();
    let archive_out = out_dir.join("libtailscale.a");

    if !has_go_source {
        if archive_out.exists() && bindings_out.exists() {
            // already downloaded on a previous build
            println!("cargo:rustc-link-search=native={}", out_dir.display());
            println!("cargo:rustc-link-lib=static=tailscale");
            link_system_libs(&rust_os);
            return;
        }

        let url = format!("{RELEASE_URL}/v{VERSION}/libtailscale-{goos}-{goarch}.a");
        if try_download(&url, &archive_out) {
            println!("cargo:rustc-link-search=native={}", out_dir.display());
            println!("cargo:rustc-link-lib=static=tailscale");
            link_system_libs(&rust_os);
            gen_bindings(&header, &bindings_out);
            return;
        }

        panic!("no go source and failed to download prebuilt archive from {url}");
    }

    // dev path: build from go source + compile tailscale.c wrapper
    println!("cargo:rerun-if-changed={}", libtailscale_dir.display());

    let go_archive = libtailscale_dir.join("libtailscale.a");
    let status = std::process::Command::new("go")
        .args(["build", "-buildmode=c-archive", "-o"])
        .arg(&go_archive)
        .arg(".")
        .current_dir(&libtailscale_dir)
        .status()
        .expect("go not found");

    if !status.success() {
        panic!("go build failed: {status}");
    }

    cc::Build::new()
        .file(libtailscale_dir.join("tailscale.c"))
        .include(&libtailscale_dir)
        .compile("tailscale_c");

    println!("cargo:rustc-link-search=native={}", libtailscale_dir.display());
    println!("cargo:rustc-link-lib=static=tailscale");
    link_system_libs(&rust_os);
    gen_bindings(&header, &bindings_out);
}

fn gen_bindings(header: &std::path::Path, out: &std::path::Path) {
    bindgen::Builder::default()
        .header(header.to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen failed")
        .write_to_file(out)
        .expect("failed to write bindings");
}

fn try_download(url: &str, dest: &PathBuf) -> bool {
    let ok = std::process::Command::new("curl")
        .args(["-fSL", "--retry", "3", "--retry-delay", "2", "-o"])
        .arg(dest).arg(url)
        .status()
        .is_ok_and(|s| s.success());
    if ok && dest.exists() && dest.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        eprintln!("downloaded {url}");
        true
    } else {
        let _ = std::fs::remove_file(dest);
        false
    }
}

fn link_system_libs(os: &str) {
    if os == "macos" {
        for fw in ["CoreFoundation", "Security", "IOKit"] {
            println!("cargo:rustc-link-lib=framework={fw}");
        }
    }
    println!("cargo:rustc-link-lib=resolv");
}

const STUB_BINDINGS: &str = r#"
pub type tailscale = ::std::os::raw::c_int;
pub type tailscale_conn = ::std::os::raw::c_int;
pub type tailscale_listener = ::std::os::raw::c_int;
unsafe extern "C" { pub fn tailscale_new() -> tailscale; }
unsafe extern "C" { pub fn tailscale_start(sd: tailscale) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_up(sd: tailscale) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_close(sd: tailscale) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_set_dir(sd: tailscale, dir: *const ::std::os::raw::c_char) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_set_hostname(sd: tailscale, hostname: *const ::std::os::raw::c_char) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_set_authkey(sd: tailscale, authkey: *const ::std::os::raw::c_char) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_set_control_url(sd: tailscale, control_url: *const ::std::os::raw::c_char) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_set_ephemeral(sd: tailscale, ephemeral: ::std::os::raw::c_int) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_set_logfd(sd: tailscale, fd: ::std::os::raw::c_int) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_getips(sd: tailscale, buf: *mut ::std::os::raw::c_char, buflen: usize) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_dial(sd: tailscale, network: *const ::std::os::raw::c_char, addr: *const ::std::os::raw::c_char, conn_out: *mut tailscale_conn) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_listen(sd: tailscale, network: *const ::std::os::raw::c_char, addr: *const ::std::os::raw::c_char, listener_out: *mut tailscale_listener) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_listen_tls(sd: tailscale, network: *const ::std::os::raw::c_char, addr: *const ::std::os::raw::c_char, listener_out: *mut tailscale_listener) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_listen_service(sd: tailscale, service_name: *const ::std::os::raw::c_char, service_mode: *const ::std::os::raw::c_char, port: ::std::os::raw::c_int, https: ::std::os::raw::c_int, terminate_tls: ::std::os::raw::c_int, listener_out: *mut tailscale_listener, fqdn_out: *mut ::std::os::raw::c_char, fqdn_len: usize) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_accept(listener: tailscale_listener, conn_out: *mut tailscale_conn) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_loopback(sd: tailscale, addr_out: *mut ::std::os::raw::c_char, addrlen: usize, proxy_cred_out: *mut ::std::os::raw::c_char, local_api_cred_out: *mut ::std::os::raw::c_char) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_enable_funnel_to_localhost_plaintext_http1(sd: tailscale, localhostPort: ::std::os::raw::c_int) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_errmsg(sd: tailscale, buf: *mut ::std::os::raw::c_char, buflen: usize) -> ::std::os::raw::c_int; }
unsafe extern "C" { pub fn tailscale_getremoteaddr(l: tailscale_listener, conn: tailscale_conn, buf: *mut ::std::os::raw::c_char, buflen: usize) -> ::std::os::raw::c_int; }
"#;
