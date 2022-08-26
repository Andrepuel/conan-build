use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

const BUILD_INFO: &str = "conanbuildinfo.json";

pub struct Conan {
    build_info: HashMap<String, Value>,
    libs: HashMap<String, Link>,
    settings: Value,
}
impl Default for Conan {
    fn default() -> Self {
        Self::new()
    }
}
impl Conan {
    pub fn new() -> Conan {
        let build_info_path = Self::find_build_info();
        println!(
            "cargo:rerun-if-changed={path}",
            path = build_info_path.to_string_lossy()
        );
        let build_info: Value =
            serde_json::from_str(&std::fs::read_to_string(&build_info_path).unwrap())
                .expect("Invalid build info json");

        let settings = build_info["settings"].clone();
        let build_info = crate::build_info(&build_info);
        let libs = crate::find_all_libs(build_info.iter());

        Conan {
            build_info,
            libs,
            settings,
        }
    }

    pub fn find_build_info() -> PathBuf {
        std::env::current_dir()
            .unwrap()
            .ancestors()
            .find_map(|path| {
                let mut path = path.to_owned();
                path.push(BUILD_INFO);
                eprintln!("Trying {path:?}");

                match path.exists() {
                    true => Some(path),
                    false => None,
                }
            })
            .unwrap_or_else(|| {
                panic!(
                    "Could not find {BUILD_INFO}. Did you forget to run conan install?",
                    BUILD_INFO = BUILD_INFO
                )
            })
    }

    pub fn depends_on<'a, I: IntoIterator<Item = &'a str>>(&self, packages: I) {
        DependsOn::extend_all(
            packages
                .into_iter()
                .map(|package| self.get_depends_on_package(package)),
        )
        .apply()
    }

    pub fn depends_on_optional<'a, I: IntoIterator<Item = &'a str>>(&self, packages: I) {
        DependsOn::extend_all(
            packages
                .into_iter()
                .filter(|package| self.try_package(package).is_some())
                .map(|package| self.get_depends_on_package(package)),
        )
        .apply()
    }

    pub fn depends_on_libcxx(&self) {
        if let Some(cxx) = self.libcxx() {
            cxx.apply();
        }
    }

    pub fn get_depends_on<'a, I: IntoIterator<Item = &'a str>>(&self, packages: I) -> DependsOn {
        packages
            .into_iter()
            .map(|package| self.get_depends_on_package(package))
            .fold(DependsOn::default(), |mut a, b| {
                a.extend(b);
                a
            })
    }

    pub fn get_depends_on_package(&self, package: &str) -> DependsOn {
        let libs = self
            .libs_for(package)
            .into_iter()
            .map(|name| Lib {
                is_static: !self.is_shared(name),
                name: name.to_string(),
            })
            .collect();
        let libdirs = self
            .libdir_for(package)
            .into_iter()
            .map(|dir| LibDir(dir.to_string()))
            .collect();

        DependsOn { libs, libdirs }
    }

    pub fn all_deps(&self) -> impl Iterator<Item = &str> {
        self.build_info.keys().map(|x| x.as_str())
    }

    pub fn libs_for(&self, package: &str) -> Vec<&str> {
        self.package(package)["libs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lib| lib.as_str().unwrap())
            .collect()
    }

    pub fn libdir_for(&self, package: &str) -> Vec<&str> {
        Self::libdir_for_package(self.package(package)).collect()
    }

    pub fn package(&self, package: &str) -> &Value {
        self.try_package(package)
            .unwrap_or_else(|| panic!("No dependency {package:?} in conan info"))
    }

    pub fn try_package(&self, package: &str) -> Option<&Value> {
        self.build_info.get(package)
    }

    pub fn libdir_for_package(value: &Value) -> impl Iterator<Item = &str> {
        value["lib_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lib| lib.as_str().unwrap())
    }

    pub fn bindir_for(&self, package: &str) -> Vec<&str> {
        self.build_info[package]["bin_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lib| lib.as_str().unwrap())
            .collect()
    }

    pub fn rootpath_for(&self, package: &str) -> &str {
        self.build_info[package]["rootpath"].as_str().unwrap()
    }

    pub fn generate_env_source(&self) {
        let openssl_dir = self.rootpath_for("openssl");
        let lddirs = self.libdir_for("openssl").join(":");

        let mut env = File::create("env.sh").unwrap();
        writeln!(env, "export OPENSSL_DIR={openssl_dir}",).unwrap();
        writeln!(env, "export LD_LIBRARY_PATH={lddirs}").unwrap();

        let mut env = File::create("env.ps1").unwrap();
        let lddirs = self.bindir_for("openssl").join(";").replace('\\', "\\\\");
        writeln!(env, "$env:OPENSSL_DIR=\"{openssl_dir}\"").unwrap();
        writeln!(env, "$env:PATH=\"{lddirs};$env:PATH\"").unwrap();
    }

    pub fn is_shared(&self, lib: &str) -> bool {
        self.libs.get(lib).copied().unwrap_or(Link::Shared) == Link::Shared
    }

    pub fn package_is_shared(options: &HashMap<String, String>, package: &str) -> Option<bool> {
        let option = format!("{package}:shared");

        let r = options.get(&option)?.to_lowercase().parse().unwrap();

        Some(r)
    }

    pub fn libcxx(&self) -> Option<Lib> {
        self.libcxx_name().map(|name| Lib {
            is_static: false,
            name: name.to_string(),
        })
    }

    fn libcxx_name(&self) -> Option<&str> {
        let libcxx = self
            .settings
            .as_object()
            .expect("settings attribute is an object")
            .get("compiler.libcxx")?
            .as_str()
            .expect("compiler.libcxx attribute is an string");

        Some(match libcxx {
            "libstdc++11" => "stdc++",
            x if x.starts_with("lib") => &x[3..],
            x => x,
        })
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum Link {
    Static,
    Shared,
}

fn build_info(root: &Value) -> HashMap<String, Value> {
    root["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|dep| {
            let name = dep["name"].as_str().unwrap().to_string();
            (name, dep.clone())
        })
        .collect()
}

fn find_all_libs<'a, I: Iterator<Item = (&'a String, &'a Value)>>(it: I) -> HashMap<String, Link> {
    it.flat_map(|(_, v)| {
        Conan::libdir_for_package(v).flat_map(|path| {
            Path::new(path)
                .read_dir()
                .unwrap()
                .map(|x| x.unwrap())
                .filter_map(|entry| {
                    let lib = entry.file_name().to_string_lossy().into_owned();
                    if lib.ends_with(".lib") {
                        let lib = &lib[..lib.len() - 4];

                        let mut dll = entry.path();
                        dll.pop();
                        dll.pop();
                        dll.push("bin");
                        dll.push(format!("{lib}.dll"));

                        let link = match dll.exists() {
                            true => Link::Shared,
                            false => Link::Static,
                        };

                        return Some((lib.to_string(), link));
                    }

                    let link;
                    if lib.ends_with(".so") {
                        link = Link::Shared;
                    } else if lib.ends_with(".a") {
                        link = Link::Static;
                    } else {
                        return None;
                    }

                    let lib = match lib.starts_with("lib") {
                        true => &lib[3..],
                        false => &lib,
                    };
                    let ext = lib.rfind('.');
                    let lib = match ext {
                        Some(ext) => &lib[..ext],
                        None => lib,
                    };

                    Some((lib.to_string(), link))
                })
        })
    })
    .collect()
}

pub trait Applyable {
    fn apply(&self);
}

pub struct Lib {
    pub is_static: bool,
    pub name: String,
}
impl Applyable for Lib {
    fn apply(&self) {
        let name = &self.name;
        let is_static = self.is_static;

        let static_ = match is_static {
            true => "static=",
            false => "",
        };

        println!("cargo:rustc-link-lib={static_}{name}");
    }
}

pub struct LibDir(pub String);
impl Applyable for LibDir {
    fn apply(&self) {
        println!("cargo:rustc-link-search={dir}", dir = self.0);
    }
}

#[derive(Default)]
pub struct DependsOn {
    pub libs: Vec<Lib>,
    pub libdirs: Vec<LibDir>,
}
impl DependsOn {
    pub fn extend(&mut self, rhs: DependsOn) {
        self.libs.extend(rhs.libs);
        self.libdirs.extend(rhs.libdirs);
    }

    fn extend_all<I: IntoIterator<Item = DependsOn>>(iter: I) -> DependsOn {
        iter.into_iter()
            .reduce(|mut a, b| {
                a.extend(b);
                a
            })
            .unwrap_or_default()
    }
}
impl Applyable for DependsOn {
    fn apply(&self) {
        self.libs.iter().for_each(Applyable::apply);
        self.libdirs.iter().for_each(Applyable::apply);
    }
}
