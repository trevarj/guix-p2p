pub fn store_hash_part(store_path: &str) -> Option<String> {
    guix_p2p::store_path::hash_part(store_path).ok()
}

pub fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

pub fn json_string(value: &serde_json::Value, field: &str) -> Option<String> {
    value.get(field)?.as_str().map(str::to_string)
}

pub fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub fn sanitize_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

pub fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or("")
}

pub fn read_tail(path: &std::path::Path, max_lines: usize) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    lines[lines.len().saturating_sub(max_lines)..].join("\n")
}

pub fn unix_timestamp() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => format!("{} seconds since 1970-01-01 UTC", duration.as_secs()),
        Err(_) => "unknown".to_string(),
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let value = bytes as f64;
    match value {
        value if value >= GIB => format!("{:.2} GiB", value / GIB),
        value if value >= MIB => format!("{:.2} MiB", value / MIB),
        value if value >= KIB => format!("{:.2} KiB", value / KIB),
        _ => format!("{bytes} B"),
    }
}

pub fn parse_key_line(output: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    output.lines().filter_map(|line| line.strip_prefix(&prefix)).next_back().map(str::to_string)
}
