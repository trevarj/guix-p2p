use std::process::Stdio;

use anyhow::Context;

/// Restore a NAR into a Guix store destination path.
///
/// Guix's substitute protocol passes a destination path to the substituter and
/// expects the substituter to materialize the NAR contents there. Writing raw
/// NAR bytes at that path creates an invalid regular file.
pub async fn restore_nar_to_destination(
    nar_path: &std::path::Path,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    let nar_path = nar_path.to_path_buf();
    let dest = dest.to_path_buf();

    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::task::spawn_blocking(move || restore_nar_to_destination_sync(&nar_path, &dest)),
    )
    .await
    .context("timed out restoring nar")?
    .context("nar restore task failed")?
}

fn restore_nar_to_destination_sync(
    nar_path: &std::path::Path,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    let _ = std::fs::remove_file(dest);
    let _ = std::fs::remove_dir_all(dest);

    let program = if std::path::Path::new("/run/current-system/profile/bin/guix").exists() {
        "/run/current-system/profile/bin/guix"
    } else {
        "guix"
    };
    let script_path = nar_path.with_extension("restore.scm");

    let script = format!(
        "(use-modules (guix serialization))(let ((port (open-file {} \"rb\")))(dynamic-wind(const \
         #t)(lambda () (restore-file port {}))(lambda () (close-port port))))",
        scheme_string(&nar_path.display().to_string()),
        scheme_string(&dest.display().to_string())
    );

    std::fs::write(&script_path, script)
        .with_context(|| format!("failed to write restore helper {}", script_path.display()))?;

    let output = std::process::Command::new(program)
        .arg("repl")
        .arg("--")
        .arg(&script_path)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run {program} repl to restore nar"))?;

    let _ = std::fs::remove_file(&script_path);

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "failed to restore nar into {}: {}",
            dest.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

fn scheme_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::scheme_string;

    #[test]
    fn quotes_scheme_strings() {
        assert_eq!(scheme_string("/tmp/simple"), "\"/tmp/simple\"");
        assert_eq!(scheme_string("/tmp/a\"b\\c"), "\"/tmp/a\\\"b\\\\c\"");
    }
}
