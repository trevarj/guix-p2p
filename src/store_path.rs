/// Return the hash prefix from a Guix store path.
pub fn hash_part(store_path: &str) -> Result<String, String> {
    let rest = store_path
        .strip_prefix("/gnu/store/")
        .ok_or_else(|| format!("not a store path: {store_path}"))?;
    let (hash, _) =
        rest.split_once('-').ok_or_else(|| format!("no name separator in: {store_path}"))?;
    Ok(hash.to_string())
}

/// Convert narinfo basenames to full `/gnu/store/...` paths.
pub fn from_narinfo_path(path_or_basename: &str) -> String {
    match path_or_basename {
        "" => String::new(),
        path if path.starts_with("/gnu/store/") => path.to_string(),
        basename => format!("/gnu/store/{basename}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_hash_part() {
        assert_eq!(
            hash_part("/gnu/store/abc123def456ghi789jkl012mno345pq-foo-1.0").unwrap(),
            "abc123def456ghi789jkl012mno345pq"
        );
    }

    #[test]
    fn rejects_non_store_paths() {
        assert!(hash_part("/bad/path").is_err());
        assert!(hash_part("/gnu/store/abcdefnodash").is_err());
    }

    #[test]
    fn prefixes_narinfo_basenames() {
        assert_eq!(
            from_narinfo_path("x0qpkx4qcd7pzn121bg5plm67jf0icbz-gash-utils-0.2.0.tar.gz.drv"),
            "/gnu/store/x0qpkx4qcd7pzn121bg5plm67jf0icbz-gash-utils-0.2.0.tar.gz.drv"
        );
        assert_eq!(
            from_narinfo_path("/gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2"),
            "/gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2"
        );
        assert_eq!(from_narinfo_path(""), "");
    }
}
