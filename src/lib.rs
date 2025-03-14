use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io,
    path::{Path, PathBuf},
};

const BUILD_INFO: &str = "conanbuildinfo.json";

pub struct BuildInfoSet {
    info: HashMap<String, BuildInfo>,
}
impl BuildInfoSet {
    pub fn find_all() -> Self {
        todo!()
    }

    pub fn path_from_env() -> impl Iterator<Item = (String, PathBuf)> {
        std::env::vars().filter_map(|(key, path)| match key.split_once('_') {
            Some((target, "CONANBUILDINFO")) => Some((target.to_string(), path.into())),
            None if key == "CONANBUILDINFO" => Some(("".to_string(), path.into())),
            _ => None,
        })
    }

    pub fn path_from_filesystem() -> impl Iterator<Item = (String, PathBuf)> {
        [].into_iter()
        // std::env::current_dir()
        //     .expect("current dir may not fail")
        //     .ancestors()
        //     .flat_map(|dir| {
        //         dir.read_dir().unwrap().filter_map(|path| {

        //         })
        //     })
    }

    pub fn get_current_target(&self) -> &BuildInfo {
        todo!()
    }

    pub fn all_targets<'a>(
        &'a self,
        host: &'a str,
    ) -> impl Iterator<Item = (String, bool, &'a BuildInfo)> + use<'a> {
        self.info.iter().map(move |(target, info)| {
            let is_host = target == host || target.is_empty();
            let prefix = match target.is_empty() {
                true => String::new(),
                false => format!("{target}_"),
            };

            (prefix, is_host, info)
        })
    }
}

pub struct BuildInfo {
    path: PathBuf,
    info: HashMap<String, Value>,
    libs: HashMap<String, Link>,
    settings: Value,
}
impl BuildInfo {
    pub fn read_build_info<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let info: Value = serde_json::from_str(&std::fs::read_to_string(path.as_ref()).unwrap())
            .expect("Invalid build info json");

        let settings = info["settings"].clone();
        let info = crate::build_info(&info);
        let libs = crate::find_all_libs(info.iter());

        Ok(Self {
            path: path.as_ref().to_owned(),
            info,
            libs,
            settings,
        })
    }

    pub fn all_deps(&self) -> impl Iterator<Item = &str> + Clone {
        self.info.keys().map(String::as_str)
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

    pub fn is_shared(&self, lib: &str) -> bool {
        self.libs.get(lib).copied().unwrap_or(Link::Shared) == Link::Shared
    }

    pub fn libdir_for(&self, package: &str) -> Vec<&str> {
        Self::libdir_for_package(self.package(package)).collect()
    }

    pub fn libdir_for_package(value: &Value) -> impl Iterator<Item = &str> {
        value["lib_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lib| lib.as_str().unwrap())
    }

    pub fn libs_for(&self, package: &str) -> Vec<&str> {
        self.package(package)["libs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lib| lib.as_str().unwrap())
            .collect()
    }

    pub fn includes_for(&self, package: &str) -> Vec<&str> {
        self.package(package)["include_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|json| json.as_str().unwrap())
            .collect()
    }

    pub fn bindir_for(&self, package: &str) -> Vec<&str> {
        self.package(package)["bin_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lib| lib.as_str().unwrap())
            .collect()
    }

    pub fn rootpath_for(&self, package: &str) -> &str {
        self.package(package)["rootpath"].as_str().unwrap()
    }

    pub fn package(&self, package: &str) -> &Value {
        self.try_package(package)
            .unwrap_or_else(|| panic!("No dependency {package:?} in conan info"))
    }

    pub fn try_package(&self, package: &str) -> Option<&Value> {
        self.info.get(package)
    }

    pub fn write_env_source<W1, W2>(
        &self,
        is_host: bool,
        prefix: &str,
        mut sh: W1,
        mut ps1: W2,
    ) -> io::Result<()>
    where
        W1: io::Write,
        W2: io::Write,
    {
        writeln!(
            sh,
            "export {prefix}CONANBUILDINFO={}",
            self.path.to_string_lossy()
        )
        .unwrap();
        writeln!(
            ps1,
            "$env:{prefix}CONANBUILDINFO=\"{}\"",
            self.path.to_string_lossy()
        )
        .unwrap();

        let shared_deps = self.all_deps().filter(|package| {
            self.libs_for(package)
                .into_iter()
                .any(|lib| self.is_shared(lib))
        });

        let libdirs = shared_deps
            .clone()
            .flat_map(|package| self.libdir_for(package).into_iter())
            .collect::<Vec<_>>()
            .join(":");

        if !libdirs.is_empty() && is_host {
            writeln!(sh, "export LD_LIBRARY_PATH={libdirs}").unwrap();
        }
        let bindirs = shared_deps
            .flat_map(|package| self.bindir_for(package))
            .collect::<Vec<_>>()
            .join(";")
            .replace('\\', "\\\\");

        if !bindirs.is_empty() && is_host {
            writeln!(ps1, "$env:PATH=\"{bindirs};$env:PATH\"").unwrap();
        }

        if self.try_package("openssl").is_some() {
            let openssl_dir = self.rootpath_for("openssl");
            writeln!(sh, "export {prefix}OPENSSL_DIR={openssl_dir}",).unwrap();
            writeln!(ps1, "$env:{prefix}OPENSSL_DIR=\"{openssl_dir}\"").unwrap();
        }

        sh.flush()?;
        ps1.flush()?;

        Ok(())
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

pub struct Conan {
    build_info_set: BuildInfoSet,
    host: String,
    rerun_if_changed: bool,
}
impl Default for Conan {
    fn default() -> Self {
        Self::new()
    }
}
impl Conan {
    pub fn new() -> Conan {
        let host = std::env::var("TARGET").expect("TARGET variable must be set");
        Self::with_host(host)
    }

    pub fn with_host(host: String) -> Conan {
        let build_info_set = BuildInfoSet::find_all();

        Conan {
            build_info_set,
            host,
            rerun_if_changed: false,
        }
    }

    pub fn mark_rerun_if_changed(&mut self) {
        if self.rerun_if_changed {
            return;
        }

        self.rerun_if_changed = true;
        let build_info = self.build_info_set.get_current_target();
        println!(
            "cargo:rerun-if-changed={path}",
            path = build_info.path.to_string_lossy()
        );
    }

    pub fn build_info(&self) -> &BuildInfo {
        self.build_info_set.get_current_target()
    }

    pub fn depends_on<'a, I: IntoIterator<Item = &'a str>>(&mut self, packages: I) {
        self.mark_rerun_if_changed();
        let info = self.build_info();
        DependsOn::extend_all(
            packages
                .into_iter()
                .map(|package| info.get_depends_on_package(package)),
        )
        .apply()
    }

    pub fn depends_on_optional<'a, I: IntoIterator<Item = &'a str>>(&mut self, packages: I) {
        self.mark_rerun_if_changed();
        let info = self.build_info();
        DependsOn::extend_all(
            packages
                .into_iter()
                .filter(|package| info.try_package(package).is_some())
                .map(|package| info.get_depends_on_package(package)),
        )
        .apply()
    }

    pub fn depends_on_libcxx(&mut self) {
        self.mark_rerun_if_changed();
        if let Some(cxx) = self.build_info().libcxx() {
            cxx.apply();
        }
    }

    pub fn generate_env_source(&self) -> io::Result<()> {
        let mut sh = File::create("env.sh").unwrap();
        let mut ps1 = File::create("env.ps1").unwrap();

        for (prefix, is_host, info) in self.build_info_set.all_targets(&self.host) {
            info.write_env_source(is_host, &prefix, &mut sh, &mut ps1)?;
        }

        Ok(())
    }

    pub fn package_is_shared(options: &HashMap<String, String>, package: &str) -> Option<bool> {
        let option = format!("{package}:shared");

        let r = options.get(&option)?.to_lowercase().parse().unwrap();

        Some(r)
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
        BuildInfo::libdir_for_package(v).flat_map(|path| {
            Path::new(path)
                .read_dir()
                .unwrap_or_else(|e| panic!("Failure reading dir {path:?}: {e}"))
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
