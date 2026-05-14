use std::{
    collections::HashSet,
    env,
    path::{Path as FsPath, PathBuf},
    process::Command,
};

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct ApiPackage {
    pub source: String,
    pub name: String,
    pub version: String,
    pub output: String,
    pub store_path: String,
    pub seeded: bool,
}

pub(super) fn package_profiles() -> Vec<(String, PathBuf)> {
    package_profiles_for_home(env::var_os("HOME").map(PathBuf::from))
}

pub(super) fn package_profiles_for_home(home: Option<PathBuf>) -> Vec<(String, PathBuf)> {
    [
        Some(("system".to_string(), PathBuf::from("/run/current-system/profile"))),
        Some(("kernel".to_string(), PathBuf::from("/run/current-system/kernel"))),
        home.map(|home| ("home".to_string(), home.join(".guix-home/profile"))),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub(super) fn installed_packages_from_profile(
    source: &str,
    profile: &FsPath,
    seeded_store_paths: &HashSet<String>,
) -> Vec<ApiPackage> {
    if !profile.exists() {
        return Vec::new();
    }

    let output = match Command::new("guix")
        .arg("package")
        .arg("--list-installed")
        .arg(format!("--profile={}", profile.display()))
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            tracing::warn!(
                "failed to list Guix packages for {} profile {}: status {}",
                source,
                profile.display(),
                output.status,
            );
            return Vec::new();
        },
        Err(e) => {
            tracing::warn!(
                "failed to run guix package for {} profile {}: {}",
                source,
                profile.display(),
                e,
            );
            return Vec::new();
        },
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| parse_installed_package(source, line, seeded_store_paths))
        .collect()
}

pub(super) fn parse_installed_package(
    source: &str,
    line: &str,
    seeded_store_paths: &HashSet<String>,
) -> Option<ApiPackage> {
    let mut fields = line.split('\t').map(str::trim);
    let (name, version, output, store_path) =
        (fields.next()?, fields.next()?, fields.next()?, fields.next()?);

    if [name, version, output, store_path].iter().any(|field| field.is_empty()) {
        return None;
    }

    Some(ApiPackage {
        source: source.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        output: output.to_string(),
        store_path: store_path.to_string(),
        seeded: seeded_store_paths.contains(store_path),
    })
}
