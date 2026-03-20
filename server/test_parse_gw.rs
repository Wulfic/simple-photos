fn get_default_gateway() -> Option<String> {
    if let Ok(content) = std::fs::read_to_string("/proc/net/route") {
        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && parts[1] == "00000000" {
                let gw_hex = parts[2];
                if gw_hex.len() == 8 {
                    if let Ok(ip) = u32::from_str_radix(gw_hex, 16) {
                        let bytes = ip.to_le_bytes();
                        return Some(format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3]));
                    }
                }
            }
        }
    }
    None
}
fn main() {
    println!("{:?}", get_default_gateway());
}
