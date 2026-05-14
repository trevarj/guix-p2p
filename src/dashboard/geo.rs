pub fn extract_addr_info(addrs: &[String]) -> (Option<String>, Option<String>) {
    addrs
        .iter()
        .find_map(|addr| extract_ip_from_multiaddr(addr))
        .map(|ip| {
            let country = ip_to_country(&ip).map(str::to_string);
            (Some(ip), country)
        })
        .unwrap_or((None, None))
}

fn extract_ip_from_multiaddr(addr: &str) -> Option<String> {
    ["/ip4/", "/ip6/"].iter().find_map(|marker| {
        let rest = addr.split_once(marker)?.1;
        Some(rest.split('/').next()?.to_string())
    })
}

fn ip_to_country(ip: &str) -> Option<&'static str> {
    let first_octet = ip.split('.').next()?.parse::<u8>().ok()?;
    match first_octet {
        5 => Some("DE"),
        14 | 27 | 49 | 58 | 60 | 101 | 103 | 106 | 110..=126 => Some("JP"),
        31 => Some("NL"),
        36 | 68 => Some("CA"),
        41 => Some("KE"),
        46 => Some("SE"),
        51 => Some("NO"),
        62 => Some("IT"),
        77..=83 | 89..=95 | 176 | 178 | 212 | 213 => Some("RU"),
        84 => Some("ES"),
        85 | 86 => Some("CH"),
        87 => Some("DK"),
        88 => Some("PL"),
        102 => Some("ZA"),
        105 => Some("FR"),
        109 => Some("IL"),
        151 | 181 | 189 | 190 | 200 | 201 => Some("BR"),
        152 => Some("MX"),
        154 => Some("TR"),
        177 => Some("AR"),
        179 => Some("PE"),
        184 => Some("CL"),
        185 => Some("CZ"),
        187 => Some("PT"),
        188 => Some("RO"),
        191 => Some("CO"),
        193 => Some("HU"),
        194 => Some("AT"),
        195 => Some("GR"),
        196 => Some("UA"),
        197 => Some("NG"),
        214..=215 => Some("PH"),
        217 => Some("BE"),
        1..=4
        | 8
        | 13
        | 20..=24
        | 40
        | 44
        | 45
        | 47
        | 50
        | 52
        | 54
        | 63..=67
        | 69
        | 71..=76
        | 96..=100
        | 104
        | 107
        | 108
        | 128..=150
        | 153
        | 155..=175
        | 180
        | 182
        | 183
        | 186
        | 192
        | 198
        | 199
        | 204..=209
        | 216 => Some("US"),
        _ => None,
    }
}

pub fn country_flag(code: &str) -> String {
    let [a, b] = code.as_bytes() else {
        return String::new();
    };
    let a = a.to_ascii_uppercase();
    let b = b.to_ascii_uppercase();

    if !a.is_ascii_uppercase() || !b.is_ascii_uppercase() {
        return String::new();
    }

    format!(
        "{}{}",
        char::from_u32(0x1F1E6 + (a - b'A') as u32).unwrap_or(' '),
        char::from_u32(0x1F1E6 + (b - b'A') as u32).unwrap_or(' '),
    )
}
