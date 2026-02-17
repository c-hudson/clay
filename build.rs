use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::Path;

fn main() {
    // Collect all source files for hashing
    let mut files: BTreeSet<String> = BTreeSet::new();
    collect_source_files(Path::new("src"), &mut files);

    // Also include build.rs itself and Cargo.toml
    files.insert("build.rs".to_string());
    files.insert("Cargo.toml".to_string());

    // Compute hash of all file contents and find most recent modification time
    let mut hasher = Sha256::new();
    let mut newest_mtime: u64 = 0;
    for file_path in &files {
        if let Ok(mut file) = fs::File::open(file_path) {
            let mut contents = Vec::new();
            if file.read_to_end(&mut contents).is_ok() {
                hasher.update(&contents);
            }
        }
        if let Ok(metadata) = fs::metadata(file_path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                    let secs = duration.as_secs();
                    if secs > newest_mtime {
                        newest_mtime = secs;
                    }
                }
            }
        }
    }

    let hash = hasher.finalize();
    let hash_hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
    let short_hash = &hash_hex[..8];

    // Format the newest modification time as YY/MM/DD HH:MM in local time
    let build_date = format_unix_timestamp(newest_mtime);

    println!("cargo:rustc-env=BUILD_HASH={}", short_hash);
    println!("cargo:rustc-env=BUILD_DATE={}", build_date);

    // Rerun if any source file changes
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    // On Windows, embed the application icon into the executable
    #[cfg(target_os = "windows")]
    embed_windows_icon();
}

/// Embed Windows PE metadata (version info, description) and optionally an icon.
/// PE metadata helps AV heuristics trust the binary and reduces false positives.
#[cfg(target_os = "windows")]
fn embed_windows_icon() {
    let mut res = winres::WindowsResource::new();

    // PE version info — reduces AV heuristic false positives
    res.set("FileDescription", "Clay - A MUD Client");
    res.set("ProductName", "Clay");
    res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
    res.set("FileVersion", env!("CARGO_PKG_VERSION"));
    res.set("CompanyName", "Clay MUD Client");
    res.set("LegalCopyright", "Copyright (c) 2024-2026 Clay contributors");
    res.set("OriginalFilename", "clay.exe");
    res.set("InternalName", "clay");

    // Optionally embed application icon from clay_icon.png
    let png_path = Path::new("clay_icon.png");
    if png_path.exists() {
        if let Some(ico_path) = build_ico_from_png(png_path) {
            res.set_icon(ico_path.to_str().unwrap());
        }
        println!("cargo:rerun-if-changed=clay_icon.png");
    }

    if let Err(e) = res.compile() {
        eprintln!("cargo:warning=Failed to compile Windows resource: {}", e);
    }
}

/// Convert a PNG file to ICO format (Vista+ PNG-in-ICO).
/// Returns the path to the generated ICO file, or None on failure.
#[cfg(target_os = "windows")]
fn build_ico_from_png(png_path: &Path) -> Option<std::path::PathBuf> {
    let out_dir = std::env::var("OUT_DIR").ok()?;
    let out_path = Path::new(&out_dir);

    // Read the PNG file
    let mut png_data = Vec::new();
    fs::File::open(png_path)
        .and_then(|mut f| f.read_to_end(&mut png_data))
        .ok()?;

    // Parse PNG dimensions from IHDR chunk (bytes 16-23)
    if png_data.len() < 24 {
        return None;
    }
    let width = u32::from_be_bytes([png_data[16], png_data[17], png_data[18], png_data[19]]);
    let height = u32::from_be_bytes([png_data[20], png_data[21], png_data[22], png_data[23]]);

    // Build ICO file: header + one directory entry + PNG data
    // ICO files can embed PNG data directly (Vista+ format)
    let ico_header_size: u32 = 6 + 16; // ICO header (6) + one dir entry (16)
    let mut ico = Vec::new();

    // ICO header: reserved(2) + type=1(2) + count=1(2)
    ico.extend_from_slice(&[0, 0]); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    ico.extend_from_slice(&1u16.to_le_bytes()); // count: 1 image

    // Directory entry: w, h, colors, reserved, planes, bpp, size, offset
    let w = if width >= 256 { 0u8 } else { width as u8 };
    let h = if height >= 256 { 0u8 } else { height as u8 };
    ico.push(w);
    ico.push(h);
    ico.push(0); // color palette count (0 = no palette)
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    ico.extend_from_slice(&(png_data.len() as u32).to_le_bytes()); // image size
    ico.extend_from_slice(&ico_header_size.to_le_bytes()); // offset to image data

    // Append the raw PNG data
    ico.extend_from_slice(&png_data);

    let ico_path = out_path.join("clay_icon.ico");
    fs::write(&ico_path, &ico).ok()?;
    Some(ico_path)
}

/// Format a Unix timestamp as "YY/MM/DD HH:MM" in local time.
/// No external dependencies — computes local time from /etc/localtime or TZ env.
fn format_unix_timestamp(timestamp: u64) -> String {
    // Try to get UTC offset from TZ environment or /etc/localtime
    let offset_secs = get_local_utc_offset();
    let local_ts = timestamp as i64 + offset_secs;

    // Convert to calendar date/time (civil time from Unix epoch)
    let secs_per_day: i64 = 86400;
    let mut days = local_ts / secs_per_day;
    let mut time_of_day = local_ts % secs_per_day;
    if time_of_day < 0 {
        days -= 1;
        time_of_day += secs_per_day;
    }

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;

    // Days since Unix epoch (Jan 1, 1970) to (year, month, day)
    // Algorithm from Howard Hinnant's chrono-compatible date library
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:02}/{:02}/{:02} {:02}:{:02}", y % 100, m, d, hours, minutes)
}

/// Get the local UTC offset in seconds by reading /etc/localtime TZif file.
fn get_local_utc_offset() -> i64 {
    // Try TZ environment variable for simple offset formats
    if let Ok(tz) = std::env::var("TZ") {
        if let Some(offset) = parse_simple_tz_offset(&tz) {
            return offset;
        }
    }

    // Try reading /etc/localtime (TZif binary format)
    if let Ok(data) = fs::read("/etc/localtime") {
        if let Some(offset) = parse_tzif_offset(&data) {
            return offset;
        }
    }

    0 // Fall back to UTC
}

/// Parse simple TZ offset like "EST5EDT" or "UTC-5" — returns offset in seconds
fn parse_simple_tz_offset(tz: &str) -> Option<i64> {
    // Look for a digit or minus sign after alphabetic chars
    let bytes = tz.as_bytes();
    let mut i = 0;
    // Skip alphabetic prefix
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    // Parse number (POSIX TZ offset is WEST-positive, so negate for UTC offset)
    let remaining = &tz[i..];
    // Find end of number part
    let mut end = 0;
    if end < remaining.len() && (remaining.as_bytes()[end] == b'-' || remaining.as_bytes()[end] == b'+') {
        end += 1;
    }
    while end < remaining.len() && remaining.as_bytes()[end].is_ascii_digit() {
        end += 1;
    }
    let num_str = &remaining[..end];
    let hours: i64 = num_str.parse().ok()?;
    Some(-hours * 3600) // POSIX TZ is west-positive
}

/// Parse a TZif (timezone info) binary file and return the last transition's UTC offset.
fn parse_tzif_offset(data: &[u8]) -> Option<i64> {
    // TZif format: "TZif" magic, then header with counts
    if data.len() < 44 || &data[0..4] != b"TZif" {
        return None;
    }
    let version = data[4];

    // For TZif2/3 (version '2' or '3'), skip v1 data and parse v2 header
    if version == b'2' || version == b'3' {
        // v1 header counts (at offset 20)
        let tzh_timecnt = u32::from_be_bytes(data[32..36].try_into().ok()?) as usize;
        let tzh_typecnt = u32::from_be_bytes(data[36..40].try_into().ok()?) as usize;
        let tzh_charcnt = u32::from_be_bytes(data[40..44].try_into().ok()?) as usize;
        // v1 data size: timecnt*4 (times) + timecnt*1 (type indices) + typecnt*6 (ttinfos) + charcnt
        let v1_data_size = tzh_timecnt * 5 + tzh_typecnt * 6 + tzh_charcnt;
        // Also account for leap seconds, std/wall, ut/local indicators
        let tzh_leapcnt = u32::from_be_bytes(data[28..32].try_into().ok()?) as usize;
        let tzh_ttisstdcnt = u32::from_be_bytes(data[24..28].try_into().ok()?) as usize;
        let tzh_ttisutcnt = u32::from_be_bytes(data[20..24].try_into().ok()?) as usize;
        let v1_total = 44 + v1_data_size + tzh_leapcnt * 8 + tzh_ttisstdcnt + tzh_ttisutcnt;

        if data.len() > v1_total + 44 && &data[v1_total..v1_total + 4] == b"TZif" {
            // Parse v2 header (uses 8-byte timestamps)
            let v2 = &data[v1_total..];
            return parse_tzif_block(v2, 8);
        }
    }

    parse_tzif_block(data, 4)
}

/// Parse a TZif data block and return the last ttinfo UTC offset.
/// `time_size` is 4 for v1 (32-bit timestamps) or 8 for v2/v3 (64-bit timestamps).
fn parse_tzif_block(data: &[u8], time_size: usize) -> Option<i64> {
    if data.len() < 44 {
        return None;
    }
    let tzh_timecnt = u32::from_be_bytes(data[32..36].try_into().ok()?) as usize;
    let tzh_typecnt = u32::from_be_bytes(data[36..40].try_into().ok()?) as usize;
    if tzh_typecnt == 0 {
        return None;
    }

    // Time values start at offset 44 (each is `time_size` bytes)
    let times_start = 44;
    // Type indices follow times
    let types_start = times_start + tzh_timecnt * time_size;
    // TTInfo structs follow type indices (each is 6 bytes: i32 utoff, u8 dst, u8 idx)
    let ttinfos_start = types_start + tzh_timecnt;

    if data.len() < ttinfos_start + tzh_typecnt * 6 {
        return None;
    }

    // Get the type index of the last transition (or type 0 if no transitions)
    let type_idx = if tzh_timecnt > 0 {
        data[types_start + tzh_timecnt - 1] as usize
    } else {
        0
    };

    if type_idx >= tzh_typecnt {
        return None;
    }

    // Read UTC offset from ttinfo
    let ttinfo_offset = ttinfos_start + type_idx * 6;
    let utoff = i32::from_be_bytes(data[ttinfo_offset..ttinfo_offset + 4].try_into().ok()?);
    Some(utoff as i64)
}

fn collect_source_files(dir: &Path, files: &mut BTreeSet<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_source_files(&path, files);
            } else if path.extension().map(|e| e == "rs" || e == "html" || e == "css" || e == "js").unwrap_or(false) {
                if let Some(path_str) = path.to_str() {
                    files.insert(path_str.to_string());
                }
            }
        }
    }
}

// Simple SHA256 implementation (no external dependencies in build script)
struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: Vec::new(),
            total_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
        self.total_len += data.len() as u64;

        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().unwrap();
            self.process_block(&block);
            self.buffer.drain(..64);
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len * 8;
        self.buffer.push(0x80);
        while (self.buffer.len() % 64) != 56 {
            self.buffer.push(0);
        }
        self.buffer.extend_from_slice(&bit_len.to_be_bytes());

        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().unwrap();
            self.process_block(&block);
            self.buffer.drain(..64);
        }

        let mut result = [0u8; 32];
        for (i, &val) in self.state.iter().enumerate() {
            result[i * 4..(i + 1) * 4].copy_from_slice(&val.to_be_bytes());
        }
        result
    }

    fn process_block(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g; g = f; f = e;
            e = d.wrapping_add(temp1);
            d = c; c = b; b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}
