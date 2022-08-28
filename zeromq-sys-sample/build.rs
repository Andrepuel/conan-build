use conan_build::Conan;
use std::process::Command;

fn main() {
    // Remarks: It is not recommended to run conan install on build.rs
    // as it does not sit well with multiple crates.
    let status = Command::new("conan")
        .args([
            "install",
            ".",
            "-g",
            "json",
            "-o",
            "zeromq:shared=True",
            "-b",
            "missing",
        ])
        .status()
        .unwrap();

    if !status.success() {
        panic!("conan install failed");
    }

    let conan = Conan::new();
    conan.generate_env_source();
    conan.depends_on(["zeromq"]);
    conan.depends_on_optional(["libsodium"]);
    conan.depends_on_libcxx();
}
