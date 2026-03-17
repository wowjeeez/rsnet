use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=./libtailscale");
    Command::new("go").arg("build").arg("-buildmode=c-archive")
        .current_dir(Path::new("./libtailscale"))
        .status().unwrap();
    println!("cargo:rustc-link-search=./libtailscale");
    let bindings = bindgen::Builder::default()
        .header("./libtailscale/tailscale.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    bindings.write_to_file("src/vendor/libtailscale.rs").expect("Couldn't write bindings!");
}