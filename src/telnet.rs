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
pub const TELNET_SE: u8 = 240;   // Subnegotiation End
pub const TELNET_NOP: u8 = 241;  // No Operation (keepalive)

/// Command types for the writer task
pub enum WriteCommand {
    Text(String),     // Regular command (will add \r\n)
    Raw(Vec<u8>),     // Raw bytes (for telnet responses and NOP)
}

/// Stream wrapper enums for supporting both plain TCP and TLS connections
pub enum StreamReader {
    Plain(tokio::net::tcp::OwnedReadHalf),
    Tls(ReadHalf<TlsStream<TcpStream>>),
}

pub enum StreamWriter {
    Plain(tokio::net::tcp::OwnedWriteHalf),
    Tls(WriteHalf<TlsStream<TcpStream>>),
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
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            StreamWriter::Plain(s) => Pin::new(s).poll_flush(cx),
            StreamWriter::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            StreamWriter::Plain(s) => Pin::new(s).poll_shutdown(cx),
            StreamWriter::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Process telnet sequences in incoming data.
/// Returns (cleaned_data, telnet_responses, telnet_detected, prompt).
/// - cleaned_data: data with telnet sequences removed (prompt text excluded if GA found)
/// - telnet_responses: bytes to send back (WONT/DONT responses)
/// - telnet_detected: true if any telnet IAC sequences were found
/// - prompt: text from last newline to GA, if GA was found
pub fn process_telnet(data: &[u8]) -> (Vec<u8>, Vec<u8>, bool, Option<Vec<u8>>) {
    let mut cleaned = Vec::with_capacity(data.len());
    let mut responses = Vec::new();
    let mut telnet_detected = false;
    let mut prompt: Option<Vec<u8>> = None;
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
                    // Respond with DONT for WILL, WONT for DO
                    match cmd {
                        TELNET_WILL => {
                            responses.extend_from_slice(&[TELNET_IAC, TELNET_DONT, option]);
                        }
                        TELNET_DO => {
                            responses.extend_from_slice(&[TELNET_IAC, TELNET_WONT, option]);
                        }
                        _ => {} // WONT/DONT - no response needed
                    }
                    i += 3;
                }
                TELNET_SB => {
                    // Skip subnegotiation until SE
                    i += 2;
                    while i < data.len() {
                        if data[i] == TELNET_IAC && i + 1 < data.len() && data[i + 1] == TELNET_SE {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                TELNET_GA => {
                    // Go Ahead - extract prompt (text from last newline to here)
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

    (cleaned, responses, telnet_detected, prompt)
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
}

impl AutoConnectType {
    pub fn name(&self) -> &'static str {
        match self {
            AutoConnectType::Connect => "Connect",
            AutoConnectType::Prompt => "Prompt",
            AutoConnectType::MooPrompt => "MOO_prompt",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            AutoConnectType::Connect => AutoConnectType::Prompt,
            AutoConnectType::Prompt => AutoConnectType::MooPrompt,
            AutoConnectType::MooPrompt => AutoConnectType::Connect,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            AutoConnectType::Connect => AutoConnectType::MooPrompt,
            AutoConnectType::Prompt => AutoConnectType::Connect,
            AutoConnectType::MooPrompt => AutoConnectType::Prompt,
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "prompt" => AutoConnectType::Prompt,
            "moo_prompt" | "mooprompt" => AutoConnectType::MooPrompt,
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
