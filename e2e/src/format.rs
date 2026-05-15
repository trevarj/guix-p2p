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
        Ok(duration) => format_unix_timestamp_utc(duration.as_secs()),
        Err(_) => "unknown".to_string(),
    }
}

fn format_unix_timestamp_utc(timestamp: u64) -> String {
    const SECONDS_PER_DAY: u64 = 86_400;
    let days = timestamp / SECONDS_PER_DAY;
    let seconds_of_day = timestamp % SECONDS_PER_DAY;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let (year, month, day) = civil_from_unix_days(days as i64);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

// Howard Hinnant's civil-from-days algorithm, adjusted from 1970-01-01.
fn civil_from_unix_days(days_since_epoch: i64) -> (i64, u64, u64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month as u64, day as u64)
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

#[cfg(test)]
mod tests {
    use super::format_unix_timestamp_utc;

    #[test]
    fn formats_unix_epoch_as_utc_datetime() {
        assert_eq!(format_unix_timestamp_utc(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn formats_recent_unix_timestamp_as_utc_datetime() {
        assert_eq!(format_unix_timestamp_utc(1_778_794_987), "2026-05-14 21:43:07 UTC");
    }
}
