use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, ReadHalf, WriteHalf};
use tokio::net::TcpStream;

#[cfg(feature = "native-tls-backend")]
use tokio_native_tls::TlsStream;

#[cfg(feature = "rustls-backend")]
use tokio_rustls::client::TlsStream;

// Telnet protocol constants
pub const TELNET_IAC: u8 = 255;  // Interpret As Command
pub const TELNET_DONT: u8 = 254;
pub const TELNET_DO: u8 = 253;
pub const TELNET_WONT: u8 = 252;
pub const TELNET_WILL: u8 = 251;
pub const TELNET_SB: u8 = 250;   // Subnegotiation Begin
pub const TELNET_GA: u8 = 249;   // Go Ahead (prompt marker)
pub const TELNET_EOR: u8 = 239;  // End of Record (alternative prompt marker)
pub const TELNET_SE: u8 = 240;   // Subnegotiation End
pub const TELNET_NOP: u8 = 241;  // No Operation (keepalive)

// Telnet options
pub const TELNET_OPT_ECHO: u8 = 1;    // Echo option
pub const TELNET_OPT_SGA: u8 = 3;     // Suppress Go Ahead
pub const TELNET_OPT_TTYPE: u8 = 24;  // Terminal Type
pub const TELNET_OPT_EOR: u8 = 25;    // End of Record
pub const TELNET_OPT_NAWS: u8 = 31;   // Negotiate About Window Size
pub const TELNET_OPT_MSDP: u8 = 69;   // MUD Server Data Protocol
pub const TELNET_OPT_GMCP: u8 = 201;  // Generic MUD Communication Protocol

// MSDP sub-negotiation markers
pub const MSDP_VAR: u8 = 1;
pub const MSDP_VAL: u8 = 2;
pub const MSDP_TABLE_OPEN: u8 = 3;
pub const MSDP_TABLE_CLOSE: u8 = 4;
pub const MSDP_ARRAY_OPEN: u8 = 5;
pub const MSDP_ARRAY_CLOSE: u8 = 6;

// TTYPE subnegotiation commands
pub const TTYPE_IS: u8 = 0;    // Terminal type IS (response)
pub const TTYPE_SEND: u8 = 1;  // Send terminal type (request)

/// Command types for the writer task
pub enum WriteCommand {
    Text(String),     // Regular command (will add \r\n)
    Raw(Vec<u8>),     // Raw bytes (for telnet responses and NOP)
    Shutdown,         // Close the connection gracefully
}

/// Stream wrapper enums for supporting both plain TCP and TLS connections
pub enum StreamReader {
    Plain(tokio::net::tcp::OwnedReadHalf),
    Tls(ReadHalf<TlsStream<TcpStream>>),
    #[cfg(unix)]
    Proxy(tokio::net::unix::OwnedReadHalf),  // Unix socket for TLS proxy
}

pub enum StreamWriter {
    Plain(tokio::net::tcp::OwnedWriteHalf),
    Tls(WriteHalf<TlsStream<TcpStream>>),
    #[cfg(unix)]
    Proxy(tokio::net::unix::OwnedWriteHalf),  // Unix socket for TLS proxy
}

impl AsyncRead for StreamReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            StreamReader::Plain(s) => Pin::new(s).poll_read(cx, buf),
            StreamReader::Tls(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(unix)]
            StreamReader::Proxy(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for StreamWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            StreamWriter::Plain(s) => Pin::new(s).poll_write(cx, buf),
            StreamWriter::Tls(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(unix)]
            StreamWriter::Proxy(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            StreamWriter::Plain(s) => Pin::new(s).poll_flush(cx),
            StreamWriter::Tls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(unix)]
            StreamWriter::Proxy(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            StreamWriter::Plain(s) => Pin::new(s).poll_shutdown(cx),
            StreamWriter::Tls(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(unix)]
            StreamWriter::Proxy(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Result of processing telnet sequences
pub struct TelnetResult {
    pub cleaned: Vec<u8>,       // Data with telnet sequences removed
    pub responses: Vec<u8>,     // Bytes to send back (WILL/WONT/DO/DONT responses)
    pub telnet_detected: bool,  // True if any telnet IAC sequences were found
    pub prompt: Option<Vec<u8>>, // Text from last newline to GA/EOR/WONT_ECHO, if found
    pub wont_echo_seen: bool,   // True if IAC WONT ECHO was received
    pub naws_requested: bool,   // True if server sent DO NAWS (we responded WILL NAWS)
    pub ttype_requested: bool,  // True if server sent SB TTYPE SEND (we need to send terminal type)
    pub gmcp_data: Vec<(String, String)>,  // (package.message, json_data)
    pub msdp_data: Vec<(String, String)>,  // (variable_name, value_json)
    pub gmcp_negotiated: bool,  // True if server sent WILL GMCP
    pub msdp_negotiated: bool,  // True if server sent WILL MSDP
}

/// Process telnet sequences in incoming data.
/// Returns TelnetResult with cleaned data and negotiation info.
pub fn process_telnet(data: &[u8]) -> TelnetResult {
    let mut cleaned = Vec::with_capacity(data.len());
    let mut responses = Vec::new();
    let mut telnet_detected = false;
    let mut prompt: Option<Vec<u8>> = None;
    let mut wont_echo_seen = false;
    let mut naws_requested = false;
    let mut ttype_requested = false;
    let mut gmcp_data = Vec::new();
    let mut msdp_data = Vec::new();
    let mut gmcp_negotiated = false;
    let mut msdp_negotiated = false;
    let mut i = 0;

    while i < data.len() {
        if data[i] == TELNET_IAC {
            telnet_detected = true;
            if i + 1 >= data.len() {
                break; // Incomplete sequence
            }
            let cmd = data[i + 1];
            match cmd {
                TELNET_IAC => {
                    // Escaped 255 byte
                    cleaned.push(TELNET_IAC);
                    i += 2;
                }
                TELNET_WILL | TELNET_WONT | TELNET_DO | TELNET_DONT => {
                    if i + 2 >= data.len() {
                        break; // Incomplete sequence
                    }
                    let option = data[i + 2];
                    // Respond based on option
                    match cmd {
                        TELNET_WILL => {
                            // Server wants to enable an option - we accept some
                            if option == TELNET_OPT_SGA || option == TELNET_OPT_EOR {
                                // Accept Suppress Go Ahead and End of Record
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_DO, option]);
                            } else if option == TELNET_OPT_GMCP {
                                // Accept GMCP
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_DO, option]);
                                gmcp_negotiated = true;
                            } else if option == TELNET_OPT_MSDP {
                                // Accept MSDP
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_DO, option]);
                                msdp_negotiated = true;
                            } else {
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_DONT, option]);
                            }
                        }
                        TELNET_DO => {
                            // Server wants us to enable an option
                            if option == TELNET_OPT_NAWS {
                                // Accept NAWS - we'll send window size
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_WILL, option]);
                                naws_requested = true;
                            } else if option == TELNET_OPT_TTYPE {
                                // Accept TTYPE - server will send subnegotiation to request type
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_WILL, option]);
                            } else if option == TELNET_OPT_EOR {
                                // Accept EOR - we'll handle IAC EOR as prompt marker
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_WILL, option]);
                            } else {
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_WONT, option]);
                            }
                        }
                        TELNET_WONT if option == TELNET_OPT_ECHO => {
                            // WONT ECHO often precedes login/password prompts
                            // Mark that we saw it - we'll extract prompt at end
                            wont_echo_seen = true;
                        }
                        _ => {} // Other WONT/DONT - no response needed
                    }
                    i += 3;
                }
                TELNET_SB => {
                    // Subnegotiation - parse the content
                    let sb_start = i + 2;
                    i += 2;
                    // Find IAC SE that ends subnegotiation
                    while i < data.len() {
                        if data[i] == TELNET_IAC && i + 1 < data.len() {
                            if data[i + 1] == TELNET_SE {
                                // Found end of subnegotiation
                                let sb_data = &data[sb_start..i];
                                // Check for TTYPE SEND request
                                if sb_data.len() >= 2 && sb_data[0] == TELNET_OPT_TTYPE && sb_data[1] == TTYPE_SEND {
                                    ttype_requested = true;
                                }
                                // Check for GMCP data (option 201)
                                if sb_data.len() >= 2 && sb_data[0] == TELNET_OPT_GMCP {
                                    let payload = &sb_data[1..];
                                    // GMCP format: "package.message json_data"
                                    // Split at first space to separate package from JSON
                                    if let Ok(text) = std::str::from_utf8(payload) {
                                        let (package, json) = if let Some(pos) = text.find(' ') {
                                            (text[..pos].to_string(), text[pos+1..].trim().to_string())
                                        } else {
                                            (text.to_string(), String::new())
                                        };
                                        gmcp_data.push((package, json));
                                    }
                                }
                                // Check for MSDP data (option 69)
                                if sb_data.len() >= 2 && sb_data[0] == TELNET_OPT_MSDP {
                                    let payload = &sb_data[1..];
                                    let pairs = parse_msdp_pairs(payload);
                                    msdp_data.extend(pairs);
                                }
                                i += 2;
                                break;
                            } else if data[i + 1] == TELNET_IAC {
                                // Escaped 0xFF - skip the doubled byte
                                i += 2;
                                continue;
                            }
                        }
                        i += 1;
                    }
                }
                TELNET_GA | TELNET_EOR => {
                    // Go Ahead or End of Record - extract prompt (text from last newline to here)
                    // Find last newline in cleaned data
                    let last_newline = cleaned.iter().rposition(|&b| b == b'\n');
                    let prompt_start = last_newline.map(|p| p + 1).unwrap_or(0);

                    if prompt_start < cleaned.len() {
                        // Extract the prompt text
                        prompt = Some(cleaned.drain(prompt_start..).collect());
                    }
                    i += 2;
                }
                TELNET_NOP | TELNET_SE => {
                    // Skip NOP and stray SE
                    i += 2;
                }
                _ => {
                    // Other 2-byte commands
                    i += 2;
                }
            }
        } else {
            cleaned.push(data[i]);
            i += 1;
        }
    }

    // If WONT ECHO was seen, extract any trailing partial line as prompt
    // (the prompt text comes AFTER the IAC WONT ECHO sequence)
    if wont_echo_seen && prompt.is_none() {
        let last_newline = cleaned.iter().rposition(|&b| b == b'\n');
        let prompt_start = last_newline.map(|p| p + 1).unwrap_or(0);

        // Only extract if there's text after the last newline and it doesn't end with newline
        if prompt_start < cleaned.len() && cleaned.last() != Some(&b'\n') {
            prompt = Some(cleaned.drain(prompt_start..).collect());
        }
    }

    TelnetResult {
        cleaned,
        responses,
        telnet_detected,
        prompt,
        wont_echo_seen,
        naws_requested,
        ttype_requested,
        gmcp_data,
        msdp_data,
        gmcp_negotiated,
        msdp_negotiated,
    }
}

/// Build a TTYPE IS subnegotiation response with the given terminal type
pub fn build_ttype_response(terminal_type: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_TTYPE, TTYPE_IS];
    msg.extend_from_slice(terminal_type.as_bytes());
    msg.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    msg
}

/// Build a NAWS subnegotiation message with the given dimensions
pub fn build_naws_subnegotiation(width: u16, height: u16) -> Vec<u8> {
    let mut result = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_NAWS];
    let data_bytes = [
        (width >> 8) as u8, (width & 0xFF) as u8,
        (height >> 8) as u8, (height & 0xFF) as u8,
    ];
    for &b in &data_bytes {
        result.push(b);
        if b == TELNET_IAC {
            result.push(TELNET_IAC); // Escape 0xFF as 0xFF 0xFF
        }
    }
    result.push(TELNET_IAC);
    result.push(TELNET_SE);
    result
}

/// Parse MSDP VAR/VAL pairs from subnegotiation payload
pub fn parse_msdp_pairs(data: &[u8]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] == MSDP_VAR {
            i += 1;
            // Read variable name until MSDP_VAL
            let name_start = i;
            while i < data.len() && data[i] != MSDP_VAL {
                i += 1;
            }
            let name = String::from_utf8_lossy(&data[name_start..i]).to_string();
            if i < data.len() && data[i] == MSDP_VAL {
                i += 1;
                let value = parse_msdp_value(data, &mut i);
                pairs.push((name, value));
            }
        } else {
            i += 1;
        }
    }
    pairs
}

/// Recursively parse an MSDP value (string, table, or array) into a JSON string
fn parse_msdp_value(data: &[u8], i: &mut usize) -> String {
    if *i >= data.len() {
        return "\"\"".to_string();
    }
    match data[*i] {
        MSDP_TABLE_OPEN => {
            *i += 1;
            let mut entries = Vec::new();
            while *i < data.len() && data[*i] != MSDP_TABLE_CLOSE {
                if data[*i] == MSDP_VAR {
                    *i += 1;
                    let key_start = *i;
                    while *i < data.len() && data[*i] != MSDP_VAL {
                        *i += 1;
                    }
                    let key = String::from_utf8_lossy(&data[key_start..*i]).to_string();
                    if *i < data.len() && data[*i] == MSDP_VAL {
                        *i += 1;
                        let val = parse_msdp_value(data, i);
                        entries.push(format!("\"{}\":{}", escape_json_string(&key), val));
                    }
                } else {
                    *i += 1;
                }
            }
            if *i < data.len() && data[*i] == MSDP_TABLE_CLOSE {
                *i += 1;
            }
            format!("{{{}}}", entries.join(","))
        }
        MSDP_ARRAY_OPEN => {
            *i += 1;
            let mut elements = Vec::new();
            while *i < data.len() && data[*i] != MSDP_ARRAY_CLOSE {
                if data[*i] == MSDP_VAL {
                    *i += 1;
                    let val = parse_msdp_value(data, i);
                    elements.push(val);
                } else {
                    *i += 1;
                }
            }
            if *i < data.len() && data[*i] == MSDP_ARRAY_CLOSE {
                *i += 1;
            }
            format!("[{}]", elements.join(","))
        }
        _ => {
            // Plain string value - read until next MSDP marker or end
            let start = *i;
            while *i < data.len()
                && data[*i] != MSDP_VAR
                && data[*i] != MSDP_VAL
                && data[*i] != MSDP_TABLE_OPEN
                && data[*i] != MSDP_TABLE_CLOSE
                && data[*i] != MSDP_ARRAY_OPEN
                && data[*i] != MSDP_ARRAY_CLOSE
            {
                *i += 1;
            }
            let s = String::from_utf8_lossy(&data[start..*i]).to_string();
            format!("\"{}\"", escape_json_string(&s))
        }
    }
}

/// Escape a string for JSON output
fn escape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

/// Build a GMCP subnegotiation message: IAC SB 201 <package> <json> IAC SE
pub fn build_gmcp_message(package: &str, json: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP];
    msg.extend_from_slice(package.as_bytes());
    if !json.is_empty() {
        msg.push(b' ');
        msg.extend_from_slice(json.as_bytes());
    }
    msg.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    msg
}

/// Build an MSDP request message: IAC SB 69 MSDP_VAR <name> IAC SE
pub fn build_msdp_request(variable: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
    msg.extend_from_slice(variable.as_bytes());
    msg.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    msg
}

/// Build an MSDP set message: IAC SB 69 MSDP_VAR <name> MSDP_VAL <value> IAC SE
pub fn build_msdp_set(variable: &str, value: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
    msg.extend_from_slice(variable.as_bytes());
    msg.push(MSDP_VAL);
    msg.extend_from_slice(value.as_bytes());
    msg.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    msg
}

/// Check if there's an incomplete ANSI escape sequence or telnet sequence at the end.
/// Returns the safe split point - data before this can be processed.
pub fn find_safe_split_point(data: &[u8]) -> usize {
    if data.is_empty() {
        return 0;
    }

    let len = data.len();

    // Check for incomplete ANSI escape sequence at the end
    if let Some(esc_pos) = data.iter().rposition(|&b| b == 0x1B) {
        let after_esc = &data[esc_pos..];

        if after_esc.len() < 2 {
            // Just ESC at the end, incomplete
            return esc_pos;
        }

        if after_esc[1] == b'[' {
            // CSI sequence (ESC [) - look for terminating byte (0x40-0x7E)
            let mut found_terminator = false;
            for &b in &after_esc[2..] {
                if (0x40..=0x7E).contains(&b) {
                    found_terminator = true;
                    break;
                }
                if !((0x30..=0x3F).contains(&b) || b == b';') {
                    found_terminator = true;
                    break;
                }
            }
            if !found_terminator {
                // Incomplete CSI sequence
                return esc_pos;
            }
        }
    }

    // Check for incomplete telnet IAC sequences at the end
    if len >= 1 && data[len - 1] == TELNET_IAC {
        return len - 1;
    }
    if len >= 2 && data[len - 2] == TELNET_IAC {
        let cmd = data[len - 1];
        if matches!(cmd, TELNET_WILL | TELNET_WONT | TELNET_DO | TELNET_DONT) {
            return len - 2;
        }
    }

    // Check for incomplete subnegotiation (IAC SB without matching IAC SE)
    // Scan backwards for the last IAC SB
    if len >= 3 {
        let mut j = len.saturating_sub(2);
        while j > 0 {
            if data[j] == TELNET_IAC && j + 1 < len && data[j + 1] == TELNET_SB {
                // Found IAC SB - check if there's a matching IAC SE after it
                let mut k = j + 2;
                let mut found_se = false;
                while k + 1 < len {
                    if data[k] == TELNET_IAC && data[k + 1] == TELNET_SE {
                        found_se = true;
                        break;
                    } else if data[k] == TELNET_IAC && data[k + 1] == TELNET_IAC {
                        k += 2; // Skip escaped 0xFF
                        continue;
                    }
                    k += 1;
                }
                if !found_se {
                    return j; // Split before incomplete subnegotiation
                }
                break; // Found complete subnegotiation, no need to check further
            }
            j -= 1;
        }
    }

    // All sequences complete, send everything
    len
}

/// AutoConnectType for auto-login behavior
#[derive(Clone, Copy, PartialEq, Default)]
pub enum AutoConnectType {
    #[default]
    Connect,   // Send "connect <user> <password>" after connection
    Prompt,    // Send username on 1st prompt, password on 2nd prompt
    MooPrompt, // Like Prompt but also send username on 3rd prompt
    NoLogin,   // No auto-login even if credentials are set
}

impl AutoConnectType {
    pub fn name(&self) -> &'static str {
        match self {
            AutoConnectType::Connect => "Connect",
            AutoConnectType::Prompt => "Prompt",
            AutoConnectType::MooPrompt => "MOO_prompt",
            AutoConnectType::NoLogin => "None",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            AutoConnectType::Connect => AutoConnectType::Prompt,
            AutoConnectType::Prompt => AutoConnectType::MooPrompt,
            AutoConnectType::MooPrompt => AutoConnectType::NoLogin,
            AutoConnectType::NoLogin => AutoConnectType::Connect,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            AutoConnectType::Connect => AutoConnectType::NoLogin,
            AutoConnectType::Prompt => AutoConnectType::Connect,
            AutoConnectType::MooPrompt => AutoConnectType::Prompt,
            AutoConnectType::NoLogin => AutoConnectType::MooPrompt,
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "prompt" => AutoConnectType::Prompt,
            "moo_prompt" | "mooprompt" => AutoConnectType::MooPrompt,
            "none" | "nologin" | "no_login" => AutoConnectType::NoLogin,
            _ => AutoConnectType::Connect,
        }
    }
}

/// KeepAliveType for connection keepalive behavior
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum KeepAliveType {
    None,    // Disabled - no keepalive sent
    #[default]
    Nop,     // Send telnet NOP character (default)
    Custom,  // Send user-defined command
    Generic, // Send "help commands ##_idler_message_<rand>_###"
}

impl KeepAliveType {
    pub fn name(&self) -> &'static str {
        match self {
            KeepAliveType::None => "None",
            KeepAliveType::Nop => "NOP",
            KeepAliveType::Custom => "Custom",
            KeepAliveType::Generic => "Generic",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            KeepAliveType::None => KeepAliveType::Nop,
            KeepAliveType::Nop => KeepAliveType::Custom,
            KeepAliveType::Custom => KeepAliveType::Generic,
            KeepAliveType::Generic => KeepAliveType::None,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            KeepAliveType::None => KeepAliveType::Generic,
            KeepAliveType::Nop => KeepAliveType::None,
            KeepAliveType::Custom => KeepAliveType::Nop,
            KeepAliveType::Generic => KeepAliveType::Custom,
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "none" => KeepAliveType::None,
            "custom" => KeepAliveType::Custom,
            "generic" => KeepAliveType::Generic,
            _ => KeepAliveType::Nop,
        }
    }
}
