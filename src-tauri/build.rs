use cmake;
use std::env;
use std::path::PathBuf;

fn main() {
    // Build opus as a static library
    let opus_dst = cmake::build("opus");
    println!("cargo:rustc-link-search=native={}/lib", opus_dst.display());
    println!("cargo:rustc-link-lib=static=opus");

    // Tell cargo to invalidate the built crate whenever the wrapper changes
    println!("cargo:rerun-if-changed=opus/include/opus.h");

    // Generate bindings for opus
    let bindings = bindgen::Builder::default()
        // The input header we would like to generate bindings for
        .header("opus/include/opus.h")
        // Add the opus include directory
        .clang_arg("-Iopus/include")
        // Only generate bindings for opus encoder functions
        .allowlist_function("opus_encoder_.*")
        .allowlist_function("opus_encode.*")
        .allowlist_function("opus_strerror")
        .allowlist_function("opus_get_version_string")
        // Include the relevant types
        .allowlist_type("OpusEncoder")
        // Include relevant constants
        .allowlist_var("OPUS_.*")
        // Include CTL requests
        .allowlist_var("OPUS_GET_LOOKAHEAD_REQUEST")
        // Make OpusEncoder opaque since we only use it as a pointer
        .opaque_type("OpusEncoder")
        // Tell cargo to invalidate the built crate whenever any of the included header files changed
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        // Finish the builder and generate the bindings
        .generate()
        .expect("Unable to generate opus bindings");

    // Write the bindings to the $OUT_DIR/opus_bindings.rs file
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("opus_bindings.rs"))
        .expect("Couldn't write opus bindings!");

    tauri_build::build()
}