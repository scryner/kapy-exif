use std::env;
use std::path::PathBuf;

const PATH_SEPARATOR: &str = match cfg!(target_os = "windows") {
    true => ";",
    _ => ":",
};

fn main() {
    // find exiv2
    let exiv2_inc_dirs = find_library(
        FromPkgConfig {
            name: "exiv2".to_string(),
            atleast_version: "0.27.6".to_string(),
        },
        FromEnv {
            env_key_include_dirs: "EXIV2_INCLUDE_DIRS".to_string(),
            env_key_lib_dirs: "EXIV2_LIB_DIRS".to_string(),
            libs: vec!["exiv2".to_string()],
        },
    )
    .unwrap();

    // find libssh
    let libssh_inc_dirs = find_library(
        FromPkgConfig {
            name: "libssh".to_string(),
            atleast_version: "0.10.4".to_string(),
        },
        FromEnv {
            env_key_include_dirs: "LIBSSH_INCLUDE_DIRS".to_string(),
            env_key_lib_dirs: "LIBSSH_LIB_DIRS".to_string(),
            libs: vec!["ssh".to_string()],
        },
    )
    .unwrap();

    // find libheif
    find_library(
        FromPkgConfig {
            name: "libheif".to_string(),
            atleast_version: "1.15.2".to_string(),
        },
        FromEnv {
            env_key_include_dirs: "LIBHEIF_INCLUDE_DIRS".to_string(),
            env_key_lib_dirs: "LIBHEIF_LIB_DIRS".to_string(),
            libs: vec!["heif".to_string()],
        },
    )
    .unwrap();

    // compile c files
    cc::Build::new()
        .cpp(true)
        .flag("-std=c++17")
        .file("lib/exif.cpp")
        .include("lib")
        .includes(exiv2_inc_dirs)
        .includes(libssh_inc_dirs)
        .compile("libexif");

    println!("cargo:rerun-if-changed=lib/exif.h");
    println!("cargo:rerun-if-changed=lib/exif.cpp");
}

struct FromPkgConfig {
    name: String,

    #[allow(unused)]
    atleast_version: String,
}

struct FromEnv {
    env_key_include_dirs: String,
    env_key_lib_dirs: String,
    libs: Vec<String>,
}

fn find_library(from_pkg_config: FromPkgConfig, from_env: FromEnv) -> Result<Vec<PathBuf>, String> {
    // try to find from env
    match find_library_from_env(&from_env) {
        Ok(inc_dirs) => return Ok(inc_dirs),
        _ => (),
    }

    // try to find from pkg_config
    find_library_internal(&from_pkg_config)
}

#[cfg(not(target_os = "windows"))]
fn find_library_internal(from_pkg_config: &FromPkgConfig) -> Result<Vec<PathBuf>, String> {
    let name = from_pkg_config.name.as_str();

    let library = pkg_config::Config::new()
        .atleast_version(from_pkg_config.atleast_version.as_str())
        .probe(name)
        .map_err(|e| format!("Failed to find library '{}' from pkg_config: {}", name, e))?;

    Ok(library.include_paths)
}

#[cfg(target_os = "windows")]
fn find_library_internal(from_pkg_config: &FromPkgConfig) -> Result<Vec<PathBuf>, String> {
    let name = from_pkg_config.name.as_str();

    let library = vcpkg::find_package(name)
        .map_err(|e| format!("Failed to find library '{}' from pkg_config: {}", name, e))?;

    Ok(library.include_paths)
}

fn find_library_from_env(from_env: &FromEnv) -> Result<Vec<PathBuf>, String> {
    let include_dirs = verify_directories_from_env(from_env.env_key_include_dirs.as_str())?;
    for dir in include_dirs.iter() {
        println!("cargo:rustc-link-search=native={}", dir.to_str().unwrap())
    }

    let lib_dirs = verify_directories_from_env(from_env.env_key_lib_dirs.as_str())?;
    for dir in lib_dirs.iter() {
        println!("cargo:rustc-link-search=native={}", dir.to_str().unwrap())
    }

    for lib in from_env.libs.iter() {
        println!("cargo:rustc-link-lib={}", lib);
    }

    Ok(include_dirs)
}

fn verify_directories_from_env(env_key: &str) -> Result<Vec<PathBuf>, String> {
    println!("cargo:rerun-if-env-changed={}", env_key);

    match env::var(env_key) {
        Ok(val) => {
            let dirs: Vec<String> = val.split(PATH_SEPARATOR).map(|x| x.to_string()).collect();

            let mut paths = Vec::new();

            for dir in dirs.iter() {
                let path = PathBuf::from(dir);
                let meta = match path.metadata() {
                    Ok(meta) => meta,
                    Err(e) => {
                        return Err(format!("Failed to get metadata from '{}': {}", dir, e));
                    }
                };

                if !meta.is_dir() {
                    return Err(format!("'{}' is not directory", dir));
                }

                paths.push(path);
            }

            Ok(paths)
        }
        Err(e) => Err(format!("failed to verify directories from env: {}", e)),
    }
}
