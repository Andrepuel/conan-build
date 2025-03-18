use conan_build::Conan;

fn main() {
    Conan::with_host(env!("TARGET").to_owned()).generate_env_source();
}
