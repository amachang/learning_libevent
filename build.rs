use std::{
    env,
    path::{
        Path,
        PathBuf,
    },
};

use conan::*;
use bindgen;

fn main() {
    println!("cargo:rerun-if-changed=conanfile.txt");
    println!("cargo:rerun-if-changed=wrapper.h");

    let conan_profile = "default";

    let command = InstallCommandBuilder::new()
        .with_profile(&conan_profile)
        .build_policy(BuildPolicy::Missing)
        .recipe_path(Path::new("conanfile.txt"))
        .build();

    let Some(build_info) = command.generate() else {
        eprintln!("Conan command failed!: args={:?} output_dir={:?} output_file={:?}", command.args(), command.output_dir(), command.output_file());
        std::process::exit(1);
    };

    build_info.cargo_emit();
    

    let mut bindgen_builder = bindgen::Builder::default();

    for dependency in build_info.dependencies() {
        if let Some(include_dir) = dependency.get_include_dir() {
            bindgen_builder = bindgen_builder.clang_arg(format!("-I{}", include_dir));
        }
    }

    let bindings = bindgen_builder
        .header("wrapper.h")
        .generate()
        .expect("Failed to generate bindings");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Failed to get OUT_DIR"));
    bindings.write_to_file(out_dir.join("bindings.rs"))
        .expect("Failed to write bindings")
}


