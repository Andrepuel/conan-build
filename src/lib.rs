use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io,
    ops::Deref,
    path::{Path, PathBuf},
};

const BUILD_INFO: &str = "conanbuildinfo.json";

pub struct BuildInfoSet {
    info: HashMap<&'static str, BuildInfo>,
}
impl BuildInfoSet {
    pub fn find_all() -> io::Result<Self> {
        let current_dir = std::env::current_dir()?;
        let info = Self::path_from_filesystem(&current_dir)
            .chain(Self::path_from_env())
            .filter(|path| path.exists())
            .map(|path| BuildInfo::read_build_info(&path).map_err(|e| (path, e)))
            .filter_map(|r| {
                r.map(|info| (info.target(), info))
                    .map_err(|(path, e)| eprintln!("Error opening {path:?}: {e}"))
                    .ok()
            })
            .collect();

        Ok(Self { info })
    }

    pub fn path_from_env() -> impl Iterator<Item = PathBuf> {
        std::env::vars().filter_map(|(key, path)| match key.split_once('_') {
            Some((_target, "CONANBUILDINFO")) => Some(path.into()),
            None if key == "CONANBUILDINFO" => Some(path.into()),
            _ => None,
        })
    }

    pub fn path_from_filesystem(current_dir: &Path) -> impl Iterator<Item = PathBuf> + use<'_> {
        current_dir
            .ancestors()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .flat_map(|dir| {
                dir.read_dir()
                    .unwrap()
                    .map(|path| path.expect("read dir may not fail").path().join(BUILD_INFO))
                    .chain([dir.join(BUILD_INFO)])
            })
    }

    pub fn get_current_target(&self, host: &str) -> Option<&BuildInfo> {
        self.info.get(host)
    }

    pub fn all_targets<'a>(
        &'a self,
        host: &'a str,
    ) -> impl Iterator<Item = (bool, &'a BuildInfo)> + use<'a> {
        self.info.iter().map(move |(&target, info)| {
            let is_host = target == host;

            (is_host, info)
        })
    }

    pub fn targets_and_paths(&self) -> impl Iterator<Item = (&'static str, &Path)> + use<'_> {
        self.info
            .iter()
            .map(|(key, info)| (*key, info.path.deref()))
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
        let info: Value = serde_json::from_str(&std::fs::read_to_string(path.as_ref())?)
            .expect("Invalid build info json");

        let settings = info["settings"].clone();
        let info = crate::build_info(&info);
        let libs = crate::find_all_libs(info.iter())?;

        Ok(Self {
            path: path.as_ref().to_owned(),
            info,
            libs,
            settings,
        })
    }

    pub fn target(&self) -> &'static str {
        Self::target_from_arch_and_os(
            self.settings["arch"].as_str().unwrap(),
            self.settings["os"].as_str().unwrap(),
        )
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

    pub fn write_env_source<W1, W2>(&self, is_host: bool, mut sh: W1, mut ps1: W2) -> io::Result<()>
    where
        W1: io::Write,
        W2: io::Write,
    {
        let prefix = self.target().replace('-', "_");

        writeln!(
            sh,
            "export {prefix}_CONANBUILDINFO={}",
            self.path.to_string_lossy()
        )?;
        writeln!(
            ps1,
            "$env:{prefix}_CONANBUILDINFO=\"{}\"",
            self.path.to_string_lossy()
        )?;

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
            writeln!(sh, "export LD_LIBRARY_PATH={libdirs}")?;
        }
        let bindirs = shared_deps
            .flat_map(|package| self.bindir_for(package))
            .collect::<Vec<_>>()
            .join(";")
            .replace('\\', "\\\\");

        if !bindirs.is_empty() && is_host {
            writeln!(ps1, "$env:PATH=\"{bindirs};$env:PATH\"")?;
        }

        if self.try_package("openssl").is_some() {
            let openssl_dir = self.rootpath_for("openssl");
            writeln!(sh, "export {prefix}_OPENSSL_DIR={openssl_dir}",)?;
            writeln!(ps1, "$env:{prefix}_OPENSSL_DIR=\"{openssl_dir}\"")?;

            if is_host {
                writeln!(sh, "export OPENSSL_DIR={openssl_dir}",)?;
                writeln!(ps1, "$env:OPENSSL_DIR=\"{openssl_dir}\"")?;
            }
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

    fn target_from_arch_and_os(arch: &str, os: &str) -> &'static str {
        match os {
            "Linux" => match arch {
                "x86_64" => "x86_64-unknown-linux-gnu",
                "x86" => "i686-unknown-linux-gnu",
                arch => unimplemented!("Unsupported architecture {arch:?}/{os:?}"),
            },
            "Windows" => match arch {
                "x86_64" => "x86_64-pc-windows-msvc",
                "x86" => "i686-pc-windows-msvc",
                arch => unimplemented!("Unsupported architecture {arch:?}/{os:?}"),
            },
            "Macos" => match arch {
                "armv8" => "aarch64-apple-darwin",
                "x86_64" => "x86_64-apple-darwin",
                arch => unimplemented!("Unsupported architecture {arch:?}/{os:?}"),
            },
            "iOS" => match arch {
                "armv8" => "aarch64-apple-ios",
                arch => unimplemented!("Unsupported architecture {arch:?}/{os:?}"),
            },
            "Android" => match arch {
                "armv8" => "aarch64-linux-android",
                "armv7" => "armv7-linux-androideabi",
                "x86" => "i686-linux-android",
                "x86_64" => "x86_64-linux-android",
                arch => unimplemented!("Unsupported architecture {arch:?}/{os:?}"),
            },
            os => unimplemented!("Unsupported OS {os:?}"),
        }
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
        let build_info_set = BuildInfoSet::find_all().expect("Failure reading conanbuildinfo");

        eprintln!("Targets:");
        for (target, path) in build_info_set.targets_and_paths() {
            eprintln!("    {target}: {}", path.to_string_lossy());
        }

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
        let build_info = self.build_info();
        println!(
            "cargo:rerun-if-changed={path}",
            path = build_info.path.to_string_lossy()
        );
    }

    pub fn build_info(&self) -> &BuildInfo {
        self.build_info_set
            .get_current_target(&self.host)
            .unwrap_or_else(|| {
                panic!(
                    "Could not find build info for {:?}, available are: {:?}",
                    self.host,
                    self.build_info_set.info.keys().collect::<Vec<_>>()
                )
            })
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

    pub fn generate_env_source(&self) {
        let mut sh = File::create("env.sh").unwrap();
        let mut ps1 = File::create("env.ps1").unwrap();

        for (is_host, info) in self.build_info_set.all_targets(&self.host) {
            info.write_env_source(is_host, &mut sh, &mut ps1).unwrap();
        }
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

fn find_all_libs<'a, I>(it: I) -> io::Result<HashMap<String, Link>>
where
    I: Iterator<Item = (&'a String, &'a Value)>,
{
    let mut result = HashMap::new();
    for (_, v) in it {
        for path in BuildInfo::libdir_for_package(v) {
            let libs = Path::new(path)
                .read_dir()
                .map_err(|e| {
                    io::Error::new(e.kind(), format!("Failure reading dir {path:?}: {e}"))
                })?
                .filter_map(|entry| {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(e) => return Some(Err(e)),
                    };

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

                        return Some(Ok((lib.to_string(), link)));
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

                    Some(Ok((lib.to_string(), link)))
                });

            for lib_r in libs {
                let (key, lib) = lib_r?;
                result.insert(key, lib);
            }
        }
    }

    Ok(result)
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
