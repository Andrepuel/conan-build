use conan_build::Conan;

fn main() {
    let mut conan = Conan::new();
    conan.depends_on(["zeromq"]);
    conan.depends_on_optional(["libsodium"]);
    conan.depends_on_libcxx();
}
