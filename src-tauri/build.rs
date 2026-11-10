fn main() {
    #[cfg(windows)] {
        let out=std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
        let profile=out.ancestors().nth(3).expect("Cargo profile directory");
        let runtime=std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory")).join("runtime/windows");
        std::fs::create_dir_all(&runtime).expect("Create runtime staging directory");
        std::fs::create_dir_all(profile.join("deps")).expect("Create test runtime directory");
        for name in ["sherpa-onnx-c-api.dll","onnxruntime.dll","onnxruntime_providers_shared.dll"] {
            let source=profile.join(name);
            println!("cargo:rerun-if-changed={}",source.display());
            // App and unit-test executables must load the same ONNX version, not
            // the unrelated older DLL included in Windows' system directory.
            std::fs::copy(&source,runtime.join(name)).expect("Stage meeting runtime for installer");
            std::fs::copy(&source,profile.join("deps").join(name)).expect("Stage meeting runtime for tests");
        }
    }
    tauri_build::build()
}
