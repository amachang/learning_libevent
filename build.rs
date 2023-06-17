use std::{
    process::exit,
    env,
    path::{
        Path,
        PathBuf,
    },
};

use conan;
use bindgen;
use regex::Regex;

fn main() {
    println!("cargo:rerun-if-changed=conanfile.txt");
    println!("cargo:rerun-if-changed=wrapper.h");

    let conan_profile = "default";

    if None == conan::find_program() {
        eprintln!("Conan command not found!");
        exit(1);
    };

    let conan_version_regex = Regex::new(r"^(\d+)\.(\d+).(\d+)$").unwrap();
    let conan_version = conan::find_version();
    let Some(conan_version) = conan_version else {
        eprintln!("Unrecognized conan version!");
        exit(1);
    };
    let conan_version_captures = conan_version_regex.captures(&conan_version).expect("Invalid version number.");
    let conan_major_version = conan_version_captures[1].parse::<u8>();
    let Ok(conan_major_version) = conan_major_version else {
        eprintln!("Unrecognized conan version!: {}", conan_version);
        exit(1);
    };
    if conan_major_version != 1 {
        eprintln!("Unsupported conan version!: {}", conan_version);
        exit(1);
    };

    let command = conan::InstallCommandBuilder::new()
        .with_profile(&conan_profile)
        .build_policy(conan::BuildPolicy::Missing)
        .recipe_path(Path::new("conanfile.txt"))
        .build();

    let Some(build_info) = command.generate() else {
        eprintln!("Conan command failed!: args={:?} output_dir={:?} output_file={:?}", command.args(), command.output_dir(), command.output_file());
        exit(1);
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


