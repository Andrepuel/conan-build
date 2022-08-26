use conan_build::{Applyable, Conan};
use std::process::Command;

fn main() {
    // Remarks: It is not recommended to run conan install on build.rs
    // as it does not sit well with multiple crates.
    Command::new("conan")
        .args(["install", ".", "-g", "json"])
        .status()
        .unwrap();

    let conan = Conan::new();
    conan.depends_on(["zeromq"]);
    conan.depends_on_optional(["libsodium"]);

    if let Some(cxx) = conan.libcxx() {
        cxx.apply();
    }
}
