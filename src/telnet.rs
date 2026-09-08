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
pub const TELNET_OPT_CHARSET: u8 = 42; // CHARSET (RFC 2066)
pub const TELNET_OPT_MSDP: u8 = 69;   // MUD Server Data Protocol
pub const TELNET_OPT_MSSP: u8 = 70;   // MUD Server Status Protocol (plan Job 12 / 4.2)
pub const TELNET_OPT_MCCP2: u8 = 86;  // MUD Client Compression Protocol v2
pub const TELNET_OPT_MSP: u8 = 90;    // MUD Sound Protocol (plan Job 14)
pub const TELNET_OPT_GMCP: u8 = 201;  // Generic MUD Communication Protocol

// CHARSET subnegotiation opcodes (RFC 2066)
const CHARSET_REQUEST: u8 = 1;
const CHARSET_ACCEPTED: u8 = 2;
const CHARSET_REJECTED: u8 = 3;
const CHARSET_TTABLE_IS: u8 = 4;
const CHARSET_TTABLE_REJECTED: u8 = 7;

// MSDP sub-negotiation markers
pub const MSDP_VAR: u8 = 1;
pub const MSDP_VAL: u8 = 2;
pub const MSDP_TABLE_OPEN: u8 = 3;
pub const MSDP_TABLE_CLOSE: u8 = 4;
pub const MSDP_ARRAY_OPEN: u8 = 5;
pub const MSDP_ARRAY_CLOSE: u8 = 6;

// MSSP sub-negotiation markers (plan Job 12 / 4.2). Numerically identical to
// MSDP_VAR/MSDP_VAL - MSSP and MSDP independently chose 1/2 for "name"/"value"
// - but option 70 (MSSP) and option 69 (MSDP) are unrelated wire protocols
// (MSSP has no TABLE/ARRAY nesting, and a name may repeat with more than one
// value in a row), so MSSP gets its own named constants rather than reusing
// MSDP's.
pub const MSSP_VAR: u8 = 1;
pub const MSSP_VAL: u8 = 2;

// TTYPE subnegotiation commands
pub const TTYPE_IS: u8 = 0;    // Terminal type IS (response)
pub const TTYPE_SEND: u8 = 1;  // Send terminal type (request)

// MTTS (option 24 TTYPE's "MTTS <bitmask>" convention, plan Job 12 / 4.1):
// bit values a client may report in its third-and-later TTYPE IS answer,
// once the plain name/terminal-type answers are exhausted (see
// `TelnetSession::mtts_bitmask`'s doc comment for which bits Clay actually
// sets and why). Kept in full, including bits Clay never sets, as the
// reference table - see mudstandards.org's MTTS page / the plan's "Protocol
// landscape, rated" section for the source of these values.
pub const MTTS_ANSI: u16 = 1;
pub const MTTS_VT100: u16 = 2;
pub const MTTS_UTF8: u16 = 4;
pub const MTTS_256_COLOR: u16 = 8;
pub const MTTS_MOUSE_TRACKING: u16 = 16;
pub const MTTS_OSC_COLOR_PALETTE: u16 = 32;
pub const MTTS_SCREEN_READER: u16 = 64;
pub const MTTS_PROXY: u16 = 128;
pub const MTTS_TRUECOLOR: u16 = 256;
pub const MTTS_MNES: u16 = 512;
pub const MTTS_MSLP: u16 = 1024;
pub const MTTS_SSL: u16 = 2048;

// MSP (plan Job 14) in-band trigger markers. MSP is not a subnegotiation
// protocol at all despite having a telnet option (90) — the actual triggers
// are plain ASCII text embedded in the ordinary output stream (`!!SOUND(...)`
// / `!!MUSIC(...)`), which is exactly why so many servers emit them without
// ever negotiating the option: there is nothing to negotiate for the trigger
// itself to work. See `extract_msp_triggers`.
const MSP_SOUND_MARKER: &[u8] = b"!!SOUND(";
const MSP_MUSIC_MARKER: &[u8] = b"!!MUSIC(";
// Both markers are the same length by construction (asserted in tests) so
// `extract_msp_triggers` only needs one constant for "where params start".
const MSP_MARKER_LEN: usize = MSP_SOUND_MARKER.len();
/// How far past a `!!SOUND(`/`!!MUSIC(` marker `extract_msp_triggers` will
/// hold back an unclosed trigger across `feed()` calls before giving up and
/// flushing it as ordinary text (see that function's doc comment). Real
/// triggers routinely carry a filename plus `V=`/`L=`/`P=`/`T=`/`U=` — a full
/// URL in `U=` alone can be long — so this is deliberately much larger than
/// `MAX_TEXT_HOLDBACK`'s 32 bytes, which exists for a short ANSI CSI/UTF-8
/// remnant, not a whole parameter list.
const MAX_MSP_TRIGGER_HOLDBACK: usize = 1024;

/// Command types for the writer task
#[derive(Debug)]
pub enum WriteCommand {
    Text(String),     // Regular command (will add \r\n)
    Raw(Vec<u8>),     // Raw bytes (for telnet responses and NOP)
    Shutdown,         // Close the connection gracefully
    /// Switch the writer's active `Encoding` for every subsequent `Text`
    /// command (plan Job 13, Phase 4 / 4.4, finding 7). Sent in-band through
    /// the same `mpsc` channel as `Text`/`Raw` rather than shared mutable
    /// state on `World`, specifically so ordering is right *by construction*:
    /// anything already queued ahead of this command (typed before a CHARSET
    /// accept completes) is written in the old encoding, and everything
    /// after switches — no lock, no race between "decide to switch" and
    /// "the writer notices." See `spawn_telnet_writer`'s doc comment for the
    /// alternative this was weighed against.
    SetEncoding(crate::encoding::Encoding),
}

/// Stream wrapper enums for supporting both plain TCP and TLS connections
pub enum StreamReader {
    Plain(tokio::net::tcp::OwnedReadHalf),
    Tls(ReadHalf<TlsStream<TcpStream>>),
    #[cfg(unix)]
    Proxy(tokio::net::unix::OwnedReadHalf),  // Unix socket for TLS proxy
    #[cfg(windows)]
    NamedPipeProxy(tokio::io::ReadHalf<tokio::net::windows::named_pipe::NamedPipeClient>),
    /// In-process byte pipe for `spawn_telnet_reader` unit tests
    /// (`src/telnet_reader.rs`, plan Job 4) — lets a test drive the exact
    /// `StreamReader` type production code uses without a real socket. Not
    /// reachable from production code (no non-test constructor ever builds
    /// one).
    #[cfg(test)]
    Duplex(ReadHalf<tokio::io::DuplexStream>),
    /// A scripted sequence of socket reads, each either a chunk of bytes or an `io::Error`
    /// (plan Job 1, T1.15) — used to test `spawn_telnet_reader`'s `Err(e)` arm, which
    /// `Duplex` above can't reach: dropping the duplex's peer only ever produces a clean
    /// `Ok(0)` EOF, never a real read error. Each `poll_read` call pops the front of the
    /// queue; an `Ok` chunk longer than the caller's buffer is split, with the remainder
    /// pushed back for the next call. An empty queue reads as `Ok(())` with nothing written
    /// to `buf` — a clean EOF, so a script that doesn't end in an explicit `Err` still
    /// terminates the reader task instead of hanging it forever.
    #[cfg(test)]
    Scripted(std::collections::VecDeque<io::Result<Vec<u8>>>),
}

pub enum StreamWriter {
    Plain(tokio::net::tcp::OwnedWriteHalf),
    Tls(WriteHalf<TlsStream<TcpStream>>),
    #[cfg(unix)]
    Proxy(tokio::net::unix::OwnedWriteHalf),  // Unix socket for TLS proxy
    #[cfg(windows)]
    NamedPipeProxy(tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>),
    /// In-process byte pipe for `spawn_telnet_writer` unit tests
    /// (`src/telnet_writer.rs`, plan Job 13) — mirrors `StreamReader::Duplex`
    /// above. Not reachable from production code.
    #[cfg(test)]
    Duplex(WriteHalf<tokio::io::DuplexStream>),
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
            #[cfg(windows)]
            StreamReader::NamedPipeProxy(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(test)]
            StreamReader::Duplex(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(test)]
            StreamReader::Scripted(queue) => match queue.pop_front() {
                Some(Ok(mut chunk)) => {
                    let n = chunk.len().min(buf.remaining());
                    buf.put_slice(&chunk[..n]);
                    if n < chunk.len() {
                        // Caller's buffer was smaller than this scripted chunk: push the
                        // remainder back for the next poll_read, same as a real socket
                        // would just deliver it across two reads.
                        queue.push_front(Ok(chunk.split_off(n)));
                    }
                    Poll::Ready(Ok(()))
                }
                Some(Err(e)) => Poll::Ready(Err(e)),
                None => Poll::Ready(Ok(())), // queue exhausted: reads as EOF (0 bytes)
            },
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
            #[cfg(windows)]
            StreamWriter::NamedPipeProxy(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(test)]
            StreamWriter::Duplex(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            StreamWriter::Plain(s) => Pin::new(s).poll_flush(cx),
            StreamWriter::Tls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(unix)]
            StreamWriter::Proxy(s) => Pin::new(s).poll_flush(cx),
            #[cfg(windows)]
            StreamWriter::NamedPipeProxy(s) => Pin::new(s).poll_flush(cx),
            #[cfg(test)]
            StreamWriter::Duplex(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            StreamWriter::Plain(s) => Pin::new(s).poll_shutdown(cx),
            StreamWriter::Tls(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(unix)]
            StreamWriter::Proxy(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(windows)]
            StreamWriter::NamedPipeProxy(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(test)]
            StreamWriter::Duplex(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Result of processing telnet sequences.
///
/// Private by design (Phase 2, Step 2.6): `process_telnet` is now used only as
/// the oracle for the characterization and differential tests in `mod tests`
/// below, which can still see it because a private item is visible to its
/// defining module's descendants. No production code may call it — that is
/// what makes a fourteenth divergent reader loop impossible to write.
///
/// `#[cfg(test)]`: this type exists purely for `process_telnet`'s test-oracle
/// role, so it is compiled only for test builds — a plain non-test `cargo
/// build` would otherwise flag it (and `process_telnet`/`find_safe_split_point`
/// below) as dead code.
#[cfg(test)]
struct TelnetResult {
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
    pub charset_request: Option<Vec<String>>,  // Charsets offered by server via CHARSET REQUEST
    pub mccp2_activated: bool,  // True if IAC SB MCCP2 IAC SE was received (compression starts)
    pub mccp2_offset: usize,    // Byte offset in input data where compressed stream begins
}

/// Process telnet sequences in incoming data.
/// Returns TelnetResult with cleaned data and negotiation info.
///
/// Private (Phase 2, Step 2.6) — kept only as the test oracle. `TelnetSession`
/// (below) is the sole production path now.
#[cfg(test)]
fn process_telnet(data: &[u8]) -> TelnetResult {
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
    let mut charset_request: Option<Vec<String>> = None;
    let mut mccp2_activated = false;
    let mut mccp2_offset: usize = 0;
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
                            } else if option == TELNET_OPT_MCCP2 {
                                // Accept MCCP2 compression
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_DO, option]);
                            } else if option == TELNET_OPT_CHARSET {
                                // Accept CHARSET negotiation (RFC 2066)
                                responses.extend_from_slice(&[TELNET_IAC, TELNET_DO, option]);
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
                                // Check for MCCP2 activation (option 86)
                                // IAC SB MCCP2 IAC SE - all data after SE is zlib compressed
                                if !sb_data.is_empty() && sb_data[0] == TELNET_OPT_MCCP2 {
                                    mccp2_activated = true;
                                    mccp2_offset = i + 2; // offset after IAC SE
                                    // Stop processing - remaining bytes are compressed
                                    i += 2;
                                    break;
                                }
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
                                // Check for CHARSET subnegotiation (option 42, RFC 2066)
                                if sb_data.len() >= 3 && sb_data[0] == TELNET_OPT_CHARSET {
                                    if sb_data[1] == CHARSET_REQUEST {
                                        charset_request = Some(parse_charset_request(&sb_data[1..]));
                                    } else if sb_data[1] == CHARSET_TTABLE_IS {
                                        // We don't support TTABLEs — reject
                                        responses.extend_from_slice(&[
                                            TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_TTABLE_REJECTED,
                                            TELNET_IAC, TELNET_SE,
                                        ]);
                                    }
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
                    // MCCP2: stop outer loop - remaining bytes are compressed
                    if mccp2_activated {
                        break;
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
        charset_request,
        mccp2_activated,
        mccp2_offset,
    }
}

/// Append `bytes` to `out`, doubling every `TELNET_IAC` (0xFF) byte per the telnet
/// byte-stuffing rule (RFC 854). Use this ONLY for payload data — framing bytes
/// (the `IAC SB <opt>` prefix, the `IAC SE` terminator, opcode/marker bytes such as
/// `MSDP_VAR`/`MSDP_VAL`, and fixed separators such as GMCP's space) are structure,
/// not payload, and must be pushed directly rather than through this function, or
/// the far end can no longer find the subnegotiation terminator.
///
/// `pub(crate)` (plan Job 13, Phase 4 / 4.4): `telnet_writer.rs`'s
/// `spawn_telnet_writer` is the other caller now — a `WriteCommand::Text`
/// payload, once actually encoded via `Encoding::encode` instead of assumed
/// UTF-8, can produce a genuine `0xFF` byte (a Latin1/Fansi high-byte
/// character where a UTF-8 `&str` never could), which must be escaped here
/// exactly like every other outbound payload.
pub(crate) fn push_escaped(out: &mut Vec<u8>, bytes: &[u8]) {
    for &b in bytes {
        out.push(b);
        if b == TELNET_IAC {
            out.push(TELNET_IAC); // Escape 0xFF as 0xFF 0xFF
        }
    }
}

/// Undo telnet byte-stuffing (RFC 854) in a subnegotiation *payload*: every
/// `IAC IAC` pair represents one literal `0xFF` byte in the data as the far
/// end meant it. `push_escaped` above is the outbound half of this rule;
/// this is the inbound half — finding 6 / Phase 3.1's "real half", since the
/// terminator scan in `feed()`'s `TELNET_SB` handling already skips an
/// escaped `IAC IAC` correctly when hunting for the closing `IAC SE` (so a
/// doubled 0xFF is never mistaken for the terminator), but until now the
/// *payload itself* — handed to `parse_msdp_pairs`, the GMCP package/JSON
/// split, and `parse_charset_request` — still contained both bytes of every
/// escaped pair verbatim, so an MSDP value/GMCP JSON string/CHARSET name
/// containing a literal 0xFF arrived at its parser doubled instead of
/// singled.
///
/// Used only on subnegotiation payload bytes handed to those three parsers.
/// Never on the raw buffer `feed()` scans for framing: MCCP2's activation
/// offset (`after_se`, used to split off the raw compressed tail) is a
/// position in *that* buffer, and must stay in raw index space — computed
/// before, and independently of, any unescaped copy a sibling branch of
/// `handle_subnegotiation` builds for GMCP/MSDP/CHARSET.
fn unescape_iac(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        out.push(data[i]);
        if data[i] == TELNET_IAC && i + 1 < data.len() && data[i + 1] == TELNET_IAC {
            i += 2; // doubled pair -> one literal 0xFF byte, already pushed once
        } else {
            i += 1;
        }
    }
    out
}

/// Build a TTYPE IS subnegotiation response with the given terminal type
pub fn build_ttype_response(terminal_type: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_TTYPE, TTYPE_IS];
    push_escaped(&mut msg, terminal_type.as_bytes());
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
    push_escaped(&mut result, &data_bytes);
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

/// Parse an MSSP (option 70, plan Job 12 / 4.2) subnegotiation payload into
/// ordered name/value pairs: `(MSSP_VAR name MSSP_VAL value)*`. Unlike MSDP,
/// MSSP has no TABLE/ARRAY nesting, but a name may legitimately repeat with
/// more than one `MSSP_VAL` in a row (e.g. `MSSP_VAR "GENRE" MSSP_VAL
/// "Adventure" MSSP_VAL "Fantasy"`) - each `MSSP_VAL` under the same
/// still-open name becomes its own `(name, value)` entry, never collapsed or
/// concatenated, which is why this returns a `Vec` rather than a map.
///
/// Malformed or truncated input never panics and never produces a
/// half-populated pair: a pair is only pushed once a complete `MSSP_VAL` has
/// been scanned for a name that's actually in scope, so a `MSSP_VAR` with no
/// following `MSSP_VAL` at all (including one cut off at the end of the
/// payload) simply contributes nothing, and a stray `MSSP_VAL` before any
/// `MSSP_VAR` is dropped rather than paired with an empty/garbage name.
pub fn parse_mssp_pairs(data: &[u8]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut i = 0;
    let mut current_name: Option<String> = None;
    while i < data.len() {
        match data[i] {
            MSSP_VAR => {
                i += 1;
                let start = i;
                while i < data.len() && data[i] != MSSP_VAL && data[i] != MSSP_VAR {
                    i += 1;
                }
                current_name = Some(String::from_utf8_lossy(&data[start..i]).to_string());
            }
            MSSP_VAL => {
                i += 1;
                let start = i;
                while i < data.len() && data[i] != MSSP_VAL && data[i] != MSSP_VAR {
                    i += 1;
                }
                if let Some(ref name) = current_name {
                    let value = String::from_utf8_lossy(&data[start..i]).to_string();
                    pairs.push((name.clone(), value));
                }
                // A MSSP_VAL with no name in scope (malformed) is dropped
                // silently rather than paired with an invented name.
            }
            _ => {
                i += 1; // stray byte before the first MSSP_VAR; skip
            }
        }
    }
    pairs
}

/// Build a GMCP subnegotiation message: IAC SB 201 <package> <json> IAC SE
pub fn build_gmcp_message(package: &str, json: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP];
    push_escaped(&mut msg, package.as_bytes());
    if !json.is_empty() {
        msg.push(b' '); // fixed separator - structure, not payload
        push_escaped(&mut msg, json.as_bytes());
    }
    msg.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    msg
}

/// Build an MSDP request message: IAC SB 69 MSDP_VAR <name> IAC SE
pub fn build_msdp_request(variable: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
    push_escaped(&mut msg, variable.as_bytes());
    msg.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    msg
}

/// Build an MSDP set message: IAC SB 69 MSDP_VAR <name> MSDP_VAL <value> IAC SE
pub fn build_msdp_set(variable: &str, value: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
    push_escaped(&mut msg, variable.as_bytes());
    msg.push(MSDP_VAL); // marker byte - structure, not payload
    push_escaped(&mut msg, value.as_bytes());
    msg.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    msg
}

/// Parse a CHARSET REQUEST payload into a list of charset names.
/// Format: REQUEST <sep> [TTABLE <version>] <charset1> <sep> <charset2> ...
fn parse_charset_request(data: &[u8]) -> Vec<String> {
    // data[0] is REQUEST opcode (already checked by caller)
    if data.len() < 3 {
        return Vec::new();
    }
    let sep = data[1];
    let mut start = 2;

    // Check for optional TTABLE flag (byte value 1) followed by version byte
    if start < data.len() && data[start] == 1 {
        // Skip TTABLE flag and version byte
        start += 2;
    }

    // Split remaining bytes by separator into charset names
    let mut charsets = Vec::new();
    let mut name_start = start;
    for i in start..data.len() {
        if data[i] == sep {
            if i > name_start {
                if let Ok(name) = std::str::from_utf8(&data[name_start..i]) {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() {
                        charsets.push(trimmed.to_string());
                    }
                }
            }
            name_start = i + 1;
        }
    }
    // Last charset (after final separator or no trailing separator)
    if name_start < data.len() {
        if let Ok(name) = std::str::from_utf8(&data[name_start..]) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                charsets.push(trimmed.to_string());
            }
        }
    }
    charsets
}

/// Build a CHARSET ACCEPTED subnegotiation response
pub fn build_charset_accepted(charset_name: &str) -> Vec<u8> {
    let mut msg = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_ACCEPTED];
    push_escaped(&mut msg, charset_name.as_bytes());
    msg.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
    msg
}

/// Build a CHARSET REJECTED subnegotiation response
pub fn build_charset_rejected() -> Vec<u8> {
    vec![TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_REJECTED, TELNET_IAC, TELNET_SE]
}

/// A single MCCP2 decompression pass (one `feed()` call) is capped at this
/// many produced bytes — the decompression-bomb guard (plan Phase 1.3).
/// Real MUD output within one TCP read is nowhere near this; a hostile zlib
/// stream can expand a few kilobytes of input by three or more orders of
/// magnitude, and once `TelnetSession` owns the decompressor itself (rather
/// than leaving it to a caller-owned buffer, as every production reader
/// loop does today) it must not let that turn into unbounded memory growth.
const MCCP2_BOMB_LIMIT: usize = 4 * 1024 * 1024; // 4 MiB per feed() call

/// Defence in depth for `run_decompressor`'s `pending_compressed` carry (T1.1, plan Job 1,
/// step 3). Normal streaming decompression can leave a genuine few-byte carry when a zlib
/// block boundary doesn't land on a `feed()` call boundary — that's expected and small. If
/// the unconsumed remainder ever exceeds this, something has gone wrong beyond ordinary
/// streaming (the decompressor is stuck making no real progress), and it is treated the same
/// as an outright inflate error: poison the session and disconnect (design decision D1)
/// rather than let `pending_compressed` become the unbounded retention buffer T1.1 was about.
const MAX_PENDING_COMPRESSED: usize = 65_536;

/// Outcome of one `mccp2_decompress_inner` call (plan Job 1, T1.1/step 2). Replaces the old
/// four-element tuple specifically so `run_decompressor` can tell "zlib itself errored" apart
/// from "needs more input" — the two used to collapse onto the same `consumed`-short-of-
/// `compressed.len()` shape, which is the root cause of T1.1's leftover-retention bug: every
/// later byte piled into `pending_compressed` forever because nothing distinguished the two.
struct InflateStep {
    /// Decompressed bytes produced this call.
    output: Vec<u8>,
    /// How many bytes of `compressed` were actually fed to zlib — the remainder, if any, is
    /// unconsumed input the caller must carry to the next call rather than discard (unless
    /// `error` is `Some`, in which case nothing should ever be retried against this
    /// decompressor again).
    consumed: usize,
    /// `Status::StreamEnd` (`Z_STREAM_END`) — the far end turned compression off.
    stream_end: bool,
    /// `output.len() >= max_output` (checked at the top of each loop iteration, so the actual
    /// length can exceed `max_output` by up to one iteration's output — a few kilobytes —
    /// which is fine for a "few MiB is ample" guard).
    bomb_hit: bool,
    /// `Some(reason)` iff zlib itself reported a decompression error — a poisoned stream
    /// (T1.1). `reason` comes from `flate2::DecompressError`'s `Display`, which never
    /// includes byte counts or positions (see design decision D1's requirement that a
    /// `CompressionFailed` message stay split-invariant), so it is safe to surface directly.
    error: Option<String>,
}

/// Shared decompression loop behind `mccp2_decompress`/`mccp2_decompress_ex`
/// and `TelnetSession`'s inline decompression (`mccp2_decompress_capped`).
/// Feeds `compressed` through `decompressor` until it stops making
/// progress, the zlib stream ends, `max_output` decompressed bytes have been
/// produced, or zlib reports an error.
fn mccp2_decompress_inner(
    decompressor: &mut flate2::Decompress,
    compressed: &[u8],
    max_output: usize,
) -> InflateStep {
    let mut output = Vec::with_capacity(compressed.len().saturating_mul(4).min(max_output.max(1)));
    let mut buf = [0u8; 8192];
    let mut total_in = 0;
    let mut stream_end = false;
    let mut bomb_hit = false;
    let mut error = None;

    loop {
        if output.len() >= max_output {
            bomb_hit = true;
            break;
        }
        let before_in = decompressor.total_in();
        let before_out = decompressor.total_out();
        let status = decompressor.decompress(
            &compressed[total_in..],
            &mut buf,
            flate2::FlushDecompress::None,
        );
        let consumed = (decompressor.total_in() - before_in) as usize;
        let produced = (decompressor.total_out() - before_out) as usize;
        total_in += consumed;

        if produced > 0 {
            output.extend_from_slice(&buf[..produced]);
        }

        match status {
            Ok(flate2::Status::Ok) => {
                if consumed == 0 && produced == 0 {
                    break; // No progress, need more input
                }
            }
            Ok(flate2::Status::StreamEnd) => {
                // Server ended compression (e.g., MCCP2 turned off)
                stream_end = true;
                break;
            }
            Ok(flate2::Status::BufError) => {
                break; // Need more input or output space exhausted
            }
            Err(e) => {
                // T1.1: the old code discarded `e` and just broke here, leaving
                // `run_decompressor` unable to tell this apart from "needs more input" -
                // that ambiguity is the whole bug. Once `Decompress` is in this state every
                // later call re-fails the same way, so the caller must poison, not retry.
                error = Some(e.to_string());
                break;
            }
        }
    }
    InflateStep { output, consumed: total_in, stream_end, bomb_hit, error }
}

/// Decompress MCCP2 zlib data, returning decompressed bytes, how many bytes
/// of `compressed` were actually consumed, and whether the zlib stream
/// ended (`Status::StreamEnd` / `Z_STREAM_END`). The consumed count and
/// stream-end flag are exactly what the old `mccp2_decompress` discarded,
/// which is why `Z_STREAM_END` recovery was impossible for any of its
/// callers. Uncapped (see `mccp2_decompress_capped` for the bomb-guarded
/// variant `TelnetSession` uses). The decompressor maintains state across
/// calls for streaming decompression. Like `mccp2_decompress` below, this
/// legacy wrapper still discards the error/bomb information `InflateStep`
/// now carries — untouched by plan Job 1, which only hardens
/// `TelnetSession`'s own `run_decompressor` path.
pub fn mccp2_decompress_ex(
    decompressor: &mut flate2::Decompress,
    compressed: &[u8],
) -> (Vec<u8>, usize, bool) {
    let step = mccp2_decompress_inner(decompressor, compressed, usize::MAX);
    (step.output, step.consumed, step.stream_end)
}

/// Decompress MCCP2 zlib data. Returns decompressed bytes.
/// The decompressor maintains state across calls for streaming decompression.
///
/// Thin backward-compatible wrapper over `mccp2_decompress_ex` for the
/// thirteen reader-loop call sites (main.rs/daemon.rs/commands.rs) that
/// predate `TelnetSession` and are migrated onto it in a later job (plan
/// Phase 2). They don't consume the consumed-count/stream-end information,
/// so this keeps their behaviour byte-for-byte identical to before Job 3.
pub fn mccp2_decompress(decompressor: &mut flate2::Decompress, compressed: &[u8]) -> Vec<u8> {
    mccp2_decompress_ex(decompressor, compressed).0
}

/// `mccp2_decompress_inner` with the Phase 1.3 decompression-bomb cap
/// applied. Used exclusively by `TelnetSession::run_decompressor`, which —
/// unlike the legacy callers above — owns its decompressor for the whole
/// life of the connection and therefore must bound how much a single
/// `feed()` call can produce.
fn mccp2_decompress_capped(
    decompressor: &mut flate2::Decompress,
    compressed: &[u8],
) -> InflateStep {
    mccp2_decompress_inner(decompressor, compressed, MCCP2_BOMB_LIMIT)
}

/// Whether `data` ends in an ANSI escape sequence that hasn't reached a
/// terminating byte yet — a lone trailing ESC, or an `ESC [` (CSI) with no
/// final byte (0x40-0x7E) after it. Returns the offset of the ESC that opens
/// it. `None` means whatever escape sequence(s) `data` contains are all
/// complete (or there is no ESC at all).
///
/// Lifted out of `find_safe_split_point` (D2 in the plan): that function's
/// IAC/subnegotiation branches are wrong for `TelnetSession::pending_text` —
/// by the time text lands there, an `IAC IAC` escape has already become a
/// literal `0xFF` byte that must *not* be held back, and there is no
/// subnegotiation payload in plain text at all — but this ESC/CSI branch is
/// exactly as correct there as it always was, so `idle_release_len` uses it
/// directly and `find_safe_split_point` now calls it too rather than
/// duplicating the logic, keeping the characterization tests below exercising
/// the real, shared implementation.
fn unterminated_escape_start(data: &[u8]) -> Option<usize> {
    let esc_pos = data.iter().rposition(|&b| b == 0x1B)?;
    let after_esc = &data[esc_pos..];

    if after_esc.len() < 2 {
        // Just ESC at the end, incomplete.
        return Some(esc_pos);
    }

    if after_esc[1] == b'[' {
        // CSI sequence (ESC [) - look for a terminating byte (0x40-0x7E).
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
            // Incomplete CSI sequence.
            return Some(esc_pos);
        }
    }

    None
}

/// Check if there's an incomplete ANSI escape sequence or telnet sequence at the end.
/// Returns the safe split point - data before this can be processed.
///
/// Private (Phase 2, Step 2.6) — per design commitment 4, this role is gone
/// in production: `TelnetSession` owns real carry-over instead. Kept only as
/// the test oracle for the split-point characterization tests below.
#[cfg(test)]
fn find_safe_split_point(data: &[u8]) -> usize {
    if data.is_empty() {
        return 0;
    }

    let len = data.len();

    // Check for incomplete ANSI escape sequence at the end (see
    // `unterminated_escape_start`'s doc comment for why this is now shared
    // rather than duplicated here).
    if let Some(esc_pos) = unterminated_escape_start(data) {
        return esc_pos;
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

// ============================================================================
// TelnetSession — Phase 1 (Jobs 2 and 3 of the telnet-protocol-work plan).
//
// A pure, synchronous, stateful telnet parser with *real* carry-over across
// `feed` calls (Design commitment 1 in
// investigate-differences-between-tinyfugu-fluffy-stallman.md), including
// inline MCCP2 decompression (Job 3 — see the `decomp` field doc comment).
// Nothing in the production code calls this yet —
// `process_telnet`/`find_safe_split_point` above remain the live path (and
// are now also this module's oracle) until Phase 2 migrates every reader
// loop onto `TelnetSession`. The RFC 1143 option state machine (Job 9) is
// deliberately not wired in here.
// ============================================================================

/// A subnegotiation with no terminating `IAC SE` is abandoned once its
/// accumulated bytes (from the `IAC` that opened `IAC SB` through the current
/// end of the buffer) exceed this. Without a cap, removing `find_safe_split_point`'s
/// "send it anyway" fallback (finding 3) would let a server that never sends
/// `IAC SE` grow `inbuf` without bound and stall the world forever.
const MAX_SUBNEG_BYTES: usize = 8192;

/// How many trailing "not yet known to be complete" text bytes `feed` will
/// hold back across calls before giving up and flushing anyway. See the
/// "Text hold-back" section of `TelnetSession`'s doc comment for what this
/// buys beyond the ANSI-CSI/UTF-8 case the plan names it for.
const MAX_TEXT_HOLDBACK: usize = 32;

/// Configuration for a `TelnetSession`. Only what Phase 1 framing needs is
/// wired up today; the struct exists now (per the plan) so later phases —
/// offer flags, a GMCP package allow-list, MTTS bits, initiate-negotiation
/// toggles — can grow it without touching every call site again.
#[derive(Clone, Debug)]
pub struct TelnetConfig {
    /// Sent in the TTYPE IS response (`build_ttype_response`) - the second
    /// answer of the MTTS cycle (plan Job 12 / 4.1, see
    /// `TelnetSession::mtts_answer`), uppercased.
    pub term_type: String,
    /// The first answer of the MTTS cycle (plan Job 12 / 4.1): the client's
    /// own name, uppercased, e.g. "CLAY".
    pub client_name: String,
    /// Job 12 (plan Phase 4, 4.1): whether the underlying connection this
    /// session is parsing is actually TLS-encrypted — feeds the MTTS `SSL`
    /// bit (see `TelnetSession::mtts_bitmask`). `TelnetSession` has no socket
    /// of its own to inspect (design commitment 1: pure, synchronous, no
    /// I/O), so this has to come in from the call site, which already knows
    /// (it chose `StreamReader::Tls`/`Proxy`/`NamedPipeProxy` vs `Plain`, or
    /// tracks it on `World::is_tls`).
    pub is_tls: bool,
    /// Job 14 (plan Phase 4): per-world escape hatch for MSP (`!!SOUND(...)`/
    /// `!!MUSIC(...)`) trigger recognition — mirrors
    /// `World::settings.msp_enabled` (default on): audio triggered by a
    /// remote server is something a user must be able to turn off. This
    /// gates *inbound* behaviour, not an outbound offer: when `false`,
    /// `extract_msp_triggers` is not even called (see `feed`), so a
    /// `!!SOUND(...)` trigger passes through as perfectly ordinary text —
    /// off means MSP does not exist as far as this session is concerned, not
    /// merely "recognised but muted".
    pub msp_enabled: bool,
}

impl Default for TelnetConfig {
    fn default() -> Self {
        TelnetConfig {
            // Matches main.rs's existing `$TERM`-or-"ANSI" fallback so a
            // freshly-migrated call site sees the same default it does today.
            term_type: "ANSI".to_string(),
            client_name: "CLAY".to_string(),
            is_tls: false,
            msp_enabled: true,
        }
    }
}

/// Events a `TelnetSession` can emit from `feed`/`flush_eof`.
///
/// Deliberately covers more than any one job needs at the time it's added —
/// every variant here (including `CompressionEnded`, wired up by Job 3, and
/// `OptionEnabled`/`OptionDisabled`/`WontEchoPromptHint`, wired up by Job 4.5)
/// is reachable today, so that Design commitment 2's exhaustive `match` in
/// the future reader's event mapper does not need a new variant added at the
/// same moment as its first real producer.
#[derive(Debug, Clone, PartialEq)]
pub enum TelnetEvent {
    /// At least one IAC byte has been seen on this connection. Emitted once
    /// — the first time it happens — not once per `feed` call that happens
    /// to contain one (see `TelnetSession::telnet_seen`).
    TelnetDetected,
    /// Prompt text carved out of the emitted text stream at a GA/EOR or a
    /// WONT-ECHO boundary. Mirrors `TelnetResult::prompt`.
    Prompt(Vec<u8>),
    /// Server sent `DO NAWS`; we already answered `WILL NAWS` (in `wire`).
    NawsRequested,
    /// Server sent `SB TTYPE SEND`. Before Job 12 (plan Phase 4, 4.1) this
    /// had no immediate `wire` reply (matching `process_telnet`) and
    /// `App::handle_telnet_event` answered it separately with a fixed
    /// `$TERM` value every time. As of Job 12 the session answers directly,
    /// in `wire`, with the MTTS-cycled value (`TelnetSession::mtts_answer` —
    /// client name, then terminal type, then `MTTS <bitmask>` forever) the
    /// instant this event is produced: only the session can see
    /// `ttype_index`, so only the session can decide which cycle position
    /// this request is. This event now exists purely so a consumer can
    /// observe "TTYPE was asked for" (nothing does today; `App` has nothing
    /// left to do here, see its `TtypeRequested` arm) — the actual reply
    /// bytes are already in the same `TelnetOutcome.wire` this event travels
    /// with.
    TtypeRequested,
    /// Server sent `SB CHARSET REQUEST ...`; the offered charset names.
    CharsetRequest(Vec<String>),
    /// `SB GMCP <package> [<json>]`: (package.message, json - possibly empty).
    GmcpMessage(String, String),
    /// One MSDP VAR/VAL pair (variable name, JSON-encoded value).
    MsdpVariable(String, String),
    /// `SB MSSP (MSSP_VAR name MSSP_VAL value)* SE` (plan Job 12 / 4.2): the
    /// server's status listing as ordered name/value pairs (see
    /// `parse_mssp_pairs` — a name may repeat with more than one value, so
    /// this is never collapsed into a map). Each occurrence is a complete,
    /// self-contained snapshot per the MSSP convention (not an incremental
    /// update), so `App::handle_telnet_event` replaces `World::mssp_data`
    /// with it rather than merging.
    MsspData(Vec<(String, String)>),
    /// `IAC SB MCCP2 IAC SE` was seen. By the time this event is observed
    /// the decompressor has already been created, and any compressed bytes
    /// that arrived in the same `feed` call have already been decompressed
    /// in place and folded into parsing (Job 3) — there is no separate step
    /// or second buffer the caller needs to drive.
    CompressionStarted,
    /// The MCCP2 zlib stream ended (`Status::StreamEnd`, i.e. `Z_STREAM_END`
    /// — the far end turned compression off). The decompressor has already
    /// been dropped; any bytes after the stream's end are ordinary telnet
    /// again and have already been parsed as such in this same call
    /// wherever possible. Production code today has no equivalent of this:
    /// the decompressor is kept forever and every later byte silently
    /// vanishes.
    CompressionEnded,
    /// The MCCP2 zlib stream is unrecoverably broken: either `decompressor.decompress`
    /// itself reported an error (a corrupt/hostile stream, T1.1), or the decompression-bomb
    /// guard tripped (`MCCP2_BOMB_LIMIT`, T1.14). Design decision D1: a poisoned zlib stream
    /// has no resync point — roughly 1 in 256 of its bytes is `0xFF`, so parsing them as
    /// telnet would flip option state and send wire replies to the server from what is
    /// really still-compressed noise — so this is fatal, never a fall-back-to-plaintext
    /// case like `ProtocolError` below. The session sets its `poisoned` flag before pushing
    /// this event; every `feed()` call after this one returns `TelnetOutcome::default()`
    /// structurally, and the reader (`telnet_reader::spawn_telnet_reader`) treats this event
    /// as fatal: forward it, then `flush_eof()`, then a disconnect message, then
    /// `AppEvent::Disconnected`, the same sequence a clean `Ok(0)` EOF uses. The `String` is
    /// a human-readable reason with no byte counts or positions (unlike `ProtocolError`
    /// below, which may) — one is embedded in the disconnect message the reader builds, and
    /// a value that varied with how the corrupt stream happened to be chunked across reads
    /// would break the split-invariance property every other event here already has.
    CompressionFailed(String),
    /// A subnegotiation was abandoned (see `MAX_SUBNEG_BYTES`), an MCCP2 activation
    /// subnegotiation was ignored because it wasn't negotiated or a decompressor was
    /// already active (T1.2), or another recoverable protocol anomaly was seen — the
    /// session keeps parsing normally afterward, unlike `CompressionFailed` above. Carries a
    /// human-readable description naming the option/limit, for `remote.log`-style
    /// diagnostics.
    ProtocolError(String),
    /// A telnet option was just accepted in this direction — `WILL` answered
    /// with `DO`, or `DO` answered with `WILL` — carrying the option code
    /// (`TELNET_OPT_*`). This is `process_telnet`'s `gmcp_negotiated`/
    /// `msdp_negotiated` booleans generalized to *every* option Clay accepts
    /// today (SGA, EOR, NAWS, TTYPE, CHARSET, MCCP2, GMCP, MSDP, and — as of
    /// Job 12 — MSSP).
    ///
    /// Job 9 (finding 4, RFC 1143's Q method — see `OptionState`,
    /// `q_receive_will_wont`, `q_receive_do_dont`) backs this with real
    /// per-option state: fired exactly once per `No -> Yes` transition, not
    /// once per `WILL`/`DO` byte sequence that happens to arrive — a
    /// repeated `WILL`/`DO` for an already-enabled option gets no reply and
    /// no repeat event, which is the loop-prevention RFC 854 requires and
    /// `process_telnet`'s unconditional answer lacked.
    OptionEnabled(u8),
    /// The server withdrew an option with `WONT`/`DONT`, for an option Clay
    /// had accepted (see `OptionEnabled`) — carrying the option code. Job 9
    /// makes this real: fired exactly once, on the `Yes -> No` transition,
    /// and consumed by `App::handle_telnet_event` to clear the matching
    /// `World` mirror (`gmcp_enabled`, `msdp_enabled`, `naws_enabled`) —
    /// finding 4's "DONT/WONT are ignored entirely and never clear
    /// anything," fixed. `WONT`/`DONT` for an option that was never enabled
    /// (state already `No`) is the RFC 1143 steady state and fires nothing,
    /// same as before.
    OptionDisabled(u8),
    /// `IAC WONT ECHO` was seen. Mirrors `TelnetResult::wont_echo_seen`
    /// exactly: fires every time this sequence is seen, independent of
    /// whether a `Prompt` event also fires in the same call (see
    /// `extract_prompt`'s use below).
    ///
    /// Named for what this codebase actually *does* with it today, not for
    /// what the telnet option means in general: there is no echo-state
    /// tracking or input masking yet, so this is not "echo is off." It is a
    /// prompt-boundary hint — the sole existing consumer is
    /// `World::uses_wont_echo_prompt`, which arms the 150ms timeout-prompt
    /// path (servers that mark a prompt by refusing to echo it, instead of
    /// sending GA/EOR). `EchoOff`/`EchoOn` below carry the different, real
    /// meaning ECHO gets as of Job 10b (plan Phase 3, step 3.4): reusing
    /// either name here would have collided with that and meant two
    /// different things at different times in the codebase's history, so
    /// this heuristic keeps firing exactly as before, unconditionally
    /// alongside whichever of the two real events also fires for the same
    /// `WONT ECHO`.
    WontEchoPromptHint,
    /// `IAC WILL ECHO` was seen and answered `IAC DO ECHO` (finding 7): the
    /// standard way a server says "I will echo for you," which is how a MUD
    /// asks a client to stop echoing locally so a typed password isn't
    /// shown in the clear. Before Job 10b this fell through to the WILL
    /// match's generic refusal (`IAC DONT ECHO`) via `q_receive_will_wont`'s
    /// `support_him` check, so Clay never actually masked anything.
    ///
    /// ECHO deliberately stays outside the Q method's `support_him` table
    /// (Job 9's note in `support_him`'s doc comment): it is answered
    /// unconditionally every time it's seen, exactly like the pre-existing
    /// `WONT ECHO` special case below, not deduplicated against a No/Yes
    /// `options_him` state — real per-direction echo tracking would need its
    /// own state, which nothing here builds since Clay only ever needs to
    /// know "currently masked or not," which `App::handle_telnet_event`
    /// tracks on `World::echo_masked` (set here, cleared by `EchoOn`).
    EchoOff,
    /// `IAC WONT ECHO` was seen and answered `IAC DONT ECHO`, the real
    /// counterpart to `EchoOff`: the peer is no longer offering to echo for
    /// us, so any masking `EchoOff` armed should stop. Fires every time
    /// `IAC WONT ECHO` is seen, in addition to (never instead of)
    /// `WontEchoPromptHint` above — the two are independent consumers of the
    /// same wire event, one driving `World::echo_masked` and the other the
    /// unrelated prompt-boundary heuristic.
    EchoOn,
    /// A complete `!!SOUND(...)`/`!!MUSIC(...)` trigger was recognised and
    /// stripped out of the emitted text (Job 14, plan Phase 4). See
    /// `extract_msp_triggers` and `MspTrigger`. Gated on `cfg.msp_enabled` —
    /// this event is never produced at all when that's off (see
    /// `msp_enabled`'s doc comment).
    MspTrigger(MspTrigger),
}

/// One parsed MSP trigger (see `TelnetEvent::MspTrigger` /
/// `extract_msp_triggers`). Field defaults are MSP's own: volume 100, loops
/// 1, priority 50 — all three are already clamped to a sane range by
/// `parse_msp_trigger` (`clamp_msp_volume`/`clamp_msp_loops`/
/// `clamp_msp_priority`) before this event is ever produced, since `volume`
/// and `loops` end up driving a real audio sink (`audio::play_file`) and
/// every field here comes straight from the MUD.
#[derive(Debug, Clone, PartialEq)]
pub struct MspTrigger {
    /// `true` for `!!MUSIC(...)`, `false` for `!!SOUND(...)`.
    pub is_music: bool,
    /// `!!SOUND(Off)` / `!!MUSIC(Off)` (case-insensitive, per the MSP spec):
    /// stop playback. `name` is empty and every numeric field holds its
    /// default when this is set — there is nothing else to parse out of
    /// `Off`.
    pub off: bool,
    /// The file name (or, combined with `url` below, the tail of a URL) to
    /// play. Empty only when `off` is set. This is MUD-supplied text used to
    /// build a local file path when `url` is `None` — see
    /// `audio::resolve_msp_local_path` for why that is new attack surface
    /// GMCP `Client.Media.*` never had.
    pub name: String,
    /// `V=`, clamped to `0..=100`. Default 100.
    pub volume: i64,
    /// `L=`, clamped to `-1` (the spec's "loop forever" sentinel) or
    /// `1..=100`. Default 1.
    pub loops: i64,
    /// `P=`, clamped to `0..=100`. Default 50. Not used to decide whether one
    /// trigger interrupts another today — parsed and clamped because it
    /// reaches nothing unsafe either way, kept for a future consumer rather
    /// than dropped silently.
    pub priority: i64,
    /// `T=`: a free-form category/type string (e.g. "combat"). `None` when
    /// absent.
    pub media_type: Option<String>,
    /// `U=`: a base URL. When present, the real fetch target is
    /// `format!("{url}{name}")`, and it is routed through
    /// `audio::download_to_cache` — the same `validate_media_url` SSRF guard
    /// GMCP media already uses — never a second URL path. `None` means "play
    /// `name` from the local media cache directory" instead.
    pub url: Option<String>,
}

/// RFC 1143 Q method per-option, per-direction state (finding 4 / Job 9).
/// Flattened to six values with no separate state+queue-flag pair, matching
/// common Q method implementations (e.g. libtelnet's `telnet_q_t`).
///
/// `No`/`Yes` are reachable purely by *receiving* WILL/WONT/DO/DONT and
/// replying (see `q_receive_will_wont`/`q_receive_do_dont`) — Clay is fully
/// reactive and never initiates a negotiation itself. The four `Want*`
/// states — `WantNo`, `WantYes`, and both `*Opposite` variants — arise only
/// once something actively initiates a request (a disable, or a second
/// request queued behind one still in flight); implemented here in full, per
/// RFC 1143's tables, so a future job adding self-initiated negotiation does
/// not need to touch this type or its transition functions again, but
/// nothing in this codebase drives them yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OptionState {
    #[default]
    No,
    Yes,
    // Never constructed yet: nothing in this codebase decides on its own to
    // send `WONT`/`DONT` for an option it already has on, or `WILL`/`DO` to
    // request one be turned on, so neither `Yes -> WantNo` nor `No -> WantYes`
    // (via self-initiation) ever fires. Kept for RFC 1143 completeness.
    #[allow(dead_code)]
    WantNo,
    // `WantNoOpposite`/`WantYesOpposite` are never *constructed* yet (only
    // matched against, in the transition tables below) — reaching either
    // requires Clay to have already sent its own request and had a second,
    // opposite one queued behind it while waiting, which nothing here does.
    // Kept for RFC 1143 completeness (see `OptionState`'s doc comment)
    // rather than removed and re-added later.
    #[allow(dead_code)]
    WantNoOpposite,
    #[allow(dead_code)]
    WantYes,
    #[allow(dead_code)]
    WantYesOpposite,
}

/// Options Clay accepts a `WILL` for — the "him" direction of the Q method:
/// a capability the *peer* offers about itself, which Clay answers with
/// `DO`/`DONT`. Exactly `process_telnet`'s historical unconditional accept
/// list (finding 4's starting point) plus MSSP (Job 12 / plan Phase 4, 4.2 —
/// a server volunteering its own status is the same "him" shape as GMCP/MSDP
/// volunteering data about itself): Job 9 changes *when* Clay replies (once
/// per state change, via `q_receive_will_wont`, never every time the peer
/// repeats itself), not *which* options it accepts.
fn support_him(opt: u8) -> bool {
    matches!(
        opt,
        TELNET_OPT_SGA | TELNET_OPT_EOR | TELNET_OPT_GMCP | TELNET_OPT_MSDP
            | TELNET_OPT_MCCP2 | TELNET_OPT_CHARSET | TELNET_OPT_MSSP | TELNET_OPT_MSP
    )
}

/// Options Clay accepts a `DO` for — the "us" direction of the Q method:
/// something the peer asks *Clay itself* to do, answered with `WILL`/
/// `WONT`. Same starting list, and same "when, not which" caveat, as
/// `support_him`.
fn support_us(opt: u8) -> bool {
    matches!(opt, TELNET_OPT_NAWS | TELNET_OPT_TTYPE | TELNET_OPT_EOR)
}

/// Result of one `feed`/`flush_eof` call.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TelnetOutcome {
    /// Plain text ready to display; telnet sequences already removed.
    pub text: Vec<u8>,
    /// Bytes to write back to the server (negotiation replies).
    pub wire: Vec<u8>,
    pub events: Vec<TelnetEvent>,
}

/// Drain the trailing partial line (since the last `\n`) out of `text` as a
/// `TelnetEvent::Prompt`, and move whatever precedes it straight into
/// `out_text` — bypassing the text hold-back entirely, per the "flushed
/// unconditionally ... when a GA/EOR prompt boundary is crossed" rule. Used
/// both for GA/EOR (called unconditionally) and for the WONT-ECHO fallback
/// (called only when the caller has confirmed a prompt has not already
/// claimed this line). Does nothing if there is nothing after the last
/// newline to drain (mirrors process_telnet's `prompt_start < cleaned.len()`
/// guard exactly).
fn extract_prompt(
    text: &mut Vec<u8>,
    out_text: &mut Vec<u8>,
    events: &mut Vec<TelnetEvent>,
    prompt_open: &mut bool,
) {
    let last_newline = text.iter().rposition(|&b| b == b'\n');
    let prompt_start = last_newline.map(|p| p + 1).unwrap_or(0);
    if prompt_start < text.len() {
        let prompt: Vec<u8> = text.drain(prompt_start..).collect();
        out_text.append(text); // moves the newline-terminated (or empty) prefix out
        events.push(TelnetEvent::Prompt(prompt));
        *prompt_open = true;
    }
}

/// Clamp MSP `V=` to `0..=100` (plan Job 14). `volume`/`loops` reach a real
/// audio sink (`audio::play_file`) and come straight from the MUD, so they
/// are never trusted verbatim, unlike `T=`/`U=` which are just strings until
/// they're used elsewhere (`U=` still goes through its own guard,
/// `audio::validate_media_url`, at the point it's actually fetched).
fn clamp_msp_volume(v: i64) -> i64 {
    v.clamp(0, 100)
}

/// Clamp MSP `L=` to the spec's `-1` "loop forever" sentinel or `1..=100`
/// otherwise (plan Job 14). `0` and other non-`-1` negatives don't mean
/// anything in the protocol, so they clamp up to the default (`1`) rather
/// than being reinterpreted as "infinite" — only a literal `-1` gets that
/// meaning, matching `audio::play_file`'s own `loops == -1` check.
fn clamp_msp_loops(l: i64) -> i64 {
    if l == -1 { -1 } else { l.clamp(1, 100) }
}

/// Clamp MSP `P=` to `0..=100` (plan Job 14). See `MspTrigger::priority`'s
/// doc comment — parsed and clamped for a future consumer, not acted on
/// today.
fn clamp_msp_priority(p: i64) -> i64 {
    p.clamp(0, 100)
}

/// Parse the parameter text between a `!!SOUND(`/`!!MUSIC(` marker and its
/// closing `)` (already isolated by `extract_msp_triggers`) into an
/// `MspTrigger`. Never fails: an unrecognised token is silently ignored
/// (matches the "malformed trigger displays as text rather than vanishing"
/// posture at the extraction level — this function only ever sees text
/// `extract_msp_triggers` already decided was a *complete* trigger, so its
/// own job is just "make the best of whatever is inside the parens").
fn parse_msp_trigger(is_music: bool, params: &[u8]) -> MspTrigger {
    let params_str = String::from_utf8_lossy(params);
    let trimmed = params_str.trim();
    if trimmed.eq_ignore_ascii_case("off") {
        return MspTrigger {
            is_music,
            off: true,
            name: String::new(),
            volume: 100,
            loops: 1,
            priority: 50,
            media_type: None,
            url: None,
        };
    }

    let mut parts = trimmed.split_whitespace();
    let name = parts.next().unwrap_or("").to_string();
    let mut volume = 100i64;
    let mut loops = 1i64;
    let mut priority = 50i64;
    let mut media_type = None;
    let mut url = None;
    for tok in parts {
        if let Some(rest) = tok.strip_prefix("V=") {
            volume = clamp_msp_volume(rest.parse().unwrap_or(100));
        } else if let Some(rest) = tok.strip_prefix("L=") {
            loops = clamp_msp_loops(rest.parse().unwrap_or(1));
        } else if let Some(rest) = tok.strip_prefix("P=") {
            priority = clamp_msp_priority(rest.parse().unwrap_or(50));
        } else if let Some(rest) = tok.strip_prefix("T=") {
            media_type = Some(rest.to_string());
        } else if let Some(rest) = tok.strip_prefix("U=") {
            url = Some(rest.to_string());
        }
        // Anything else (an unrecognised parameter letter, stray text) is
        // ignored rather than rejecting the whole trigger — real-world MSP
        // senders are inconsistent about capitalisation of stray tokens and
        // there is nothing unsafe about ignoring one.
    }
    MspTrigger { is_music, off: false, name, volume, loops, priority, media_type, url }
}

/// Where to start holding `text` back because it might still contain (or end
/// with the start of) an MSP trigger the caller hasn't seen the whole of.
/// Checks two things and returns the earlier (more conservative) of whichever
/// apply:
///  - the last `!!SOUND(`/`!!MUSIC(` marker in `text` with neither a `)` nor
///    a `\n` anywhere after it — a marker that fully matched but hasn't been
///    resolved either way yet (`extract_msp_triggers`'s own loop already
///    catches this for a single `feed()` call via its `incomplete_from`, so
///    this branch only matters when `text` is a *carried-over* hold-back —
///    e.g. `idle_release_len` calling this directly on `pending_text` — that
///    was never re-scanned by that loop). Checking for `\n` too (not just
///    `)`) matters here: a marker the loop already resolved as malformed (no
///    `)` before end-of-line) leaves that `\n` sitting right there in `text`,
///    and without this check this branch would mistake that *settled*
///    malformed trigger for one still waiting on more input and hold it back
///    forever;
///  - failing that, a trailing proper prefix of either marker at least
///    `min_prefix` bytes long — the marker itself is still arriving, e.g.
///    `text` ending in `!!SOU`.
///
/// `min_prefix` lets callers tune how eager this is: `extract_msp_triggers`
/// passes 1, because `feed()` has a real next call coming that might supply
/// the rest of the marker and — per T2.1 — a bare trailing `!` must not be
/// flushed and lost if that next call turns it into `!!SOUND(`. The idle
/// flush passes 2 instead: after `IDLE_FLUSH_INTERVAL` of silence there is no
/// "next call" about to arrive, so a lone trailing `!` is far more likely to
/// be the end of an ordinary prompt (`"Hello!"`) than the start of a marker,
/// and holding it would mean never showing it until more output happens to
/// follow.
fn msp_holdback_start(text: &[u8], min_prefix: usize) -> Option<usize> {
    let markers: [&[u8]; 2] = [MSP_SOUND_MARKER, MSP_MUSIC_MARKER];

    let open = markers
        .iter()
        .filter_map(|marker| {
            text.windows(marker.len()).rposition(|w| w == *marker).filter(|&pos| {
                let after = &text[pos + marker.len()..];
                !after.contains(&b')') && !after.contains(&b'\n')
            })
        })
        .max();

    let max_len = (MSP_MARKER_LEN - 1).min(text.len());
    let prefix = (min_prefix..=max_len).rev().find_map(|len| {
        let tail = &text[text.len() - len..];
        if markers.iter().any(|m| m.starts_with(tail)) {
            Some(text.len() - len)
        } else {
            None
        }
    });

    match (open, prefix) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Scan `text` for complete `!!SOUND(...)`/`!!MUSIC(...)` triggers, strip
/// each one out and push an equivalent `TelnetEvent::MspTrigger`. Returns
/// `Some(offset)` when `text` ends with a marker that opened but has not yet
/// seen its closing `)` (nor a `\n` — see below) — the caller must hold that
/// suffix back across `feed()` calls (up to `MAX_MSP_TRIGGER_HOLDBACK`)
/// rather than flush it, or a trigger split across two reads renders as
/// broken text instead of playing.
///
/// A trigger is resolved one of three ways, all decided by whichever comes
/// first after the marker: `)` closes it (strip + emit); `\n` ends the line
/// with no `)` ever appearing, which is what "malformed" means here — MSP
/// triggers don't span lines, so this is left untouched as ordinary text,
/// exactly per the plan's "a malformed trigger should display as ordinary
/// text rather than being silently swallowed" requirement, and scanning
/// resumes right after that newline; or currently-available `text` runs out
/// with neither seen, which is the split case above.
///
/// Deliberately does not scan text already carved into a `TelnetEvent::Prompt`
/// by a GA/EOR/WONT-ECHO boundary (see `feed`'s call site) — a trigger placed
/// on the same line as such a prompt is not recognised. MSP triggers seen in
/// the wild are asynchronous room/event triggers on ordinary output lines,
/// not short status-line prompts, so this is a deliberate scope limit rather
/// than an oversight.
fn extract_msp_triggers(text: &mut Vec<u8>, events: &mut Vec<TelnetEvent>) -> Option<usize> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    let mut incomplete_from = None;
    while i < text.len() {
        let rest = &text[i..];
        let is_music = rest.starts_with(MSP_MUSIC_MARKER);
        if is_music || rest.starts_with(MSP_SOUND_MARKER) {
            let param_start = i + MSP_MARKER_LEN;
            let mut j = param_start;
            let mut close_at = None;
            while j < text.len() {
                match text[j] {
                    b')' => { close_at = Some(j); break; }
                    b'\n' => break,
                    _ => {}
                }
                j += 1;
            }
            match close_at {
                Some(close) => {
                    events.push(TelnetEvent::MspTrigger(parse_msp_trigger(is_music, &text[param_start..close])));
                    i = close + 1;
                }
                None if j < text.len() => {
                    // Hit '\n' before ')': malformed - leave the marker and
                    // everything up to (not including) the newline as
                    // ordinary text.
                    out.extend_from_slice(&text[i..j]);
                    i = j;
                }
                None => {
                    // Ran off the end of what's arrived so far with neither
                    // ')' nor '\n' seen - may still complete on the next
                    // feed() call.
                    incomplete_from = Some(out.len());
                    out.extend_from_slice(&text[i..]);
                    i = text.len();
                }
            }
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    // T2.1: the loop above only recognises an in-progress trigger once the
    // full 8-byte marker has matched. A read boundary landing *inside* the
    // marker itself (`out` ending in `!!SOU`) is invisible to it — those
    // bytes went through the `else` arm one at a time as ordinary text,
    // since `rest.starts_with(marker)` can't match a `rest` shorter than the
    // marker. Catch that here: a trailing proper prefix of either marker
    // (or, from `msp_holdback_start`'s other branch, a still-open marker —
    // structurally unreachable in *this* caller, since the loop already
    // claims that case via `incomplete_from`, but the same helper also
    // serves `idle_release_len` where it isn't) is exactly as "not yet known
    // to be complete" as the closer-to-the-end case the loop already
    // handles.
    if incomplete_from.is_none() {
        incomplete_from = msp_holdback_start(&out, 1);
    }
    *text = out;
    incomplete_from
}

/// How many bytes at the very end of `text` form an incomplete UTF-8 code
/// point (0..=3 — a full sequence is at most 4 bytes, so at most 3 of them
/// can ever be "still waiting on more"). Walks backwards from the end: a
/// continuation byte (`10xxxxxx`) just means keep walking; the first
/// non-continuation byte found is either a lead byte that needed more
/// continuation bytes than it got (incomplete — return how many bytes back
/// that lead byte was) or anything else (ASCII, a lead byte that already got
/// enough continuation bytes, or a byte that was never valid UTF-8 at all,
/// e.g. a literal `0xFF` telnet escape already unescaped to a plain byte by
/// `feed` — complete, nothing to hold, return 0).
fn incomplete_utf8_tail_len(text: &[u8]) -> usize {
    let len = text.len();
    let max_back = 3.min(len);
    for back in 1..=max_back {
        let b = text[len - back];
        if b & 0xC0 == 0x80 {
            // Continuation byte - the lead byte (if any) is further back.
            continue;
        }
        let needed = if b < 0x80 {
            1
        } else if b & 0xE0 == 0xC0 {
            2
        } else if b & 0xF0 == 0xE0 {
            3
        } else if b & 0xF8 == 0xF0 {
            4
        } else {
            // Not a valid UTF-8 lead byte at all - nothing to hold on its
            // account.
            return 0;
        };
        return if needed > back { back } else { 0 };
    }
    // Every one of the (at most 3) trailing bytes examined was a
    // continuation byte with no lead byte found in range - a real 4-byte
    // sequence has only 3 continuation bytes total, so this can't be
    // completed by holding back any of them either.
    0
}

/// How many bytes at the front of `text` are safe to release right now
/// without risking showing a truncated MSP trigger marker, ANSI CSI
/// sequence, or UTF-8 code point. Used only by the idle flush
/// (`TelnetSession::take_idle_flushable_text`) — `feed()`'s own hold-back
/// (see the struct doc comment's "Text hold-back" section) makes a coarser
/// "hold the whole tail, or don't" call because it still has a next `feed()`
/// call coming that might complete things; the idle flush doesn't, so it
/// needs the precise boundary instead of an all-or-nothing choice.
///
/// Applies, in order, each rule narrowing what the previous one already
/// allowed: an open-or-prefix MSP marker (`msp_holdback_start` with a
/// minimum prefix of 2, so a lone trailing `!` from an ordinary prompt like
/// `"Hello!"` is released rather than mistaken for the start of a trigger —
/// `feed()` itself uses 1, since unlike the idle flush it does have a next
/// read that might turn that `!` into a real marker), then an unterminated
/// ANSI escape, then an incomplete trailing UTF-8 code point.
fn idle_release_len(text: &[u8]) -> usize {
    let after_msp = msp_holdback_start(text, 2).unwrap_or(text.len());
    let after_escape = unterminated_escape_start(&text[..after_msp]).unwrap_or(after_msp);
    after_escape - incomplete_utf8_tail_len(&text[..after_escape])
}

/// A pure, synchronous, stateful telnet parser with real carry-over across
/// `feed` calls. No tokio, no I/O, no `App` — see Design commitment 1.
///
/// # Framing carry-over (`inbuf`)
///
/// Raw bytes that don't yet form a complete telnet token stay in `inbuf`
/// across calls: a lone trailing `IAC`; `IAC` plus a negotiation verb with no
/// option byte yet; an `IAC SB ...` with no `IAC SE` yet (unless it has grown
/// past `MAX_SUBNEG_BYTES`, in which case it is abandoned with a
/// `TelnetEvent::ProtocolError` instead of retained). There is no index-0
/// special case — this is what fixes finding 3's `find_safe_split_point`
/// hole, where an incomplete `IAC SB` starting at buffer offset 0 was
/// invisible to that function's backward scan and got sent as if it were
/// complete.
///
/// # Text hold-back (`pending_text`)
///
/// `pending_text` holds whatever plain text has been scanned but not yet
/// released in an `outcome.text`. At the end of every `feed` call (and,
/// separately, at every GA/EOR/WONT-ECHO prompt boundary, which flushes
/// unconditionally instead) the *tail since the last newline* decides what
/// actually gets flushed: if that tail is empty, everything flushes; if it's
/// non-empty and no longer than `MAX_TEXT_HOLDBACK` (32 bytes), only the part
/// up to and including the last newline is flushed and the tail is kept for
/// the next call; if the tail is longer than that, the cap gives up and
/// flushes everything anyway rather than holding forever.
///
/// This one rule does double duty. It is what the plan asks for literally —
/// holding back an incomplete trailing ANSI CSI sequence or a truncated
/// UTF-8 code point, scoped to *emitted text* rather than the raw wire
/// (Design commitment 4), which is what stops a `0x1B` byte inside a GMCP
/// payload from being mistaken for one (that byte never reaches `text` at
/// all — it is consumed as subnegotiation payload). It is *also* what makes
/// GA/EOR and WONT-ECHO prompt extraction give the same answer no matter
/// where a read happens to split the bytes in front of the prompt marker: a
/// short trailing line with no newline yet is exactly as "not known to be
/// complete" as an incomplete escape sequence is, and the split-invariance
/// tests below require the same prompt content regardless of where a read
/// boundary falls. `process_telnet` cannot get this right at all — it has no
/// carry-over, so a prompt marker arriving in a later read than the text in
/// front of it just loses that text — so there is no fixture pinning that
/// behaviour to preserve; this is new, correct behaviour, not a divergence
/// from anything the characterization table asserts.
///
/// T2.1 folds an in-progress MSP trigger into the same tail (see
/// `extract_msp_triggers`/`msp_holdback_start`): a trailing proper prefix of
/// `!!SOUND(`/`!!MUSIC(`, or a fully-matched marker with no closing `)` yet,
/// is held from `pos` under the larger `MAX_MSP_TRIGGER_HOLDBACK` cap instead
/// of `MAX_TEXT_HOLDBACK` whenever the ordinary rule above wouldn't already
/// cover it — never *instead* of the ordinary rule, only in addition to it,
/// since holding only from `pos` would flush an ordinary short line ending in
/// `!` (a name prompt, say) as far as `pos` and hand GA/EOR a truncated
/// prompt. See `feed`'s hold-decision code comment for the exact rule.
///
/// # Idle-flush release (`take_idle_flushable_text`)
///
/// `feed`'s hold-back above is deliberately all-or-nothing per call — it
/// always has another `feed` call that might complete things. The 150ms idle
/// timer (`spawn_telnet_reader`, T2.2) has no such call coming, so draining
/// `pending_text` wholesale there could show a truncated MSP trigger marker,
/// UTF-8 code point, or CSI sequence — exactly what `MAX_MSP_TRIGGER_HOLDBACK`
/// and the ordinary hold-back exist to prevent. `take_idle_flushable_text`
/// uses `idle_release_len` instead: release only the prefix that helper
/// certifies as safe, and leave the remainder — if any — in `pending_text`
/// for the next `feed` call or a later idle flush to finish.
pub struct TelnetSession {
    /// Read by `mtts_answer`/`mtts_bitmask` (Job 12, plan Phase 4, 4.1). Still
    /// unread by anything else - CHARSET-outbound (Job 13's `Encoding::encode`
    /// work) is the next consumer the struct doc comment above predicts.
    cfg: TelnetConfig,
    /// Raw bytes not yet fully parsed. Either a genuinely incomplete telnet
    /// sequence at the tail (see above), or — once `compression_active` is
    /// set — opaque post-MCCP2-activation bytes that Job 3's decompressor
    /// will consume.
    inbuf: Vec<u8>,
    /// See "Text hold-back" above.
    pending_text: Vec<u8>,
    /// Bumped on every TTYPE SEND request. Phase 4.1's MTTS cycling will
    /// read this to choose between the full name, a "poor man's" fallback,
    /// and an `MTTS <bits>` response; Job 2 does not vary its behaviour on
    /// it yet.
    ttype_index: u8,
    /// `Some` from the moment `IAC SB MCCP2 IAC SE` activates compression
    /// until the zlib stream reports `Status::StreamEnd`. Owning the
    /// decompressor here — rather than in a caller-owned buffer, as every
    /// production reader loop does today — is what makes finding 7's
    /// leftover bug inexpressible: there is no code path by which bytes can
    /// reach `inbuf`/`text` while this is `Some` except by first passing
    /// through `Decompress::decompress` inside `run_decompressor`.
    decomp: Option<flate2::Decompress>,
    /// Raw MCCP2 bytes received but not yet consumed by `decomp` — normal
    /// streaming-decompression carry (a zlib block boundary need not land
    /// on a `feed` call boundary), not a second copy of the leftover bug:
    /// these bytes are opaque compressed data, never treated as telnet or
    /// exposed as text, and are only ever handed to `decomp` — inside
    /// `run_decompressor` — at the start of the next call that has one.
    /// Always empty while `decomp` is `None`.
    pending_compressed: Vec<u8>,
    /// Set once, permanently, when the MCCP2 zlib stream is found unrecoverably corrupt
    /// (an inflate error or a bomb-guard trip — see `TelnetEvent::CompressionFailed`,
    /// design decision D1). Every `feed()` call checks this first and returns
    /// `TelnetOutcome::default()` structurally once set — there is no reset, and no code
    /// path re-parses anything as telnet after this, by construction: the whole point is
    /// that a poisoned zlib stream has no resync point, so nothing after it can be trusted.
    /// `flush_eof()` deliberately does NOT check this — whatever text was already safely
    /// held back in `pending_text` before poisoning is still real, undecoded plaintext and
    /// must still reach the caller when the reader's fatal sequence flushes it.
    poisoned: bool,
    /// True from the moment a GA/EOR/WONT-ECHO prompt extraction fires until
    /// the next literal `\n` is scanned. Suppresses a *second* WONT-ECHO
    /// extraction for the same still-open line. `process_telnet` has the
    /// same rule (`prompt.is_none()`), but only ever needs a local variable
    /// because a whole call's `cleaned` buffer never survives past its
    /// `return`; a session has to carry the equivalent state across calls so
    /// it gives the identical answer even when the WONT ECHO and the GA that
    /// already claimed the line land in different `feed` calls.
    prompt_open: bool,
    /// Set the first time an IAC byte is ever seen, so `TelnetEvent::TelnetDetected`
    /// is emitted exactly once per session lifetime rather than once per
    /// `feed` call whose chunk happens to contain the (possibly carried-over)
    /// telnet activity.
    telnet_seen: bool,
    /// RFC 1143 Q method "him" state per option code (0-255): whether the
    /// *peer* has each option enabled about itself. Indexed by option code.
    /// Boxed so a session stays cheap to construct despite the 256-entry
    /// table (Job 9 / finding 4). See `q_receive_will_wont`.
    options_him: Box<[OptionState; 256]>,
    /// RFC 1143 Q method "us" state per option code: whether *Clay* has
    /// each option enabled, from the peer's `DO`/`DONT` requests. See
    /// `q_receive_do_dont`.
    options_us: Box<[OptionState; 256]>,
}

impl TelnetSession {
    pub fn new(cfg: TelnetConfig) -> Self {
        TelnetSession {
            cfg,
            inbuf: Vec::new(),
            pending_text: Vec::new(),
            ttype_index: 0,
            decomp: None,
            pending_compressed: Vec::new(),
            poisoned: false,
            prompt_open: false,
            telnet_seen: false,
            options_him: Box::new([OptionState::default(); 256]),
            options_us: Box::new([OptionState::default(); 256]),
        }
    }

    /// Interpret one complete subnegotiation payload (the bytes strictly
    /// between `IAC SB` and `IAC SE`, already de-terminated). The option
    /// byte at `sb_data[0]` (and, for CHARSET, the opcode at `sb_data[1]`)
    /// are structure, never escaped in practice, and are matched on the raw
    /// slice; the payload *past* those framing bytes is un-doubled via
    /// `unescape_iac` before it reaches `parse_msdp_pairs`, the GMCP
    /// package/JSON split, or `parse_charset_request` (finding 6 / Phase
    /// 3.1). Returns `true` iff this was a genuine MCCP2 activation
    /// subnegotiation, telling the caller (`feed`) to create the
    /// decompressor and decompress the rest of this call's buffer in place
    /// before continuing to parse the same buffer (Job 3) — that offset is
    /// computed by the caller from the raw buffer, not from anything
    /// unescaped here.
    ///
    /// T1.2 (plan Job 1, step 1): activation requires `options_him[MCCP2] ==
    /// Yes` (the peer's `WILL MCCP2` was actually negotiated — Clay having
    /// replied `DO MCCP2` to the server's own offer) *and* no decompressor
    /// already active. Without the first check, a server could turn on compression
    /// without ever offering the option; without the second, the four
    /// activation bytes appearing again *inside* an already-live compressed
    /// stream — Trigger B, entirely plausible as ordinary decompressed
    /// content that happens to contain this exact byte sequence — would
    /// silently replace `self.decomp`, discarding the live zlib window and
    /// feeding already-decompressed plaintext to a fresh decompressor as if
    /// it were still compressed. Design decision D1: an unrequested or
    /// duplicate activation is not itself a failure (no `poisoned`, no
    /// disconnect) — it's ignored with a `ProtocolError` and whatever
    /// decompressor is already live (if any) keeps running untouched.
    fn handle_subnegotiation(
        &mut self,
        sb_data: &[u8],
        wire: &mut Vec<u8>,
        events: &mut Vec<TelnetEvent>,
    ) -> bool {
        if sb_data.is_empty() {
            return false;
        }
        if sb_data[0] == TELNET_OPT_MCCP2 {
            let negotiated = self.options_him[TELNET_OPT_MCCP2 as usize] == OptionState::Yes;
            if negotiated && self.decomp.is_none() {
                events.push(TelnetEvent::CompressionStarted);
                return true;
            }
            events.push(TelnetEvent::ProtocolError(
                "IAC SB MCCP2 IAC SE ignored: MCCP2 was not negotiated, or a decompressor is \
                 already active"
                    .to_string(),
            ));
            return false;
        }
        if sb_data.len() >= 2 && sb_data[0] == TELNET_OPT_TTYPE && sb_data[1] == TTYPE_SEND {
            // MTTS cycling (plan Job 12 / 4.1): answer directly here, in
            // `wire`, rather than leaving it to `App` (see `TtypeRequested`'s
            // doc comment) - only this method's `&mut self` can see
            // `ttype_index`, which is what picks the cycle position.
            self.ttype_index = self.ttype_index.wrapping_add(1);
            wire.extend_from_slice(&build_ttype_response(&self.mtts_answer()));
            events.push(TelnetEvent::TtypeRequested);
        }
        if sb_data.len() >= 2 && sb_data[0] == TELNET_OPT_GMCP {
            let payload = unescape_iac(&sb_data[1..]);
            if let Ok(text) = std::str::from_utf8(&payload) {
                let (package, json) = if let Some(pos) = text.find(' ') {
                    (text[..pos].to_string(), text[pos + 1..].trim().to_string())
                } else {
                    (text.to_string(), String::new())
                };
                events.push(TelnetEvent::GmcpMessage(package, json));
            }
        }
        if sb_data.len() >= 2 && sb_data[0] == TELNET_OPT_MSDP {
            let payload = unescape_iac(&sb_data[1..]);
            for (var, val) in parse_msdp_pairs(&payload) {
                events.push(TelnetEvent::MsdpVariable(var, val));
            }
        }
        if sb_data.len() >= 3 && sb_data[0] == TELNET_OPT_CHARSET {
            if sb_data[1] == CHARSET_REQUEST {
                let payload = unescape_iac(&sb_data[1..]);
                events.push(TelnetEvent::CharsetRequest(parse_charset_request(&payload)));
            } else if sb_data[1] == CHARSET_TTABLE_IS {
                // We don't support TTABLEs - reject, exactly like process_telnet.
                wire.extend_from_slice(&[
                    TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_TTABLE_REJECTED,
                    TELNET_IAC, TELNET_SE,
                ]);
            }
        }
        if sb_data[0] == TELNET_OPT_MSSP {
            let payload = unescape_iac(&sb_data[1..]);
            events.push(TelnetEvent::MsspData(parse_mssp_pairs(&payload)));
        }
        false
    }

    /// The MTTS answer for the request that just bumped `ttype_index` (plan
    /// Job 12 / 4.1): the client name on the first request, the terminal
    /// type on the second, and `MTTS <bitmask>` on the third and every one
    /// after that — the fixed point is how a server following the MTTS
    /// convention knows it has seen the client's whole list and can stop
    /// asking. `ttype_index` only ever increments (see the TTYPE SEND branch
    /// above), so once it passes 2 every later call keeps landing on the
    /// same `_` arm with the same bitmask, satisfying "repeats the last one
    /// forever."
    fn mtts_answer(&self) -> String {
        match self.ttype_index {
            1 => self.cfg.client_name.to_uppercase(),
            2 => self.cfg.term_type.to_uppercase(),
            _ => format!("MTTS {}", self.mtts_bitmask()),
        }
    }

    /// The MTTS bitmask Clay reports on its third-and-later TTYPE answer
    /// (plan Job 12 / 4.1). Deliberately not the "claim everything" mask:
    /// - `MTTS_ANSI`/`MTTS_UTF8`: Clay is a terminal client that parses ANSI
    ///   SGR sequences and speaks UTF-8 end to end (`encoding.rs`) — always
    ///   substantiated, no config to check.
    /// - `MTTS_256_COLOR`/`MTTS_TRUECOLOR`: confirmed in `rendering.rs`'s ANSI
    ///   parser, which turns `38;5;N`/`48;5;N` into `Color::Indexed` and
    ///   `38;2;r;g;b`/`48;2;r;g;b` into `Color::Rgb` (24-bit) — both are real
    ///   rendering paths, not aspirational.
    /// - `MTTS_SSL`: from `cfg.is_tls`, the one bit that depends on the
    ///   specific connection rather than being universally true of Clay.
    /// - Never set: `MTTS_VT100` (Clay does no VT100 line-drawing character
    ///   remapping - it passes UTF-8 through, it doesn't translate for a
    ///   dumber terminal), `MTTS_MOUSE_TRACKING`/`MTTS_SCREEN_READER`/
    ///   `MTTS_PROXY`/`MTTS_MNES` (no such feature exists),
    ///   `MTTS_OSC_COLOR_PALETTE` (Clay never emits an OSC 4/104 palette
    ///   remap — its own OSC8 hyperlink support is an unrelated OSC code).
    /// - `MTTS_MSLP` (Job 14, plan Phase 4): investigated and deliberately
    ///   left unset, same as the pre-existing MNES omission. Job 14's own
    ///   prep note assumed the rendering half was free because Clay already
    ///   emits OSC 8 hyperlinks — checking the real MSLP spec
    ///   (mudhalla.net/tintin/protocols/mslp) shows that assumption doesn't
    ///   hold: Clay's OSC 8 use is "open this URL in a browser," but MSLP's
    ///   actual payload is click-to-*send-a-command-to-the-MUD* (its
    ///   "complex" form is `ESC ] 68 ; 1 ; ; LABEL ; command BEL`, e.g.
    ///   `SEND`/`PROMPT`, wrapping an underlined span; its "simple" form
    ///   reinterprets *plain* `ESC[4m`/`ESC[24m` underline — styling every
    ///   MUD already sends for ordinary emphasis — as clickable-send by
    ///   convention alone). Building that is new interaction machinery the
    ///   console has no equivalent of (nothing today lets a keypress or
    ///   click inject a command into a world) and the web/GUI client would
    ///   need it independently (its OSC 8 rendering is equally URL-only).
    ///   Given "very low usage, new" in the plan's own protocol-landscape
    ///   table, that effort is disproportionate — and the "simple" form is
    ///   outright risky to implement at all: turning ordinary underlined
    ///   text into a clickable server command by default would misfire on
    ///   any MUD that underlines for emphasis rather than for a link. Not
    ///   implemented; the bit stays clear so as not to advertise a
    ///   capability Clay does not have (CLAUDE.md's MTTS-bit/implementation
    ///   coupling).
    fn mtts_bitmask(&self) -> u16 {
        let mut bits = MTTS_ANSI | MTTS_UTF8 | MTTS_256_COLOR | MTTS_TRUECOLOR;
        if self.cfg.is_tls {
            bits |= MTTS_SSL;
        }
        bits
    }

    /// RFC 1143 Q method: apply receipt of `WILL` (`enable = true`) or
    /// `WONT` (`enable = false`) for `opt` to its "him" state — the peer
    /// announcing or withdrawing a capability of its own — writing any
    /// reply this transition requires into `wire` and any
    /// `OptionEnabled`/`OptionDisabled` event into `events`. This is
    /// finding 4's actual fix: a reply and an event go out only on a real
    /// `No <-> Yes` state change, never on a repeat.
    ///
    /// Only `No`/`Yes` are reachable today, from ordinary receive-only
    /// traffic (see `OptionState`'s doc comment) — Clay is fully reactive
    /// and never initiates a negotiation of its own, so `options_him` never
    /// starts out anywhere but `No`. `WantNo`/`WantNoOpposite`/`WantYes`/
    /// `WantYesOpposite` remain RFC 1143's tables in full, not dead guesses,
    /// against the day something in this codebase does initiate.
    fn q_receive_will_wont(
        &mut self,
        opt: u8,
        enable: bool,
        wire: &mut Vec<u8>,
        events: &mut Vec<TelnetEvent>,
    ) {
        let st = &mut self.options_him[opt as usize];
        if enable {
            match *st {
                OptionState::No => {
                    if support_him(opt) {
                        *st = OptionState::Yes;
                        wire.extend_from_slice(&[TELNET_IAC, TELNET_DO, opt]);
                        events.push(TelnetEvent::OptionEnabled(opt));
                    } else {
                        wire.extend_from_slice(&[TELNET_IAC, TELNET_DONT, opt]);
                    }
                }
                // Finding 4's loop-prevention fix: already enabled, so a
                // repeated WILL gets no reply and no repeat event.
                OptionState::Yes => {}
                OptionState::WantNo => *st = OptionState::No, // error: DONT answered by WILL
                OptionState::WantNoOpposite => *st = OptionState::Yes,
                // Unreachable today (nothing puts `options_him` into
                // `WantYes` since Clay never initiates), kept per RFC 1143's
                // full table: if something ever does initiate a `DO opt`,
                // this is the "just turned on" confirm transition, firing
                // `OptionEnabled` exactly like the `No -> Yes` arm above.
                OptionState::WantYes => {
                    *st = OptionState::Yes;
                    events.push(TelnetEvent::OptionEnabled(opt));
                }
                OptionState::WantYesOpposite => {
                    *st = OptionState::WantNo;
                    wire.extend_from_slice(&[TELNET_IAC, TELNET_DONT, opt]);
                }
            }
        } else {
            match *st {
                OptionState::No => {} // already off; ignore, matches RFC 1143
                OptionState::Yes => {
                    *st = OptionState::No;
                    wire.extend_from_slice(&[TELNET_IAC, TELNET_DONT, opt]);
                    events.push(TelnetEvent::OptionDisabled(opt));
                }
                OptionState::WantNo => *st = OptionState::No,
                OptionState::WantNoOpposite => {
                    *st = OptionState::WantYes;
                    wire.extend_from_slice(&[TELNET_IAC, TELNET_DO, opt]);
                }
                OptionState::WantYes => *st = OptionState::No, // error: DO answered by WONT
                OptionState::WantYesOpposite => *st = OptionState::No,
            }
        }
    }

    /// RFC 1143 Q method: apply receipt of `DO` (`enable = true`)/`DONT`
    /// (`enable = false`) for `opt` to its "us" state — something the peer
    /// asks *Clay itself* to do. Mirror image of `q_receive_will_wont`; see
    /// its doc comment for the shared rationale.
    ///
    /// Returns `true` exactly when this call just turned the option on: the
    /// `No -> Yes` accept transition (peer asked first — the only path Clay
    /// takes today, since it never initiates a negotiation of its own). The
    /// `WantYes -> Yes` confirm transition is kept for RFC 1143 completeness
    /// against a future self-initiating call site; `NawsRequested` (fired
    /// alongside `OptionEnabled(NAWS)` on `became_enabled`, see `feed`'s
    /// `TELNET_DO` arm) firing on `became_enabled` rather than only on the
    /// `No -> Yes` arm keeps both paths correct without extra plumbing if
    /// that day comes.
    fn q_receive_do_dont(
        &mut self,
        opt: u8,
        enable: bool,
        wire: &mut Vec<u8>,
        events: &mut Vec<TelnetEvent>,
    ) -> bool {
        let mut became_enabled = false;
        let st = &mut self.options_us[opt as usize];
        if enable {
            match *st {
                OptionState::No => {
                    if support_us(opt) {
                        *st = OptionState::Yes;
                        wire.extend_from_slice(&[TELNET_IAC, TELNET_WILL, opt]);
                        events.push(TelnetEvent::OptionEnabled(opt));
                        became_enabled = true;
                    } else {
                        wire.extend_from_slice(&[TELNET_IAC, TELNET_WONT, opt]);
                    }
                }
                OptionState::Yes => {}
                OptionState::WantNo => *st = OptionState::No,
                OptionState::WantNoOpposite => *st = OptionState::Yes,
                // Unreachable today — see the doc comment above — kept for
                // RFC 1143 completeness: if Clay's own `WILL`/`DO` request
                // ever gets confirmed here, that's a real enable, not a
                // repeat.
                OptionState::WantYes => {
                    *st = OptionState::Yes;
                    events.push(TelnetEvent::OptionEnabled(opt));
                    became_enabled = true;
                }
                OptionState::WantYesOpposite => {
                    *st = OptionState::WantNo;
                    wire.extend_from_slice(&[TELNET_IAC, TELNET_WONT, opt]);
                }
            }
        } else {
            match *st {
                OptionState::No => {}
                OptionState::Yes => {
                    *st = OptionState::No;
                    wire.extend_from_slice(&[TELNET_IAC, TELNET_WONT, opt]);
                    events.push(TelnetEvent::OptionDisabled(opt));
                }
                OptionState::WantNo => *st = OptionState::No,
                OptionState::WantNoOpposite => {
                    *st = OptionState::WantYes;
                    wire.extend_from_slice(&[TELNET_IAC, TELNET_WILL, opt]);
                }
                OptionState::WantYes => *st = OptionState::No,
                OptionState::WantYesOpposite => *st = OptionState::No,
            }
        }
        became_enabled
    }

    /// Run `raw` (freshly-arrived MCCP2 bytes, prefixed by any carry left in
    /// `self.pending_compressed`) through `self.decomp`, and return the
    /// bytes that are now safe to treat as telnet plaintext. This is the
    /// ONLY place bytes cross from "compressed" to "plaintext" — see the
    /// `decomp` field doc comment for why that is what makes finding 7's
    /// leftover-without-decompressing bug inexpressible. Requires
    /// `self.decomp.is_some()` (the caller always arranges this first).
    ///
    /// On `Status::StreamEnd`, the decompressor is dropped and a
    /// `TelnetEvent::CompressionEnded` is pushed; whatever of `raw` the
    /// zlib stream didn't need (the bytes after its own end) are ordinary
    /// telnet again and are appended to the returned plaintext — rather
    /// than kept as compressed carry — so they get parsed in this same
    /// call wherever possible instead of waiting on another `feed`.
    ///
    /// T1.1/T1.14 (plan Job 1, step 3, design decision D1): an inflate error, a
    /// `MCCP2_BOMB_LIMIT` trip, or an unconsumed remainder past
    /// `MAX_PENDING_COMPRESSED` (defence in depth — real streaming carry is a few bytes,
    /// never tens of thousands) all poison the session identically: drop `decomp`, clear
    /// `pending_compressed` (the fix for T1.1's leftover-retention bug — before this,
    /// an inflate error left `consumed == 0`-ish and the *entire* raw buffer piled into
    /// `pending_compressed` forever, re-failing the same way on every later `feed()`
    /// while producing no output), set `self.poisoned`, and push a single
    /// `TelnetEvent::CompressionFailed(reason)` instead of `ProtocolError`. Whatever
    /// plaintext was already legitimately decompressed *before* the failure point is
    /// still returned and parsed normally in this same call (it came from real inflate
    /// output, not raw compressed noise) — `poisoned` only takes effect starting with the
    /// *next* `feed()` call, via its own top-of-function check.
    fn run_decompressor(&mut self, raw: Vec<u8>, events: &mut Vec<TelnetEvent>) -> Vec<u8> {
        let decomp = self
            .decomp
            .as_mut()
            .expect("run_decompressor requires an active decompressor");
        let step = mccp2_decompress_capped(decomp, &raw);

        if let Some(reason) = step.error {
            self.decomp = None;
            self.pending_compressed.clear();
            self.poisoned = true;
            events.push(TelnetEvent::CompressionFailed(reason));
            return step.output;
        }
        if step.bomb_hit {
            self.decomp = None;
            self.pending_compressed.clear();
            self.poisoned = true;
            events.push(TelnetEvent::CompressionFailed(format!(
                "MCCP2 decompression exceeded {MCCP2_BOMB_LIMIT} bytes in a single feed() call"
            )));
            return step.output;
        }
        let leftover = raw[step.consumed..].to_vec();
        if step.stream_end {
            self.decomp = None;
            events.push(TelnetEvent::CompressionEnded);
            let mut out = step.output;
            out.extend_from_slice(&leftover);
            out
        } else if leftover.len() > MAX_PENDING_COMPRESSED {
            self.decomp = None;
            self.pending_compressed.clear();
            self.poisoned = true;
            events.push(TelnetEvent::CompressionFailed(format!(
                "MCCP2 decompressor made no progress on more than {MAX_PENDING_COMPRESSED} \
                 unconsumed bytes"
            )));
            step.output
        } else {
            self.pending_compressed = leftover;
            step.output
        }
    }

    /// Feed newly-received bytes into the session. May be called with any
    /// chunking whatsoever — see the split-invariance tests below.
    ///
    /// Design decision D1 / plan Job 1 step 4: once `self.poisoned` (a corrupted MCCP2
    /// stream, see `TelnetEvent::CompressionFailed`), every subsequent call returns
    /// `TelnetOutcome::default()` structurally — no text, no wire, no events — regardless
    /// of `data`. There is no resync point in a broken zlib stream, so nothing arriving
    /// after the failure is ever safe to parse as telnet; the reader is expected to have
    /// already disconnected by the time anything would call `feed()` again, but this is
    /// the structural guarantee, not just reader discipline.
    pub fn feed(&mut self, data: &[u8]) -> TelnetOutcome {
        if self.poisoned {
            return TelnetOutcome::default();
        }
        let mut wire = Vec::new();
        let mut events = Vec::new();

        if self.decomp.is_some() {
            // MCCP2 is active: `data` is compressed, and the only way any
            // of it (or carry from a previous call) reaches `inbuf` is by
            // first passing through `run_decompressor` — see the `decomp`
            // field doc comment.
            let mut raw = std::mem::take(&mut self.pending_compressed);
            raw.extend_from_slice(data);
            let plaintext = self.run_decompressor(raw, &mut events);
            self.inbuf.extend_from_slice(&plaintext);
        } else {
            self.inbuf.extend_from_slice(data);
        }

        // Take full ownership of the accumulated bytes for this call. `buf`
        // is now a plain local Vec, not a borrow of `self`, so the rest of
        // this function is free to take `&mut self` (e.g. `self.ttype_index`,
        // `self.decomp`) without any borrow-checker conflict. It is `mut`
        // because MCCP2 activation below splices freshly-decompressed bytes
        // into it in place and keeps parsing the same buffer, rather than
        // stopping and handing the tail to a second buffer (finding 7).
        let mut buf = std::mem::take(&mut self.inbuf);

        let mut out_text = Vec::new();
        // Seed with whatever was held back last call; see the struct doc
        // comment's "Text hold-back" section.
        let mut text = std::mem::take(&mut self.pending_text);
        let mut wont_echo_seen = false;

        let mut i = 0usize;
        while i < buf.len() {
            if buf[i] == TELNET_IAC {
                if !self.telnet_seen {
                    self.telnet_seen = true;
                    events.push(TelnetEvent::TelnetDetected);
                }
                if i + 1 >= buf.len() {
                    // Lone trailing IAC: retained, not dropped (finding 3).
                    break;
                }
                let cmd = buf[i + 1];
                match cmd {
                    TELNET_IAC => {
                        // Escaped 0xFF byte.
                        text.push(TELNET_IAC);
                        i += 2;
                    }
                    TELNET_WILL | TELNET_WONT | TELNET_DO | TELNET_DONT => {
                        if i + 2 >= buf.len() {
                            // IAC + verb with no option byte yet: retained.
                            break;
                        }
                        let option = buf[i + 2];
                        // RFC 1143 Q method (finding 4 / Job 9): a reply
                        // goes out, and OptionEnabled/OptionDisabled fires,
                        // only on an actual state change — see
                        // q_receive_will_wont/q_receive_do_dont.
                        match cmd {
                            TELNET_WILL if option == TELNET_OPT_ECHO => {
                                // Finding 7 / Job 10b: accept the peer's offer to echo for
                                // us (the standard password-prompt signal) instead of
                                // falling through to q_receive_will_wont's generic refusal.
                                // ECHO stays outside the Q method's support_him table by
                                // design (see EchoOff's doc comment) - answered
                                // unconditionally every time, exactly like the pre-existing
                                // WONT ECHO special case below.
                                wire.extend_from_slice(&[TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO]);
                                events.push(TelnetEvent::EchoOff);
                            }
                            TELNET_WILL => {
                                self.q_receive_will_wont(option, true, &mut wire, &mut events);
                            }
                            TELNET_WONT if option == TELNET_OPT_ECHO => {
                                // Mirrors process_telnet's wont_echo_seen
                                // exactly - fires every time, independent of
                                // whether a Prompt event also fires below
                                // (see WontEchoPromptHint's doc comment).
                                // ECHO is not tracked by the Q method here
                                // (see EchoOff's doc comment); this stays its
                                // own special case exactly as before. Job
                                // 10b adds the real DONT ECHO reply and
                                // EchoOn signal alongside the pre-existing
                                // prompt-boundary hint - process_telnet (the
                                // frozen characterization oracle) never sent
                                // either, so the differential test
                                // special-cases these fixtures the same way
                                // it does the MCCP2 activation fixture.
                                wire.extend_from_slice(&[TELNET_IAC, TELNET_DONT, TELNET_OPT_ECHO]);
                                wont_echo_seen = true;
                                events.push(TelnetEvent::WontEchoPromptHint);
                                events.push(TelnetEvent::EchoOn);
                            }
                            TELNET_WONT => {
                                self.q_receive_will_wont(option, false, &mut wire, &mut events);
                            }
                            TELNET_DO => {
                                let became_enabled =
                                    self.q_receive_do_dont(option, true, &mut wire, &mut events);
                                if option == TELNET_OPT_NAWS && became_enabled {
                                    events.push(TelnetEvent::NawsRequested);
                                }
                            }
                            TELNET_DONT => {
                                self.q_receive_do_dont(option, false, &mut wire, &mut events);
                            }
                            _ => {} // unreachable: the outer arm matches only these four
                        }
                        i += 3;
                    }
                    TELNET_SB => {
                        let sb_start = i; // index of the IAC that opens IAC SB
                        let payload_start = i + 2;
                        let mut k = payload_start;
                        let mut se_at = None;
                        while k + 1 < buf.len() {
                            if buf[k] == TELNET_IAC {
                                if buf[k + 1] == TELNET_SE {
                                    se_at = Some(k);
                                    break;
                                } else if buf[k + 1] == TELNET_IAC {
                                    k += 2; // escaped 0xFF within the payload
                                    continue;
                                }
                            }
                            k += 1;
                        }
                        match se_at {
                            Some(se) => {
                                let sb_data = &buf[payload_start..se];
                                let is_mccp2 =
                                    self.handle_subnegotiation(sb_data, &mut wire, &mut events);
                                let after_se = se + 2;
                                i = after_se;
                                if is_mccp2 {
                                    // Job 3: activate right here. Create the
                                    // decompressor, then decompress whatever
                                    // this call's buffer still holds after
                                    // `IAC SE` IN PLACE - splicing the
                                    // result back into `buf` and continuing
                                    // the SAME parse loop over it, rather
                                    // than stopping and handing a raw
                                    // compressed tail to a second,
                                    // caller-owned buffer (finding 7's
                                    // leftover bug).
                                    self.decomp = Some(flate2::Decompress::new(true));
                                    let compressed_tail = buf.split_off(after_se);
                                    let plaintext =
                                        self.run_decompressor(compressed_tail, &mut events);
                                    buf.extend_from_slice(&plaintext);
                                }
                            }
                            None => {
                                // Incomplete subnegotiation.
                                if buf.len() - sb_start > MAX_SUBNEG_BYTES {
                                    let option_desc = buf
                                        .get(payload_start)
                                        .map(|b| b.to_string())
                                        .unwrap_or_else(|| "?".to_string());
                                    events.push(TelnetEvent::ProtocolError(format!(
                                        "subnegotiation for option {option_desc} exceeded \
                                         {MAX_SUBNEG_BYTES} bytes without IAC SE; abandoned"
                                    )));
                                    i = buf.len(); // discard through here, resume scanning next call
                                } else {
                                    // Genuinely incomplete: retained, not dropped (finding 3).
                                    break;
                                }
                            }
                        }
                    }
                    TELNET_GA | TELNET_EOR => {
                        extract_prompt(&mut text, &mut out_text, &mut events, &mut self.prompt_open);
                        i += 2;
                    }
                    TELNET_NOP | TELNET_SE => {
                        i += 2; // skip NOP and stray SE
                    }
                    _ => {
                        i += 2; // other 2-byte commands
                    }
                }
            } else {
                let b = buf[i];
                text.push(b);
                if b == b'\n' {
                    self.prompt_open = false;
                }
                i += 1;
            }
        }

        // Whatever this call didn't consume stays for the next feed().
        self.inbuf = buf[i..].to_vec();

        // WONT ECHO's trailing-partial-line fallback, mirroring process_telnet
        // exactly (see `prompt_open`'s doc comment for why this needs to
        // survive across calls where process_telnet's single-call
        // `prompt.is_none()` did not).
        if wont_echo_seen && !self.prompt_open {
            extract_prompt(&mut text, &mut out_text, &mut events, &mut self.prompt_open);
        }

        // Job 14 (plan Phase 4): strip `!!SOUND(...)`/`!!MUSIC(...)` triggers
        // out of `text` and turn each into a TelnetEvent::MspTrigger, before
        // the hold-back decision below - see `extract_msp_triggers`'s doc
        // comment for why this runs on `text` (not `out_text`, which may
        // already hold GA/EOR/WONT-ECHO-extracted prompt bytes) and what
        // "malformed" means here. Gated on `cfg.msp_enabled`: off means a
        // `!!SOUND(...)` trigger is never even looked at, let alone stripped.
        let msp_incomplete_from = if self.cfg.msp_enabled {
            extract_msp_triggers(&mut text, &mut events)
        } else {
            None
        };

        // Final text hold-back decision on whatever's left in `text` — see
        // the struct doc comment's "Text hold-back" section. An in-progress
        // MSP trigger at the very tail (no closing ')' seen yet, or — T2.1 —
        // not even a whole marker yet) needs to survive a chunk boundary the
        // same way an incomplete ANSI CSI/UTF-8 sequence does, but real
        // triggers routinely exceed MAX_TEXT_HOLDBACK's 32 bytes (a filename,
        // a full U= URL) - so it gets its own, larger cap
        // (MAX_MSP_TRIGGER_HOLDBACK) instead of raising the general one for
        // every session.
        //
        // T2.1 also means `msp_incomplete_from` can now point at something
        // as short as a single trailing `!` (see `msp_holdback_start`'s
        // `min_prefix` doc comment), which the *ordinary* rule below must be
        // allowed to win over: holding only from `pos` (as this used to)
        // would hold back exactly that `!` and flush everything before it —
        // fine for a real in-progress marker, but wrong for an ordinary
        // short line that merely happens to end in `!` (`"Enter your name!"`
        // followed by `IAC GA` in the next `feed()` call must still produce
        // `Prompt(b"Enter your name!")`, not `Prompt(b"!")`). So: hold from
        // the ordinary tail start whenever the ordinary rule applies at all
        // (a non-empty, no-newline-since tail no longer than
        // MAX_TEXT_HOLDBACK); only when it doesn't does the MSP hold-back
        // get to hold from `pos` instead (capped at MAX_MSP_TRIGGER_HOLDBACK).
        // `extract_msp_triggers` only ever returns a position with no '\n' in
        // `[pos, text.len())`, so `pos` is always at or after the ordinary
        // tail start — this can only ever *widen* the hold versus the
        // ordinary rule alone, never narrow it.
        let last_newline = text.iter().rposition(|&b| b == b'\n');
        let ordinary_tail_len = match last_newline {
            Some(p) => text.len() - (p + 1),
            None => text.len(),
        };
        let hold_from = if ordinary_tail_len > 0 && ordinary_tail_len <= MAX_TEXT_HOLDBACK {
            Some(text.len() - ordinary_tail_len)
        } else {
            msp_incomplete_from
                .filter(|&pos| text.len() - pos <= MAX_MSP_TRIGGER_HOLDBACK)
        };
        if let Some(split_at) = hold_from {
            self.pending_text = text.split_off(split_at);
        }
        // Otherwise nothing is held: either there's no tail to hold (already
        // flushed everything below), or the tail is too long and both caps
        // say to give up and flush it anyway rather than hold forever.
        out_text.append(&mut text);

        TelnetOutcome { text: out_text, wire, events }
    }

    /// No more data is coming (EOF/disconnect): release whatever text was
    /// still held back rather than losing it. `inbuf`'s leftover, if any, is
    /// a genuinely incomplete telnet sequence — nothing useful to extract
    /// from it — and is left untouched, same as `pending_compressed` (any
    /// not-yet-decompressed MCCP2 bytes, moot once the connection is
    /// closing). Deliberately does NOT try to flush the decompressor with
    /// `FlushDecompress::Finish`: a connection can legitimately close
    /// mid-block, and there is nothing useful to do with a final partial
    /// decompressed line that `flush_eof` doesn't already do for plaintext
    /// via `pending_text`. Job 4's reader should not add one either.
    pub fn flush_eof(&mut self) -> TelnetOutcome {
        TelnetOutcome {
            text: std::mem::take(&mut self.pending_text),
            wire: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Whether `feed`'s text hold-back (see the struct doc comment's "Text
    /// hold-back" section) is currently sitting on an unflushed trailing
    /// partial line. `spawn_telnet_reader` (`src/telnet_reader.rs`, Job 4)
    /// polls this from its 150ms idle timer — the mandatory idle flush the
    /// plan's Phase 2 preamble calls out: a prompt on a server that never
    /// sends GA/EOR (the norm on MUSH/MOO) would otherwise sit in
    /// `pending_text` until more output happened to arrive, and never be
    /// shown. Exists so callers outside this module never need to reach into
    /// the private `pending_text` field directly.
    pub fn has_pending_text(&self) -> bool {
        !self.pending_text.is_empty()
    }

    /// Drain and return whatever prefix of `feed`'s text hold-back is safe to
    /// show after 150ms of no further reads (T2.2) — see the struct doc
    /// comment's "Idle-flush release" section for why this is a prefix and
    /// not the whole thing. Used by the idle-flush timer (see
    /// `has_pending_text`). Unlike `flush_eof`, the session is not closing:
    /// it keeps every other field (`inbuf`, `decomp`, `prompt_open`, ...)
    /// exactly as `feed` left it and continues parsing normally on the next
    /// `feed` call — including whatever of `pending_text` this call did not
    /// release, which stays right where it was.
    pub fn take_idle_flushable_text(&mut self) -> Vec<u8> {
        let release_len = idle_release_len(&self.pending_text);
        self.pending_text.drain(..release_len).collect()
    }

    /// Test-only: current length of `pending_compressed` (T1.1's bound test). Before the
    /// fix, an inflate error left the entire unconsumed buffer sitting here forever,
    /// growing on every later `feed()` call while no output was ever produced again; this
    /// proves it now stays 0 once the session is poisoned, not merely capped.
    #[cfg(test)]
    fn pending_compressed_len(&self) -> usize {
        self.pending_compressed.len()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_charset_will_do_negotiation() {
        // Server sends IAC WILL CHARSET → client should respond IAC DO CHARSET
        let data = [TELNET_IAC, TELNET_WILL, TELNET_OPT_CHARSET];
        let result = process_telnet(&data);
        assert!(result.telnet_detected);
        assert_eq!(result.responses, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_CHARSET]);
    }

    #[test]
    fn test_charset_request_parsing() {
        // IAC SB CHARSET REQUEST <space> UTF-8 <space> ISO-8859-1 IAC SE
        let mut data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_REQUEST, b' '];
        data.extend_from_slice(b"UTF-8");
        data.push(b' ');
        data.extend_from_slice(b"ISO-8859-1");
        data.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        let result = process_telnet(&data);
        assert!(result.telnet_detected);
        let charsets = result.charset_request.unwrap();
        assert_eq!(charsets, vec!["UTF-8", "ISO-8859-1"]);
    }

    #[test]
    fn test_charset_request_with_ttable() {
        // IAC SB CHARSET REQUEST <sep> TTABLE(1) <version> UTF-8 <sep> IBM437 IAC SE
        let sep = b';';
        let mut data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_REQUEST, sep];
        data.push(1); // TTABLE flag
        data.push(1); // TTABLE version
        data.extend_from_slice(b"UTF-8");
        data.push(sep);
        data.extend_from_slice(b"IBM437");
        data.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        let result = process_telnet(&data);
        let charsets = result.charset_request.unwrap();
        assert_eq!(charsets, vec!["UTF-8", "IBM437"]);
    }

    #[test]
    fn test_build_charset_accepted() {
        let msg = build_charset_accepted("UTF-8");
        assert_eq!(msg, vec![
            TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_ACCEPTED,
            b'U', b'T', b'F', b'-', b'8',
            TELNET_IAC, TELNET_SE,
        ]);
    }

    #[test]
    fn test_build_charset_rejected() {
        let msg = build_charset_rejected();
        assert_eq!(msg, vec![
            TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_REJECTED,
            TELNET_IAC, TELNET_SE,
        ]);
    }

    #[test]
    fn test_charset_ttable_is_rejected() {
        // Server sends TTABLE-IS → we should respond with TTABLE-REJECTED
        let mut data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_TTABLE_IS];
        data.extend_from_slice(b"some ttable data");
        data.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        let result = process_telnet(&data);
        assert!(result.responses.contains(&CHARSET_TTABLE_REJECTED));
        assert_eq!(result.responses, vec![
            TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_TTABLE_REJECTED,
            TELNET_IAC, TELNET_SE,
        ]);
    }

    #[test]
    fn test_mccp2_will_negotiation() {
        // Server sends IAC WILL MCCP2 → client should respond IAC DO MCCP2
        let data = [TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2];
        let result = process_telnet(&data);
        assert!(result.telnet_detected);
        assert_eq!(result.responses, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2]);
        assert!(!result.mccp2_activated);
    }

    #[test]
    fn test_mccp2_activation() {
        // Server sends IAC SB MCCP2 IAC SE followed by compressed data
        // Text "Hello" before the subneg, then compressed data after
        let mut data = Vec::new();
        data.extend_from_slice(b"Hello\n");
        data.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        // Append some compressed bytes (these would be zlib data)
        data.extend_from_slice(&[0x78, 0x9c]); // zlib header

        let result = process_telnet(&data);
        assert!(result.mccp2_activated);
        // mccp2_offset should point right after IAC SE
        let expected_offset = 6 + 5; // "Hello\n" (6) + IAC SB MCCP2 IAC SE (5)
        assert_eq!(result.mccp2_offset, expected_offset);
        // cleaned should contain the text before activation
        assert_eq!(result.cleaned, b"Hello\n");
    }

    #[test]
    fn test_mccp2_decompress() {
        // Create compressed data using flate2
        use flate2::{Compress, FlushCompress};
        let input = b"Hello, World! This is a test of MCCP2 compression.\n";
        let mut compressed = vec![0u8; 256];
        let mut compressor = Compress::new(flate2::Compression::default(), true);
        let status = compressor.compress(input, &mut compressed, FlushCompress::Finish).unwrap();
        assert_eq!(status, flate2::Status::StreamEnd);
        let compressed_len = compressor.total_out() as usize;
        compressed.truncate(compressed_len);

        // Decompress
        let mut decomp = flate2::Decompress::new(true);
        let output = mccp2_decompress(&mut decomp, &compressed);
        assert_eq!(output, input);
    }

    // ==================================================================
    // Step 0.2 — outbound IAC escaping (finding 6)
    // ==================================================================

    #[test]
    fn test_push_escaped_doubles_iac_only() {
        // The core of every builder's payload escaping. Every 0xFF is doubled;
        // every other byte (including other "high" bytes like 0xFE) passes through
        // untouched, exactly once.
        let mut out = Vec::new();
        push_escaped(&mut out, &[]);
        assert_eq!(out, Vec::<u8>::new(), "empty input");

        let mut out = Vec::new();
        push_escaped(&mut out, &[0x41, 0x42]);
        assert_eq!(out, vec![0x41, 0x42], "plain bytes untouched");

        let mut out = Vec::new();
        push_escaped(&mut out, &[TELNET_IAC]);
        assert_eq!(out, vec![TELNET_IAC, TELNET_IAC], "lone IAC doubled");

        let mut out = Vec::new();
        push_escaped(&mut out, &[0x41, TELNET_IAC, 0x42]);
        assert_eq!(out, vec![0x41, TELNET_IAC, TELNET_IAC, 0x42], "IAC in the middle doubled once");

        let mut out = Vec::new();
        push_escaped(&mut out, &[TELNET_IAC, TELNET_IAC, 0x00, 0xFE]);
        assert_eq!(
            out,
            vec![TELNET_IAC, TELNET_IAC, TELNET_IAC, TELNET_IAC, 0x00, 0xFE],
            "consecutive IAC bytes each doubled; 0xFE is not IAC and is left alone"
        );
    }

    #[test]
    fn test_build_naws_subnegotiation_escapes_0xff() {
        // NAWS is the one builder that already escaped 0xFF before this fix, and it's
        // also the only builder whose payload (raw width/height bytes) can genuinely
        // contain 0xFF through the public API (a u16 low byte of 255), so this is the
        // real end-to-end proof that the shared push_escaped preserves that behaviour.
        let msg = build_naws_subnegotiation(0x00FF, 0x0102);
        assert_eq!(
            msg,
            vec![
                TELNET_IAC, TELNET_SB, TELNET_OPT_NAWS,
                0x00, TELNET_IAC, TELNET_IAC, // width 0x00FF: low byte 0xFF doubled
                0x01, 0x02,                    // height 0x0102: no 0xFF byte
                TELNET_IAC, TELNET_SE,
            ]
        );

        // High byte of 255 (e.g. width 0xFF00) must also be doubled.
        let msg = build_naws_subnegotiation(0xFF00, 0);
        assert_eq!(
            msg,
            vec![
                TELNET_IAC, TELNET_SB, TELNET_OPT_NAWS,
                TELNET_IAC, TELNET_IAC, 0x00, // width 0xFF00: high byte 0xFF doubled
                0x00, 0x00,                    // height 0
                TELNET_IAC, TELNET_SE,
            ]
        );
    }

    // NOTE on the remaining builders: their payload parameters are `&str`, and a raw
    // 0xFF byte can never appear in `str::as_bytes()` — it is not a valid UTF-8 byte
    // in any encoded codepoint (UTF-8 lead/continuation bytes only ever use 0x00-0xF4
    // and 0x80-0xBF). So "pass an argument containing 0xFF" is impossible to construct
    // through the safe public API for these builders; the IAC-doubling guarantee for
    // them is pinned once, at the byte level, by test_push_escaped_doubles_iac_only
    // above (every one of these builders now calls that same function on its payload -
    // see the source). What these tests pin instead is that (a) the framing is exactly
    // as before, and (b) arbitrary multi-byte UTF-8 payload content — including bytes
    // >= 0x80 that are NOT 0xFF — passes through byte-for-byte, neither mangled nor
    // spuriously doubled.

    #[test]
    fn test_build_ttype_response_framing_and_payload() {
        let msg = build_ttype_response("xterm-256café");
        let mut expected = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_TTYPE, TTYPE_IS];
        expected.extend_from_slice("xterm-256café".as_bytes());
        expected.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        assert_eq!(msg, expected);
    }

    #[test]
    fn test_build_gmcp_message_framing_and_payload() {
        let msg = build_gmcp_message("Core.Hello", "{\"café\":1}");
        let mut expected = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP];
        expected.extend_from_slice(b"Core.Hello");
        expected.push(b' '); // separator - not escaped
        expected.extend_from_slice("{\"café\":1}".as_bytes());
        expected.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        assert_eq!(msg, expected);

        // No message: no separator byte should be emitted at all.
        let msg = build_gmcp_message("Core.Ping", "");
        assert_eq!(msg, vec![
            TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP,
            b'C', b'o', b'r', b'e', b'.', b'P', b'i', b'n', b'g',
            TELNET_IAC, TELNET_SE,
        ]);
    }

    #[test]
    fn test_build_msdp_request_framing_and_payload() {
        let msg = build_msdp_request("répertoire");
        let mut expected = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
        expected.extend_from_slice("répertoire".as_bytes());
        expected.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        assert_eq!(msg, expected);
    }

    #[test]
    fn test_build_msdp_set_framing_and_payload() {
        let msg = build_msdp_set("NAME", "Frödo");
        let mut expected = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
        expected.extend_from_slice(b"NAME");
        expected.push(MSDP_VAL); // marker - not escaped
        expected.extend_from_slice("Frödo".as_bytes());
        expected.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        assert_eq!(msg, expected);
    }

    #[test]
    fn test_build_charset_accepted_framing_and_payload() {
        let msg = build_charset_accepted("ISO-8859-1");
        assert_eq!(msg, vec![
            TELNET_IAC, TELNET_SB, TELNET_OPT_CHARSET, CHARSET_ACCEPTED,
            b'I', b'S', b'O', b'-', b'8', b'8', b'5', b'9', b'-', b'1',
            TELNET_IAC, TELNET_SE,
        ]);
    }

    // ==================================================================
    // Step 0.1 — characterization tests for process_telnet (the oracle table)
    //
    // These ~25 cases pin what process_telnet does TODAY, including behaviour the
    // plan (investigate-differences-between-tinyfuge-fluffy-stallman.md) calls a
    // bug. Job 2 must re-run this identical table against the new TelnetSession;
    // any difference is either a transcription mistake or a deliberate, reviewed
    // behavioural change - never silent drift.
    // ==================================================================

    type CheckFn = fn(&str, &TelnetResult);

    fn characterization_cases() -> Vec<(&'static str, Vec<u8>, CheckFn)> {
        vec![
            // --- WILL <option> (server offers, client answers) ---
            ("will_sga_accepted", vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_SGA], |name, r| {
                assert!(r.telnet_detected, "case {name}: telnet_detected");
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_SGA], "case {name}: responses");
            }),
            ("will_eor_accepted", vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_EOR], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_EOR], "case {name}: responses");
            }),
            ("will_gmcp_accepted", vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_GMCP], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_GMCP], "case {name}: responses");
                assert!(r.gmcp_negotiated, "case {name}: gmcp_negotiated");
            }),
            ("will_msdp_accepted", vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_MSDP], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MSDP], "case {name}: responses");
                assert!(r.msdp_negotiated, "case {name}: msdp_negotiated");
            }),
            ("will_mccp2_accepted", vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2], "case {name}: responses");
                assert!(!r.mccp2_activated, "case {name}: mccp2_activated (WILL alone must not activate)");
            }),
            ("will_charset_accepted", vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_CHARSET], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_CHARSET], "case {name}: responses");
            }),
            ("will_unknown_option_refused", vec![TELNET_IAC, TELNET_WILL, 100], |name, r| {
                // Finding 4: every WILL is answered unconditionally with no option state
                // machine; an option Clay doesn't understand is simply refused with DONT.
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_DONT, 100], "case {name}: responses");
            }),
            // --- DO <option> (server asks us to enable, client answers) ---
            ("do_naws_accepted", vec![TELNET_IAC, TELNET_DO, TELNET_OPT_NAWS], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_NAWS], "case {name}: responses");
                assert!(r.naws_requested, "case {name}: naws_requested");
            }),
            ("do_ttype_accepted", vec![TELNET_IAC, TELNET_DO, TELNET_OPT_TTYPE], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_TTYPE], "case {name}: responses");
            }),
            ("do_eor_accepted", vec![TELNET_IAC, TELNET_DO, TELNET_OPT_EOR], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_EOR], "case {name}: responses");
            }),
            ("do_unknown_option_refused", vec![TELNET_IAC, TELNET_DO, 101], |name, r| {
                assert_eq!(r.responses, vec![TELNET_IAC, TELNET_WONT, 101], "case {name}: responses");
            }),
            // --- SB TTYPE SEND ---
            ("sb_ttype_send_sets_flag", vec![
                TELNET_IAC, TELNET_SB, TELNET_OPT_TTYPE, TTYPE_SEND, TELNET_IAC, TELNET_SE,
            ], |name, r| {
                assert!(r.ttype_requested, "case {name}: ttype_requested");
                assert!(r.cleaned.is_empty(), "case {name}: cleaned");
                assert!(r.responses.is_empty(), "case {name}: responses (SB TTYPE SEND gets no immediate reply)");
            }),
            // --- SB GMCP ---
            ("sb_gmcp_with_space_splits_package_and_json", {
                let mut d = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP];
                d.extend_from_slice(b"Core.Hello {\"val\":1}");
                d.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
                d
            }, |name, r| {
                assert_eq!(
                    r.gmcp_data,
                    vec![("Core.Hello".to_string(), "{\"val\":1}".to_string())],
                    "case {name}: gmcp_data"
                );
            }),
            ("sb_gmcp_no_space_is_package_only", {
                let mut d = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP];
                d.extend_from_slice(b"Core.Ping");
                d.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
                d
            }, |name, r| {
                assert_eq!(
                    r.gmcp_data,
                    vec![("Core.Ping".to_string(), String::new())],
                    "case {name}: gmcp_data"
                );
            }),
            // --- SB MSDP (exercises parse_msdp_pairs / parse_msdp_value) ---
            ("sb_msdp_flat_var_val", {
                let mut d = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
                d.extend_from_slice(b"PLAYER");
                d.push(MSDP_VAL);
                d.extend_from_slice(b"Frodo");
                d.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
                d
            }, |name, r| {
                assert_eq!(
                    r.msdp_data,
                    vec![("PLAYER".to_string(), "\"Frodo\"".to_string())],
                    "case {name}: msdp_data"
                );
            }),
            ("sb_msdp_nested_table_with_array", {
                let mut d = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
                d.extend_from_slice(b"ROOM");
                d.push(MSDP_VAL);
                d.push(MSDP_TABLE_OPEN);
                d.push(MSDP_VAR);
                d.extend_from_slice(b"NAME");
                d.push(MSDP_VAL);
                d.extend_from_slice(b"Temple");
                d.push(MSDP_VAR);
                d.extend_from_slice(b"EXITS");
                d.push(MSDP_VAL);
                d.push(MSDP_ARRAY_OPEN);
                d.push(MSDP_VAL);
                d.extend_from_slice(b"n");
                d.push(MSDP_VAL);
                d.extend_from_slice(b"s");
                d.push(MSDP_ARRAY_CLOSE);
                d.push(MSDP_TABLE_CLOSE);
                d.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
                d
            }, |name, r| {
                assert_eq!(
                    r.msdp_data,
                    vec![(
                        "ROOM".to_string(),
                        "{\"NAME\":\"Temple\",\"EXITS\":[\"n\",\"s\"]}".to_string()
                    )],
                    "case {name}: msdp_data (nested TABLE containing an ARRAY)"
                );
            }),
            // --- IAC GA / IAC EOR prompt extraction ---
            // The prompt is always computed from the CURRENT call's `cleaned` text since
            // the last newline (or from the start if there is none in this chunk) - there
            // is no persisted "prompt so far" state across calls.
            ("ga_extracts_prompt_since_last_newline", {
                let mut d = Vec::new();
                d.extend_from_slice(b"Line one\nHave a nice day> ");
                d.extend_from_slice(&[TELNET_IAC, TELNET_GA]);
                d
            }, |name, r| {
                assert_eq!(r.cleaned, b"Line one\n", "case {name}: cleaned (prompt drained out)");
                assert_eq!(r.prompt, Some(b"Have a nice day> ".to_vec()), "case {name}: prompt");
            }),
            ("eor_extracts_whole_chunk_when_no_newline", {
                let mut d = Vec::new();
                d.extend_from_slice(b"just a prompt> ");
                d.extend_from_slice(&[TELNET_IAC, TELNET_EOR]);
                d
            }, |name, r| {
                assert_eq!(r.cleaned, b"", "case {name}: cleaned");
                assert_eq!(r.prompt, Some(b"just a prompt> ".to_vec()), "case {name}: prompt");
            }),
            // --- wont_echo_seen trailing-prompt rule (telnet.rs:341-349) ---
            ("wont_echo_extracts_trailing_partial_line", {
                let mut d = Vec::new();
                d.extend_from_slice(b"Login: ");
                d.extend_from_slice(&[TELNET_IAC, TELNET_WONT, TELNET_OPT_ECHO]);
                d
            }, |name, r| {
                assert!(r.wont_echo_seen, "case {name}: wont_echo_seen");
                assert_eq!(r.cleaned, b"", "case {name}: cleaned (prompt drained out)");
                assert_eq!(r.prompt, Some(b"Login: ".to_vec()), "case {name}: prompt");
            }),
            ("wont_echo_no_extraction_when_line_already_terminated", {
                let mut d = Vec::new();
                d.extend_from_slice(b"Some log line\n");
                d.extend_from_slice(&[TELNET_IAC, TELNET_WONT, TELNET_OPT_ECHO]);
                d
            }, |name, r| {
                assert!(r.wont_echo_seen, "case {name}: wont_echo_seen");
                assert_eq!(r.prompt, None, "case {name}: prompt (nothing to extract after trailing newline)");
                assert_eq!(r.cleaned, b"Some log line\n", "case {name}: cleaned unchanged");
            }),
            ("wont_echo_does_not_override_earlier_ga_prompt", {
                let mut d = Vec::new();
                d.extend_from_slice(b"Room\n> ");
                d.extend_from_slice(&[TELNET_IAC, TELNET_GA]);
                d.extend_from_slice(b"trailing");
                d.extend_from_slice(&[TELNET_IAC, TELNET_WONT, TELNET_OPT_ECHO]);
                d
            }, |name, r| {
                assert!(r.wont_echo_seen, "case {name}: wont_echo_seen");
                // prompt.is_none() guards the wont_echo extraction, so the GA-set prompt
                // from earlier in the same buffer is left standing.
                assert_eq!(r.prompt, Some(b"> ".to_vec()), "case {name}: prompt (from the GA, not overwritten)");
                assert_eq!(r.cleaned, b"Room\ntrailing", "case {name}: cleaned");
            }),
            // --- IAC IAC unescaping in the plain stream ---
            ("iac_iac_unescapes_to_single_0xff", {
                let mut d = Vec::new();
                d.extend_from_slice(b"foo");
                d.extend_from_slice(&[TELNET_IAC, TELNET_IAC]);
                d.extend_from_slice(b"bar");
                d
            }, |name, r| {
                let mut expected = b"foo".to_vec();
                expected.push(0xFF);
                expected.extend_from_slice(b"bar");
                assert_eq!(r.cleaned, expected, "case {name}: cleaned");
                assert!(r.telnet_detected, "case {name}: telnet_detected");
            }),
            // --- IAC NOP and stray IAC SE are both skipped with no output ---
            ("iac_nop_is_skipped", {
                let mut d = Vec::new();
                d.extend_from_slice(b"foo");
                d.extend_from_slice(&[TELNET_IAC, TELNET_NOP]);
                d.extend_from_slice(b"bar");
                d
            }, |name, r| {
                assert_eq!(r.cleaned, b"foobar", "case {name}: cleaned");
            }),
            ("stray_iac_se_is_skipped", {
                let mut d = Vec::new();
                d.extend_from_slice(b"foo");
                d.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
                d.extend_from_slice(b"bar");
                d
            }, |name, r| {
                assert_eq!(r.cleaned, b"foobar", "case {name}: cleaned");
            }),
            // --- MCCP2 activation splits cleaned/mccp2_offset at IAC SB 86 IAC SE ---
            // T1.2 (plan Job 1, step 9): TelnetSession now requires a negotiated
            // options_him[MCCP2] == Yes before activating, so this fixture leads with
            // IAC WILL MCCP2 - process_telnet (the frozen oracle) has no such gate and
            // always answers a WILL MCCP2 with IAC DO MCCP2 unconditionally, so its own
            // `responses`/`mccp2_offset` shift to account for those extra 3 bytes too.
            ("mccp2_activation_splits_at_iac_sb_se", {
                let mut d = Vec::new();
                d.extend_from_slice(b"Welcome\n");
                d.extend_from_slice(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]);
                d.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
                d.extend_from_slice(&[0x78, 0x9c]); // stand-in compressed bytes, never parsed as telnet
                d
            }, |name, r| {
                assert!(r.mccp2_activated, "case {name}: mccp2_activated");
                assert_eq!(r.mccp2_offset, "Welcome\n".len() + 3 + 5, "case {name}: mccp2_offset");
                assert_eq!(r.cleaned, b"Welcome\n", "case {name}: cleaned");
                assert_eq!(
                    r.responses,
                    vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2],
                    "case {name}: responses (process_telnet answers WILL MCCP2 unconditionally)"
                );
            }),
        ]
    }

    #[test]
    fn test_process_telnet_characterization_table() {
        let cases = characterization_cases();
        assert_eq!(cases.len(), 25, "expected exactly 25 characterization cases - update this count if you add/remove one");
        for (name, input, check) in cases {
            let result = process_telnet(&input);
            check(name, &result);
        }
    }

    // ==================================================================
    // Step 0.1 — first-ever tests for find_safe_split_point, including the two
    // known holes (finding 3). These are deliberately NOT fixed here; Job 2's
    // real carry-over buffer replaces this function's role entirely, and pinning
    // the bug now makes that a visible, intentional behavioural change later.
    // ==================================================================

    #[test]
    fn test_find_safe_split_point_complete_buffer_returns_len() {
        // A fully-terminated IAC DO NAWS sequence plus trailing plain text: nothing
        // is incomplete, so everything is safe to send.
        let mut data = Vec::new();
        data.extend_from_slice(b"hi ");
        data.extend_from_slice(&[TELNET_IAC, TELNET_DO, TELNET_OPT_NAWS]);
        data.extend_from_slice(b" there");
        assert_eq!(find_safe_split_point(&data), data.len());
    }

    #[test]
    fn test_find_safe_split_point_trailing_lone_iac_returns_zero() {
        // A buffer that is just a dangling IAC: nothing at all is safe to send yet.
        let data = [TELNET_IAC];
        assert_eq!(find_safe_split_point(&data), 0);
    }

    #[test]
    fn test_find_safe_split_point_incomplete_sb_in_the_middle_returns_its_index() {
        let mut data = Vec::new();
        data.extend_from_slice(b"AB");
        let sb_at = data.len();
        data.extend_from_slice(&[TELNET_IAC, TELNET_SB]);
        data.extend_from_slice(b"restofpayload"); // no IAC SE anywhere - incomplete
        assert_eq!(find_safe_split_point(&data), sb_at);
    }

    #[test]
    fn test_find_safe_split_point_incomplete_sb_at_index_zero_returns_len_bug() {
        // Finding 3 / structural finding 3 (find_safe_split_point scans backwards for
        // IAC SB with `while j > 0`, so it never examines index 0). An incomplete
        // subnegotiation that starts at the very front of the buffer is invisible to
        // this scan and falls through to "everything is safe" - the exact opposite of
        // correct. This is a real reachable hole (a GMCP/MSDP subnegotiation split
        // across two TCP reads, arriving as the first bytes of a read), pinned here as
        // current (wrong) behaviour so Job 2's real carry-over buffer shows up as a
        // deliberate fix rather than silent drift.
        let data = [TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP, b'H', b'i'];
        assert_eq!(find_safe_split_point(&data), data.len(), "bug: should be 0, is len");
    }

    // ==================================================================
    // Step 1.1 / 1.2 — TelnetSession: differential tests against
    // process_telnet, and split-invariance across every offset.
    //
    // Both groups below read `characterization_cases()` (Job 1's fixture
    // table) directly rather than duplicating it, per the plan.
    // ==================================================================

    /// Run a fixture through a fresh session with a single `feed`, with no
    /// `flush_eof` — this is the "one-shot" baseline the split-invariance
    /// test compares chunked feeding against.
    fn run_one_shot(input: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<TelnetEvent>) {
        let mut session = TelnetSession::new(TelnetConfig::default());
        let o = session.feed(input);
        (o.text, o.wire, o.events)
    }

    /// Compare a TelnetSession's emitted events against process_telnet's
    /// output fields for the same input. Job 4.5 added `OptionEnabled`/
    /// `WontEchoPromptHint`, so `gmcp_negotiated`/`msdp_negotiated`/
    /// `wont_echo_seen` are now checked here too (previously they weren't:
    /// Phase 1's `TelnetEvent` had no variant for "an option was just turned
    /// on" - the WILL/DO reply itself was already pinned via the `wire`
    /// comparison the caller does alongside this, but that doesn't prove the
    /// event-based migration path Jobs 5-7 need actually carries the same
    /// information).
    fn assert_events_equivalent(name: &str, events: &[TelnetEvent], expected: &TelnetResult) {
        assert_eq!(
            events.contains(&TelnetEvent::TelnetDetected),
            expected.telnet_detected,
            "case {name}: TelnetDetected vs telnet_detected"
        );
        assert_eq!(
            events.contains(&TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)),
            expected.gmcp_negotiated,
            "case {name}: OptionEnabled(GMCP) vs gmcp_negotiated"
        );
        assert_eq!(
            events.contains(&TelnetEvent::OptionEnabled(TELNET_OPT_MSDP)),
            expected.msdp_negotiated,
            "case {name}: OptionEnabled(MSDP) vs msdp_negotiated"
        );
        assert_eq!(
            events.contains(&TelnetEvent::WontEchoPromptHint),
            expected.wont_echo_seen,
            "case {name}: WontEchoPromptHint vs wont_echo_seen"
        );
        match &expected.prompt {
            Some(p) => assert!(
                events.contains(&TelnetEvent::Prompt(p.clone())),
                "case {name}: expected Prompt({:?}), got {:?}",
                String::from_utf8_lossy(p),
                events
            ),
            None => assert!(
                !events.iter().any(|e| matches!(e, TelnetEvent::Prompt(_))),
                "case {name}: unexpected Prompt event in {:?}",
                events
            ),
        }
        assert_eq!(
            events.contains(&TelnetEvent::NawsRequested),
            expected.naws_requested,
            "case {name}: NawsRequested"
        );
        assert_eq!(
            events.contains(&TelnetEvent::TtypeRequested),
            expected.ttype_requested,
            "case {name}: TtypeRequested"
        );
        let gmcp_events: Vec<(String, String)> = events
            .iter()
            .filter_map(|e| match e {
                TelnetEvent::GmcpMessage(p, j) => Some((p.clone(), j.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(gmcp_events, expected.gmcp_data, "case {name}: gmcp events");
        let msdp_events: Vec<(String, String)> = events
            .iter()
            .filter_map(|e| match e {
                TelnetEvent::MsdpVariable(v, val) => Some((v.clone(), val.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(msdp_events, expected.msdp_data, "case {name}: msdp events");
        match &expected.charset_request {
            Some(list) => assert!(
                events.contains(&TelnetEvent::CharsetRequest(list.clone())),
                "case {name}: expected CharsetRequest({list:?})"
            ),
            None => assert!(
                !events.iter().any(|e| matches!(e, TelnetEvent::CharsetRequest(_))),
                "case {name}: unexpected CharsetRequest event"
            ),
        }
    }

    #[test]
    fn test_telnet_session_differential_against_process_telnet() {
        for (name, input, _check) in characterization_cases() {
            let expected = process_telnet(&input);
            let mut session = TelnetSession::new(TelnetConfig::default());
            let mut outcome = session.feed(&input);
            let eof = session.flush_eof();
            outcome.text.extend_from_slice(&eof.text);
            outcome.wire.extend_from_slice(&eof.wire);
            outcome.events.extend(eof.events);

            if name == "mccp2_activation_splits_at_iac_sb_se" {
                // Deliberate divergence (Job 3's scope, not a bug fix): the
                // session signals activation via an event rather than a
                // boolean flag/offset into the original buffer. As of Job 3
                // it also genuinely attempts to decompress this fixture's
                // trailing bytes ([0x78, 0x9c] - a valid but data-less zlib
                // header) in place instead of merely stashing them; that
                // header alone decompresses to zero bytes of output and is
                // fully consumed, leaving the decompressor live and waiting
                // for more input rather than producing any observable
                // difference here - see the dedicated mccp2_* tests below
                // for fixtures with a real, complete compressed payload.
                // Text and wire before the activation point still match
                // exactly.
                assert_eq!(outcome.text, expected.cleaned, "case {name}: text");
                assert_eq!(
                    outcome.wire,
                    vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2],
                    "case {name}: wire (the fixture's WILL MCCP2 is answered DO; the bare SB itself gets no reply)"
                );
                assert!(
                    outcome.events.contains(&TelnetEvent::CompressionStarted),
                    "case {name}: expected CompressionStarted, got {:?}",
                    outcome.events
                );
                continue;
            }

            if name.starts_with("wont_echo_") {
                // Deliberate divergence (Job 10b's scope, not a bug in either side): Job
                // 10b makes TelnetSession actually answer `IAC WONT ECHO` with `IAC DONT
                // ECHO` and emit `EchoOn`, finding 7's fix. `process_telnet` (the frozen
                // characterization oracle, `mod tests`-only since Job 8) never sent a
                // reply here at all (see the `TELNET_WONT if option == TELNET_OPT_ECHO`
                // arm near the top of this file) and predates `EchoOn` entirely, so
                // `expected.responses`/`expected` have nothing to compare either against.
                // Text and the WontEchoPromptHint/prompt-extraction behaviour these
                // fixtures actually exist to pin are unaffected and still checked exactly.
                assert_eq!(outcome.text, expected.cleaned, "case {name}: text");
                assert_eq!(
                    outcome.wire,
                    vec![TELNET_IAC, TELNET_DONT, TELNET_OPT_ECHO],
                    "case {name}: wire (session now answers WONT ECHO; process_telnet never did)"
                );
                assert!(
                    outcome.events.contains(&TelnetEvent::EchoOn),
                    "case {name}: expected EchoOn, got {:?}",
                    outcome.events
                );
                assert_events_equivalent(name, &outcome.events, &expected);
                continue;
            }

            if name == "sb_ttype_send_sets_flag" {
                // Deliberate divergence (Job 12's scope, not a bug in either side):
                // as of Job 12 (plan Phase 4, 4.1 - MTTS cycling) the session answers
                // a TTYPE SEND immediately, in `wire`, with the first MTTS cycle
                // position (the uppercased client name). `process_telnet` (the frozen
                // characterization oracle) never sent a reply here at all (see
                // `sb_ttype_send_sets_flag`'s own characterization case, "SB TTYPE
                // SEND gets no immediate reply") and predates MTTS entirely, so
                // `expected.responses` has nothing to compare against. Text and the
                // TtypeRequested event this fixture actually exists to pin are
                // unaffected and still checked exactly.
                assert_eq!(outcome.text, expected.cleaned, "case {name}: text");
                assert_eq!(
                    outcome.wire,
                    build_ttype_response(&TelnetConfig::default().client_name.to_uppercase()),
                    "case {name}: wire (session now answers TTYPE SEND with MTTS cycle position 1; \
                     process_telnet never did)"
                );
                assert_events_equivalent(name, &outcome.events, &expected);
                continue;
            }

            assert_eq!(outcome.text, expected.cleaned, "case {name}: text");
            assert_eq!(outcome.wire, expected.responses, "case {name}: wire");
            assert_events_equivalent(name, &outcome.events, &expected);
        }
    }

    #[test]
    fn test_telnet_session_split_invariance() {
        // Highest-value test in the plan: for every fixture, for every split
        // point, two-chunk feeding must equal one-shot feeding byte for byte
        // (text, wire, and events). Unlike the differential test above, this
        // compares TelnetSession against itself - no flush_eof on either
        // side - so it is a pure self-consistency property.
        let mut combinations = 0usize;
        for (name, input, _check) in characterization_cases() {
            let (base_text, base_wire, base_events) = run_one_shot(&input);
            for k in 0..=input.len() {
                combinations += 1;
                let mut session = TelnetSession::new(TelnetConfig::default());
                let mut o1 = session.feed(&input[..k]);
                let o2 = session.feed(&input[k..]);
                o1.text.extend_from_slice(&o2.text);
                o1.wire.extend_from_slice(&o2.wire);
                o1.events.extend(o2.events);
                assert_eq!(o1.text, base_text, "case {name} split at {k}: text");
                assert_eq!(o1.wire, base_wire, "case {name} split at {k}: wire");
                assert_eq!(o1.events, base_events, "case {name} split at {k}: events");
            }
        }
        // Sanity check that this exercised a real number of fixture/offset
        // combinations rather than silently iterating over nothing (25
        // fixtures times each fixture's length+1 split points; currently 290).
        assert!(
            combinations > 250,
            "expected many fixture/offset combinations, got {combinations}"
        );
    }

    #[test]
    fn test_telnet_session_three_chunk_variant_nested_msdp_and_mccp2() {
        // "Add a three-chunk variant for the nested-MSDP and MCCP2-activation
        // fixtures" - these are the two fixtures with the deepest internal
        // structure (a TABLE containing an ARRAY; a subnegotiation followed
        // by non-telnet bytes), so a two-way split is less likely to catch a
        // three-way carry-over bug than these specifically chosen ones.
        for name in [
            "sb_msdp_nested_table_with_array",
            "mccp2_activation_splits_at_iac_sb_se",
        ] {
            let (_, input, _) = characterization_cases()
                .into_iter()
                .find(|(n, _, _)| *n == name)
                .unwrap_or_else(|| panic!("fixture {name} not found"));
            let (base_text, base_wire, base_events) = run_one_shot(&input);
            let len = input.len();
            for a in 0..=len {
                for b in a..=len {
                    let mut session = TelnetSession::new(TelnetConfig::default());
                    let mut o1 = session.feed(&input[..a]);
                    let o2 = session.feed(&input[a..b]);
                    let o3 = session.feed(&input[b..]);
                    o1.text.extend_from_slice(&o2.text);
                    o1.text.extend_from_slice(&o3.text);
                    o1.wire.extend_from_slice(&o2.wire);
                    o1.wire.extend_from_slice(&o3.wire);
                    o1.events.extend(o2.events);
                    o1.events.extend(o3.events);
                    assert_eq!(o1.text, base_text, "case {name} 3-split ({a},{b}): text");
                    assert_eq!(o1.wire, base_wire, "case {name} 3-split ({a},{b}): wire");
                    assert_eq!(o1.events, base_events, "case {name} 3-split ({a},{b}): events");
                }
            }
        }
    }

    #[test]
    fn test_telnet_session_iac_sb_at_offset_zero_split_across_two_feeds() {
        // The exact case find_safe_split_point gets wrong today (pinned above
        // in test_find_safe_split_point_incomplete_sb_at_index_zero_returns_len_bug):
        // an incomplete IAC SB starting at buffer offset 0. A session must
        // retain it (no index-0 special case) and correctly complete it once
        // the rest arrives in a later feed().
        let mut session = TelnetSession::new(TelnetConfig::default());
        let first = session.feed(&[TELNET_IAC, TELNET_SB]);
        assert!(first.text.is_empty(), "nothing to show yet");
        assert!(first.wire.is_empty());
        assert_eq!(first.events, vec![TelnetEvent::TelnetDetected]);

        let mut second_input = vec![TELNET_OPT_GMCP];
        second_input.extend_from_slice(b"Foo");
        second_input.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        let second = session.feed(&second_input);
        assert!(second.text.is_empty());
        assert!(second.wire.is_empty());
        assert_eq!(
            second.events,
            vec![TelnetEvent::GmcpMessage("Foo".to_string(), String::new())]
        );
    }

    #[test]
    fn test_telnet_session_oversized_subnegotiation_protocol_error_and_recovery() {
        // Rule 3: a retained IAC SB whose accumulated length exceeds
        // MAX_SUBNEG_BYTES is abandoned with a ProtocolError naming the
        // option, and scanning resumes - a server that never sends IAC SE
        // must not stall the world or grow inbuf without bound.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let mut data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP];
        data.extend(std::iter::repeat_n(b'x', MAX_SUBNEG_BYTES + 100)); // never terminated
        let outcome = session.feed(&data);
        assert!(outcome.text.is_empty());
        assert!(outcome.wire.is_empty());
        // TelnetDetected (first IAC ever seen) plus the ProtocolError.
        assert_eq!(outcome.events.len(), 2, "expected exactly two events, got {:?}", outcome.events);
        assert_eq!(outcome.events[0], TelnetEvent::TelnetDetected);
        match &outcome.events[1] {
            TelnetEvent::ProtocolError(msg) => {
                assert!(
                    msg.contains(&TELNET_OPT_MSDP.to_string()),
                    "error should name the option: {msg}"
                );
            }
            other => panic!("expected ProtocolError, got {other:?}"),
        }

        // Recovery: the session must still parse ordinary data correctly
        // afterwards, proving the abandoned bytes didn't wedge the parser.
        let recovered = session.feed(b"hello world\n");
        assert_eq!(recovered.text, b"hello world\n");
        assert!(recovered.events.is_empty());
    }

    #[test]
    fn test_telnet_session_gmcp_with_embedded_esc_split_mid_payload() {
        // A GMCP payload containing a literal 0x1B byte, arriving in two
        // chunks with the split landing inside the payload (right after the
        // ESC byte itself) - this must not be mistaken for an incomplete
        // ANSI CSI sequence in the raw stream (find_safe_split_point's
        // defect per finding 3) because the hold-back check only ever sees
        // *emitted text*, and subnegotiation payload bytes never become
        // emitted text.
        let mut data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP];
        data.extend_from_slice(b"Pkg.Msg ");
        data.push(b'a');
        data.push(0x1B); // literal ESC inside the payload
        data.push(b'b');
        data.extend_from_slice(&[TELNET_IAC, TELNET_SE]);

        // Split right after the embedded ESC byte.
        let split_at = data
            .iter()
            .position(|&b| b == 0x1B)
            .map(|p| p + 1)
            .unwrap();

        let mut session = TelnetSession::new(TelnetConfig::default());
        let mut outcome = session.feed(&data[..split_at]);
        let second = session.feed(&data[split_at..]);
        outcome.text.extend_from_slice(&second.text);
        outcome.events.extend(second.events);

        assert!(outcome.text.is_empty(), "no plain text at all in this fixture");
        assert_eq!(
            outcome.events,
            vec![
                TelnetEvent::TelnetDetected,
                TelnetEvent::GmcpMessage("Pkg.Msg".to_string(), "a\u{1b}b".to_string()),
            ]
        );
    }

    // ==================================================================
    // Step 3.1 (Job 9) — inbound IAC un-doubling (finding 6's "real half").
    //
    // `unescape_iac` itself is exercised directly first, then once per
    // consumer (MSDP/GMCP/CHARSET) with a payload shaped exactly like what
    // `handle_subnegotiation` slices for that consumer.
    //
    // MSDP's consumer (`parse_msdp_pairs`) uses `String::from_utf8_lossy`,
    // so a full `feed()` round trip can observe the fix directly: a raw
    // 0xFF byte is never valid UTF-8 standalone, so it always becomes one
    // U+FFFD replacement character - the doubled (buggy) form is *two*
    // separate invalid bytes (0xFF is never a valid lead byte for the
    // second to continue), hence *two* U+FFFD; the correctly-undoubled
    // single byte is one. GMCP's and CHARSET's consumers instead use
    // strict `str::from_utf8`, which rejects *any* occurrence of 0xFF
    // outright - doubled or not - so neither can ever successfully decode
    // a message containing one, and a full round trip cannot distinguish
    // the fixed case from the bug (both silently drop the message,
    // identically). Those two are therefore pinned directly against
    // `unescape_iac`, applied to the exact `&sb_data[1..]` slice
    // `handle_subnegotiation` uses.
    // ==================================================================

    #[test]
    fn test_unescape_iac_collapses_doubled_bytes() {
        // One escaped pair among ordinary bytes.
        assert_eq!(
            unescape_iac(&[0x41, TELNET_IAC, TELNET_IAC, 0x42]),
            vec![0x41, TELNET_IAC, 0x42]
        );
        // Two literal 0xFF bytes back to back -> two doubled pairs.
        assert_eq!(
            unescape_iac(&[TELNET_IAC, TELNET_IAC, TELNET_IAC, TELNET_IAC]),
            vec![TELNET_IAC, TELNET_IAC]
        );
        // No IAC at all: passthrough unchanged.
        assert_eq!(unescape_iac(b"plain text, no escapes"), b"plain text, no escapes".to_vec());
        // Empty input.
        assert_eq!(unescape_iac(&[]), Vec::<u8>::new());
        // A malformed trailing lone IAC (no pair to complete it) is passed
        // through as-is rather than panicking or being dropped - shouldn't
        // arise from a well-formed subnegotiation (the terminator scan only
        // ever hands `handle_subnegotiation` a payload where every embedded
        // 0xFF is a complete doubled pair), but the function stays total.
        assert_eq!(unescape_iac(&[0x41, TELNET_IAC]), vec![0x41, TELNET_IAC]);
    }

    #[test]
    fn test_msdp_value_with_doubled_0xff_arrives_as_single_byte_end_to_end() {
        // IAC SB MSDP VAR "KEY" VAL "AB" <doubled 0xFF, i.e. one literal
        // 0xFF byte> "CD" IAC SE.
        let mut data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
        data.extend_from_slice(b"KEY");
        data.push(MSDP_VAL);
        data.extend_from_slice(b"AB");
        data.push(TELNET_IAC);
        data.push(TELNET_IAC); // escaped 0xFF -> one literal 0xFF byte
        data.extend_from_slice(b"CD");
        data.extend_from_slice(&[TELNET_IAC, TELNET_SE]);

        let mut session = TelnetSession::new(TelnetConfig::default());
        let outcome = session.feed(&data);

        let value = outcome.events.iter().find_map(|e| match e {
            TelnetEvent::MsdpVariable(name, val) if name == "KEY" => Some(val.clone()),
            _ => None,
        }).expect("expected an MsdpVariable(\"KEY\", ..) event");

        assert_eq!(
            value.matches('\u{FFFD}').count(),
            1,
            "the doubled 0xFF byte pair must collapse to a single literal byte \
             before parsing (one replacement char), not arrive doubled (two): got {value:?}"
        );
    }

    #[test]
    fn test_gmcp_payload_doubled_0xff_undoubled_before_utf8_split() {
        // GMCP JSON must be valid UTF-8 for handle_subnegotiation's strict
        // `str::from_utf8` split to succeed at all, and a raw 0xFF byte -
        // doubled on the wire or not - is never valid UTF-8 standalone, so
        // there is no way to observe this fix through a successfully
        // decoded GmcpMessage event: the strict from_utf8 rejects the
        // payload identically either way. This pins the exact byte-level
        // transformation handle_subnegotiation applies to `&sb_data[1..]`
        // before that from_utf8 call ever runs (see the module doc comment
        // above this test).
        let mut sb_data = vec![TELNET_OPT_GMCP];
        sb_data.extend_from_slice(b"Core.Test ");
        sb_data.push(TELNET_IAC);
        sb_data.push(TELNET_IAC); // doubled 0xFF -> one literal 0xFF byte
        sb_data.extend_from_slice(b"tail");

        let unescaped = unescape_iac(&sb_data[1..]);

        let mut expected = b"Core.Test ".to_vec();
        expected.push(TELNET_IAC);
        expected.extend_from_slice(b"tail");
        assert_eq!(unescaped, expected);
    }

    #[test]
    fn test_charset_name_doubled_0xff_undoubled_before_parse() {
        // Same reasoning as the GMCP test above: parse_charset_request's
        // per-name `str::from_utf8` rejects a literal 0xFF just as surely
        // as a doubled one, so this pins the byte-level fix directly rather
        // than through a successfully-parsed CharsetRequest event.
        let mut sb_data = vec![TELNET_OPT_CHARSET, CHARSET_REQUEST, b';'];
        sb_data.push(TELNET_IAC);
        sb_data.push(TELNET_IAC); // doubled 0xFF -> one literal 0xFF byte
        sb_data.extend_from_slice(b"SET");

        let unescaped = unescape_iac(&sb_data[1..]);

        let mut expected = vec![CHARSET_REQUEST, b';', TELNET_IAC];
        expected.extend_from_slice(b"SET");
        assert_eq!(unescaped, expected);
    }

    // ==================================================================
    // Step 3.2 (Job 9) — RFC 1143 Q method (finding 4).
    // ==================================================================

    #[test]
    fn test_support_him_and_support_us_policy_tables() {
        // Exactly today's process_telnet accept lists (finding 4's starting
        // point) — Job 9 changes when Clay replies, never which options it
        // accepts.
        for opt in [
            TELNET_OPT_SGA, TELNET_OPT_EOR, TELNET_OPT_GMCP, TELNET_OPT_MSDP,
            TELNET_OPT_MCCP2, TELNET_OPT_CHARSET,
        ] {
            assert!(support_him(opt), "support_him({opt}) should accept");
        }
        for opt in [TELNET_OPT_NAWS, TELNET_OPT_TTYPE, TELNET_OPT_EOR] {
            assert!(support_us(opt), "support_us({opt}) should accept");
        }
        // Cross-checks: NAWS/TTYPE are "us"-only, GMCP/MSDP/MCCP2/CHARSET/SGA
        // are "him"-only (EOR is the one option in both lists).
        for opt in [TELNET_OPT_NAWS, TELNET_OPT_TTYPE] {
            assert!(!support_him(opt), "support_him({opt}) should refuse");
        }
        for opt in [TELNET_OPT_SGA, TELNET_OPT_GMCP, TELNET_OPT_MSDP, TELNET_OPT_MCCP2, TELNET_OPT_CHARSET] {
            assert!(!support_us(opt), "support_us({opt}) should refuse");
        }
        // Unknown/unlisted option codes: refused by both.
        for opt in [0u8, 1, 2, 5, 99, 200, 255] {
            assert!(!support_him(opt), "support_him({opt}) should refuse (unlisted)");
            assert!(!support_us(opt), "support_us({opt}) should refuse (unlisted)");
        }
    }

    #[test]
    fn test_q_receive_will_wont_state_transitions_directly() {
        // Drives q_receive_will_wont directly against support_him, pinning
        // the No<->Yes transitions RFC 1143 actually reaches today (see
        // OptionState's doc comment for why WantNo*/WantYes* aren't
        // reachable yet).
        let mut session = TelnetSession::new(TelnetConfig::default());
        assert_eq!(session.options_him[TELNET_OPT_GMCP as usize], OptionState::No);

        // No -> Yes on an accepted option: DO reply + OptionEnabled.
        let mut wire = Vec::new();
        let mut events = Vec::new();
        session.q_receive_will_wont(TELNET_OPT_GMCP, true, &mut wire, &mut events);
        assert_eq!(session.options_him[TELNET_OPT_GMCP as usize], OptionState::Yes);
        assert_eq!(wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_GMCP]);
        assert_eq!(events, vec![TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)]);

        // Yes -> Yes (repeat WILL): no reply, no event, no state change.
        let mut wire2 = Vec::new();
        let mut events2 = Vec::new();
        session.q_receive_will_wont(TELNET_OPT_GMCP, true, &mut wire2, &mut events2);
        assert_eq!(session.options_him[TELNET_OPT_GMCP as usize], OptionState::Yes);
        assert!(wire2.is_empty());
        assert!(events2.is_empty());

        // Yes -> No (WONT): DONT reply + OptionDisabled.
        let mut wire3 = Vec::new();
        let mut events3 = Vec::new();
        session.q_receive_will_wont(TELNET_OPT_GMCP, false, &mut wire3, &mut events3);
        assert_eq!(session.options_him[TELNET_OPT_GMCP as usize], OptionState::No);
        assert_eq!(wire3, vec![TELNET_IAC, TELNET_DONT, TELNET_OPT_GMCP]);
        assert_eq!(events3, vec![TelnetEvent::OptionDisabled(TELNET_OPT_GMCP)]);

        // No -> No (repeat WONT while already off): no reply, no event.
        let mut wire4 = Vec::new();
        let mut events4 = Vec::new();
        session.q_receive_will_wont(TELNET_OPT_GMCP, false, &mut wire4, &mut events4);
        assert_eq!(session.options_him[TELNET_OPT_GMCP as usize], OptionState::No);
        assert!(wire4.is_empty());
        assert!(events4.is_empty());

        // An option we refuse (not in support_him): No -> No, but still
        // replies DONT every time — there is no third "already refused"
        // state in the flattened model, and RFC 1143's literal him=No rule
        // answers every WILL, not just the first (see q_receive_will_wont's
        // doc comment on the two reachable states).
        let mut wire5 = Vec::new();
        let mut events5 = Vec::new();
        session.q_receive_will_wont(99, true, &mut wire5, &mut events5);
        assert_eq!(session.options_him[99], OptionState::No);
        assert_eq!(wire5, vec![TELNET_IAC, TELNET_DONT, 99]);
        assert!(events5.is_empty());
    }

    #[test]
    fn test_q_receive_do_dont_state_transitions_directly() {
        let mut session = TelnetSession::new(TelnetConfig::default());
        assert_eq!(session.options_us[TELNET_OPT_NAWS as usize], OptionState::No);

        let mut wire = Vec::new();
        let mut events = Vec::new();
        let became = session.q_receive_do_dont(TELNET_OPT_NAWS, true, &mut wire, &mut events);
        assert!(became, "No -> Yes must report the fresh accept transition");
        assert_eq!(session.options_us[TELNET_OPT_NAWS as usize], OptionState::Yes);
        assert_eq!(wire, vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_NAWS]);
        assert_eq!(events, vec![TelnetEvent::OptionEnabled(TELNET_OPT_NAWS)]);

        let mut wire2 = Vec::new();
        let mut events2 = Vec::new();
        let became2 = session.q_receive_do_dont(TELNET_OPT_NAWS, true, &mut wire2, &mut events2);
        assert!(!became2, "repeat DO must not report a fresh transition");
        assert!(wire2.is_empty());
        assert!(events2.is_empty());

        let mut wire3 = Vec::new();
        let mut events3 = Vec::new();
        let became3 = session.q_receive_do_dont(TELNET_OPT_NAWS, false, &mut wire3, &mut events3);
        assert!(!became3);
        assert_eq!(session.options_us[TELNET_OPT_NAWS as usize], OptionState::No);
        assert_eq!(wire3, vec![TELNET_IAC, TELNET_WONT, TELNET_OPT_NAWS]);
        assert_eq!(events3, vec![TelnetEvent::OptionDisabled(TELNET_OPT_NAWS)]);

        // DO for an option Clay refuses: No -> No, WONT every time.
        let mut wire4 = Vec::new();
        let mut events4 = Vec::new();
        session.q_receive_do_dont(99, true, &mut wire4, &mut events4);
        assert_eq!(session.options_us[99], OptionState::No);
        assert_eq!(wire4, vec![TELNET_IAC, TELNET_WONT, 99]);
        assert!(events4.is_empty());
    }

    #[test]
    fn test_repeated_will_gmcp_produces_wire_bytes_on_the_first_only() {
        let mut session = TelnetSession::new(TelnetConfig::default());
        let data = [TELNET_IAC, TELNET_WILL, TELNET_OPT_GMCP];

        let first = session.feed(&data);
        assert_eq!(first.wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_GMCP]);
        assert!(first.events.contains(&TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)));

        // Same bytes again, same session: already Yes, so no reply and no
        // repeat OptionEnabled - finding 4's actual fix, and exactly what
        // stops two Q-method peers that both re-announce from ping-ponging
        // forever.
        let second = session.feed(&data);
        assert!(second.wire.is_empty(), "repeated WILL GMCP must produce no wire bytes the second time");
        assert!(
            !second.events.contains(&TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)),
            "repeated WILL GMCP must not fire a repeat OptionEnabled"
        );
    }

    #[test]
    fn test_will_and_do_unknown_option_each_refused_exactly_once() {
        let mut will_session = TelnetSession::new(TelnetConfig::default());
        let will_outcome = will_session.feed(&[TELNET_IAC, TELNET_WILL, 99]);
        assert_eq!(will_outcome.wire, vec![TELNET_IAC, TELNET_DONT, 99]);

        let mut do_session = TelnetSession::new(TelnetConfig::default());
        let do_outcome = do_session.feed(&[TELNET_IAC, TELNET_DO, 99]);
        assert_eq!(do_outcome.wire, vec![TELNET_IAC, TELNET_WONT, 99]);
    }

    #[test]
    fn test_wont_gmcp_after_enabled_will_gmcp_emits_option_disabled() {
        let mut session = TelnetSession::new(TelnetConfig::default());
        let will_outcome = session.feed(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_GMCP]);
        assert!(will_outcome.events.contains(&TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)));

        let wont_outcome = session.feed(&[TELNET_IAC, TELNET_WONT, TELNET_OPT_GMCP]);
        assert_eq!(wont_outcome.wire, vec![TELNET_IAC, TELNET_DONT, TELNET_OPT_GMCP]);
        assert_eq!(wont_outcome.events, vec![TelnetEvent::OptionDisabled(TELNET_OPT_GMCP)]);
    }

    // ==================================================================
    // Plan Phase 3, step 3.4 (Job 10b) — finding 7: real ECHO handling.
    // `IAC WILL ECHO` (a MUD saying "I will echo for you" - the standard way
    // a server asks a client to stop echoing locally for a password prompt)
    // used to fall through to the WILL match's generic refusal (`IAC DONT
    // ECHO`), so Clay never masked anything and passwords rendered in the
    // clear. `IAC WONT ECHO` already had its own special case
    // (`WontEchoPromptHint`, Job 4.5) but replied to nothing and carried no
    // real echo-state signal. ECHO deliberately stays outside the Q
    // method's `support_him`/`support_us` tables (Job 9's note): both
    // directions are answered unconditionally every time they're seen,
    // exactly like the pre-existing WONT ECHO special case, never
    // deduplicated against a No/Yes state.
    // ==================================================================

    #[test]
    fn test_will_echo_replies_do_echo_and_emits_echo_off() {
        let mut session = TelnetSession::new(TelnetConfig::default());
        let outcome = session.feed(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_ECHO]);
        assert_eq!(outcome.wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO]);
        // TelnetDetected also fires here (first IAC byte ever seen on a fresh session).
        assert_eq!(outcome.events, vec![TelnetEvent::TelnetDetected, TelnetEvent::EchoOff]);
    }

    #[test]
    fn test_wont_echo_replies_dont_echo_and_emits_echo_on_and_prompt_hint() {
        // The real echo-state signal (EchoOn) and the pre-existing prompt-boundary
        // heuristic (WontEchoPromptHint) are independent consumers of the same wire
        // event - both must fire, per EchoOn's doc comment.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let mut data = b"Login: ".to_vec();
        data.extend_from_slice(&[TELNET_IAC, TELNET_WONT, TELNET_OPT_ECHO]);
        let outcome = session.feed(&data);
        assert_eq!(outcome.wire, vec![TELNET_IAC, TELNET_DONT, TELNET_OPT_ECHO]);
        assert!(
            outcome.events.contains(&TelnetEvent::EchoOn),
            "expected EchoOn, got {:?}", outcome.events
        );
        assert!(
            outcome.events.contains(&TelnetEvent::WontEchoPromptHint),
            "WONT ECHO must still produce the pre-existing prompt-boundary hint \
             (Job 4.5) alongside the new real-echo signal, got {:?}", outcome.events
        );
    }

    #[test]
    fn test_will_echo_is_answered_every_time_not_deduplicated() {
        // Unlike the Q-method-backed options
        // (test_repeated_will_gmcp_produces_wire_bytes_on_the_first_only), ECHO has no
        // options_him entry to dedupe against - a repeated WILL ECHO gets a repeated DO
        // ECHO reply and a repeated EchoOff event every time, exactly like the
        // pre-existing WONT ECHO special case already did.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let data = [TELNET_IAC, TELNET_WILL, TELNET_OPT_ECHO];

        let first = session.feed(&data);
        assert_eq!(first.wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO]);
        // TelnetDetected also fires on this first call (first IAC byte ever seen).
        assert_eq!(first.events, vec![TelnetEvent::TelnetDetected, TelnetEvent::EchoOff]);

        let second = session.feed(&data);
        assert_eq!(second.wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO]);
        assert_eq!(second.events, vec![TelnetEvent::EchoOff], "TelnetDetected only fires once per session");
    }

    // ==================================================================
    // Step 1.3 (Job 3) — MCCP2 inline decompression in TelnetSession.
    //
    // These build REAL compressed fixtures via flate2::Compress (not
    // hand-rolled bytes), per the plan, and are the main proof that
    // finding 7's leftover bug ("compressed bytes appended after
    // decompressed ones without being decompressed") is now structurally
    // inexpressible: TelnetSession has exactly one path from compressed
    // bytes to `text`/further parsing (`run_decompressor`), and every test
    // below exercises it under a different chunking.
    // ==================================================================

    /// Build a complete (Finish-flushed) zlib-wrapped compressed blob for
    /// `input`, matching the wire format MCCP2 servers actually send
    /// (`Decompress::new(true)` in `mccp2_decompress*` expects the zlib
    /// header/trailer, not raw deflate).
    fn zlib_compress(input: &[u8]) -> Vec<u8> {
        use flate2::{Compress, Compression, FlushCompress};
        let mut compressed = vec![0u8; input.len() + 1024];
        let mut compressor = Compress::new(Compression::default(), true);
        let status = compressor
            .compress(input, &mut compressed, FlushCompress::Finish)
            .unwrap();
        assert_eq!(status, flate2::Status::StreamEnd, "fixture must be a complete zlib stream");
        let len = compressor.total_out() as usize;
        compressed.truncate(len);
        compressed
    }

    #[test]
    fn test_telnet_session_mccp2_activation_same_read_leftover() {
        // Finding 7's exact broken case in production today: the compressed
        // payload arrives in the SAME chunk as `IAC SB 86 IAC SE`. Every
        // MCCP2-aware reader loop except one gets this branch wrong (it
        // appends the still-compressed tail after the decompressed text
        // without ever decompressing it, desyncing the zlib stream
        // permanently). Here there is no "tail" to mishandle at all -
        // activation decompresses it in place before returning.
        let plaintext = b"Some post-activation MUD output.\n";
        let compressed = zlib_compress(plaintext);

        let mut data = Vec::new();
        data.extend_from_slice(b"pre-activation text\n");
        // T1.2 (plan Job 1, step 9): activation now requires a negotiated
        // options_him[MCCP2] == Yes, so every fixture here leads with IAC WILL MCCP2.
        data.extend_from_slice(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]);
        data.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        data.extend_from_slice(&compressed);

        let mut session = TelnetSession::new(TelnetConfig::default());
        let outcome = session.feed(&data);

        assert!(outcome.events.contains(&TelnetEvent::CompressionStarted));
        let mut expected = b"pre-activation text\n".to_vec();
        expected.extend_from_slice(plaintext);
        assert_eq!(outcome.text, expected);
        assert_eq!(
            outcome.wire,
            vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2],
            "WILL MCCP2 is answered DO; the bare SB itself gets no reply"
        );
    }

    #[test]
    fn test_telnet_session_mccp2_sb_split_across_reads() {
        // The activation SB itself (`IAC SB 86 IAC SE`) is split across two
        // feed() calls, with the real compressed payload only arriving
        // after the split - i.e. the compressed bytes are not even present
        // yet when the first chunk is parsed.
        let plaintext = b"after activation\n";
        let compressed = zlib_compress(plaintext);

        let mut session = TelnetSession::new(TelnetConfig::default());
        // T1.2 (plan Job 1, step 9): negotiate before the (now-gated) activation SB.
        let negotiate = session.feed(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]);
        assert_eq!(negotiate.wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2]);

        let first = session.feed(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2]);
        assert!(first.text.is_empty());
        assert!(!first.events.contains(&TelnetEvent::CompressionStarted), "SB not terminated yet");

        let mut second_input = vec![TELNET_IAC, TELNET_SE];
        second_input.extend_from_slice(&compressed);
        let second = session.feed(&second_input);
        assert!(second.events.contains(&TelnetEvent::CompressionStarted));
        assert_eq!(second.text, plaintext);
    }

    #[test]
    fn test_telnet_session_mccp2_split_invariance_every_offset() {
        // The main proof for Job 3, extending Job 2's split-invariance
        // property (`test_telnet_session_split_invariance`) across MCCP2
        // activation and inline decompression: plaintext before
        // activation, the activation sequence, a real compressed payload,
        // Z_STREAM_END recovery, and plaintext negotiation afterward, all
        // in one fixture, fed in two chunks split at EVERY possible byte
        // offset. Two-chunk feeding must equal one-shot feeding byte for
        // byte no matter where the split falls - including inside the SB,
        // inside the compressed bytes, and inside the trailing negotiation.
        let plaintext = b"Room description here.\nExits: north, south.\n";
        let compressed = zlib_compress(plaintext);

        let mut fixture = Vec::new();
        fixture.extend_from_slice(b"before\n");
        // T1.2 (plan Job 1, step 9): negotiate before the (now-gated) activation SB.
        fixture.extend_from_slice(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]);
        fixture.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        fixture.extend_from_slice(&compressed);
        fixture.extend_from_slice(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_SGA]);
        fixture.extend_from_slice(b"after\n");

        let (base_text, base_wire, base_events) = run_one_shot(&fixture);
        assert_eq!(
            base_text,
            b"before\nRoom description here.\nExits: north, south.\nafter\n".to_vec()
        );
        assert_eq!(
            base_wire,
            vec![
                TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2,
                TELNET_IAC, TELNET_DO, TELNET_OPT_SGA,
            ]
        );
        assert!(base_events.contains(&TelnetEvent::CompressionStarted));
        assert!(base_events.contains(&TelnetEvent::CompressionEnded));

        for k in 0..=fixture.len() {
            let mut session = TelnetSession::new(TelnetConfig::default());
            let mut o1 = session.feed(&fixture[..k]);
            let o2 = session.feed(&fixture[k..]);
            o1.text.extend_from_slice(&o2.text);
            o1.wire.extend_from_slice(&o2.wire);
            o1.events.extend(o2.events);
            assert_eq!(o1.text, base_text, "split at {k}: text");
            assert_eq!(o1.wire, base_wire, "split at {k}: wire");
            assert_eq!(o1.events, base_events, "split at {k}: events");
        }
    }

    #[test]
    fn test_telnet_session_mccp2_stream_end_recovery_to_plaintext_negotiation() {
        // Finding 1's recovery path, which does not exist in production
        // today: once the zlib stream ends (Z_STREAM_END), the session
        // drops the decompressor, emits CompressionEnded, and keeps parsing
        // SUBSEQUENT bytes as ordinary telnet - including a negotiation the
        // far end sends right after turning MCCP2 off. Today's callers keep
        // the finished decompressor forever and every later byte silently
        // vanishes.
        let plaintext = b"final compressed line\n";
        let compressed = zlib_compress(plaintext);

        let mut data = Vec::new();
        // T1.2 (plan Job 1, step 9): negotiate before the (now-gated) activation SB.
        data.extend_from_slice(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]);
        data.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        data.extend_from_slice(&compressed);
        data.extend_from_slice(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_SGA]);

        let mut session = TelnetSession::new(TelnetConfig::default());
        let outcome = session.feed(&data);

        assert!(outcome.events.contains(&TelnetEvent::CompressionStarted));
        assert!(outcome.events.contains(&TelnetEvent::CompressionEnded));
        assert_eq!(outcome.text, plaintext);
        assert_eq!(
            outcome.wire,
            vec![
                TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2,
                TELNET_IAC, TELNET_DO, TELNET_OPT_SGA,
            ],
            "negotiation arriving right after Z_STREAM_END must still be answered"
        );

        // And the session must still be usable afterward for ordinary text.
        let more = session.feed(b"plain again\n");
        assert_eq!(more.text, b"plain again\n");
    }

    #[test]
    fn test_telnet_session_mccp2_decompressed_stream_contains_gmcp() {
        // Proves the parser runs over DECOMPRESSED bytes, not raw ones: this
        // GMCP subnegotiation exists only once the compressed payload has
        // been inflated. If the session were (bug) still scanning the raw
        // compressed bytes for telnet framing, it could never see it.
        let mut inner = Vec::new();
        inner.extend_from_slice(b"Something happened.\n");
        inner.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP]);
        inner.extend_from_slice(b"Room.Info {\"name\":\"Temple\"}");
        inner.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        inner.extend_from_slice(b"more text after gmcp\n");
        let compressed = zlib_compress(&inner);

        // T1.2 (plan Job 1, step 9): negotiate before the (now-gated) activation SB.
        let mut data = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2];
        data.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        data.extend_from_slice(&compressed);

        let mut session = TelnetSession::new(TelnetConfig::default());
        let outcome = session.feed(&data);

        assert_eq!(outcome.text, b"Something happened.\nmore text after gmcp\n".to_vec());
        assert!(outcome.events.contains(&TelnetEvent::GmcpMessage(
            "Room.Info".to_string(),
            "{\"name\":\"Temple\"}".to_string()
        )));
    }

    #[test]
    fn test_telnet_session_mccp2_bomb_guard() {
        // The decompression-bomb guard: a genuinely tiny compressed payload
        // (a long run of zero bytes compresses extremely well) that would
        // decompress to far more than MCCP2_BOMB_LIMIT. The session must
        // stop growing output and emit a CompressionFailed event (T1.14 -
        // NOT CompressionEnded, which means "server turned compression off,
        // plaintext follows" and is wrong for an abort) rather than trying
        // to hand the caller an unbounded Vec<u8>.
        //
        // T1.14 / design decision D1: unlike the pre-fix behaviour, the session is NOT
        // usable afterward - a bomb abort poisons the session exactly like an inflate
        // error, because the still-compressing server's later bytes are not safe to parse
        // as telnet (see CompressionFailed's doc comment). Note this test is deliberately
        // NOT part of the split-invariance suite: the bomb check is evaluated per
        // `run_decompressor` call against that call's own output alone (see
        // `mccp2_decompress_inner`'s loop), so whether/when it trips depends on how much a
        // single `feed()` call's chunk decompresses to - splitting the same compressed
        // bytes across many small `feed()` calls could avoid ever tripping it at all. That
        // is a known, accepted limitation of a per-call guard, not something this test
        // (or T1.1's split-invariance test, which uses non-zlib garbage instead) needs to
        // paper over.
        let huge = vec![0u8; MCCP2_BOMB_LIMIT * 2];
        let compressed = zlib_compress(&huge);
        assert!(
            compressed.len() < MCCP2_BOMB_LIMIT / 4,
            "fixture should compress to a small fraction of the cap: {} bytes",
            compressed.len()
        );

        // T1.2 (plan Job 1, step 9): negotiate before the (now-gated) activation SB.
        let mut data = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2];
        data.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        data.extend_from_slice(&compressed);

        let mut session = TelnetSession::new(TelnetConfig::default());
        let outcome = session.feed(&data);

        assert!(outcome.events.contains(&TelnetEvent::CompressionStarted));
        assert!(
            !outcome.events.contains(&TelnetEvent::CompressionEnded),
            "T1.14: a bomb abort must not claim a clean stream end, got {:?}",
            outcome.events
        );
        assert!(
            outcome.events.iter().any(|e| matches!(
                e,
                TelnetEvent::CompressionFailed(msg) if msg.contains("MCCP2")
            )),
            "expected a CompressionFailed naming MCCP2, got {:?}",
            outcome.events
        );
        assert!(
            outcome.text.len() >= MCCP2_BOMB_LIMIT,
            "should have produced at least the cap's worth of text, got {}",
            outcome.text.len()
        );
        assert!(
            outcome.text.len() < MCCP2_BOMB_LIMIT + 65_536,
            "should not have run far past the cap, got {}",
            outcome.text.len()
        );

        // T1.14 / D1: the session is poisoned, not merely reset to plaintext parsing -
        // every later feed() returns an empty outcome structurally.
        let recovered = session.feed(b"plain text again\n");
        assert!(recovered.text.is_empty(), "poisoned session parses nothing further (D1)");
        assert!(recovered.wire.is_empty());
        assert!(recovered.events.is_empty());
    }

    // ==================================================================
    // Plan Job 1 (investigate-differences-between-tinyfugu-fluffy-stallman.md),
    // T1.1/T1.2/D1 — the negotiation gate, the poisoned-session guarantees, and
    // the leftover-retention bound.
    // ==================================================================

    #[test]
    fn test_telnet_session_mccp2_sb_without_negotiation_ignored() {
        // T1.2: an unrequested activation subnegotiation must not activate
        // compression - options_him[MCCP2] is still No on a fresh session. Plaintext
        // parsing continues; what follows is ordinary text, never handed to a
        // decompressor.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let mut data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE];
        data.extend_from_slice(b"still plaintext\n");
        let outcome = session.feed(&data);

        assert!(
            !outcome.events.contains(&TelnetEvent::CompressionStarted),
            "unrequested SB must not activate compression, got {:?}",
            outcome.events
        );
        assert!(
            outcome.events.iter().any(|e| matches!(e, TelnetEvent::ProtocolError(_))),
            "expected a ProtocolError for the unrequested SB, got {:?}",
            outcome.events
        );
        assert_eq!(outcome.text, b"still plaintext\n".to_vec());
        assert!(session.decomp.is_none(), "no decompressor should ever have been created");
    }

    /// Like `zlib_compress` but keeps the zlib stream open (`FlushCompress::Sync` instead
    /// of `Finish`) — every byte of `input` fed so far is fully recoverable without a
    /// trailing `Status::StreamEnd`, so `self.decomp` stays live/`Some` after decompressing
    /// it. Needed to test T1.2 Trigger B (a duplicate activation sequence appearing
    /// *inside* a still-open compressed stream) — `zlib_compress`'s `Finish`-flushed
    /// fixtures always end the stream in the same call that decompresses them, which would
    /// make `self.decomp` already `None` by the time the embedded bytes are scanned,
    /// masking exactly the bug this is supposed to catch.
    fn zlib_compress_streaming(input: &[u8]) -> Vec<u8> {
        use flate2::{Compress, Compression, FlushCompress};
        let mut compressed = vec![0u8; input.len() + 1024];
        let mut compressor = Compress::new(Compression::default(), true);
        let status = compressor
            .compress(input, &mut compressed, FlushCompress::Sync)
            .unwrap();
        assert_ne!(
            status,
            flate2::Status::StreamEnd,
            "fixture must NOT end the zlib stream - decomp must stay live"
        );
        let len = compressor.total_out() as usize;
        compressed.truncate(len);
        compressed
    }

    #[test]
    fn test_telnet_session_mccp2_duplicate_sb_inside_live_stream_ignored() {
        // T1.2 Trigger B: the four activation bytes appearing again INSIDE an
        // already-live compressed stream must not reset decomp - that would discard the
        // live zlib window and try to decompress already-plaintext bytes as if they were
        // still compressed. Design decision D1: this is not a failure - one ProtocolError,
        // the live decompressor keeps running, and the surrounding text survives intact.
        let mut inner = Vec::new();
        inner.extend_from_slice(b"before duplicate SB\n");
        inner.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        inner.extend_from_slice(b"after duplicate SB\n");
        let compressed = zlib_compress_streaming(&inner);

        let mut session = TelnetSession::new(TelnetConfig::default());
        let negotiate = session.feed(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]);
        assert_eq!(negotiate.wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2]);

        let mut data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE];
        data.extend_from_slice(&compressed);
        let outcome = session.feed(&data);

        let started = outcome
            .events
            .iter()
            .filter(|e| **e == TelnetEvent::CompressionStarted)
            .count();
        assert_eq!(started, 1, "exactly one real activation, got {:?}", outcome.events);
        let proto_errors = outcome
            .events
            .iter()
            .filter(|e| matches!(e, TelnetEvent::ProtocolError(_)))
            .count();
        assert_eq!(
            proto_errors, 1,
            "exactly one ProtocolError for the duplicate SB, got {:?}",
            outcome.events
        );
        assert!(
            !outcome.events.iter().any(|e| matches!(e, TelnetEvent::CompressionFailed(_))),
            "a duplicate activation is not a failure (D1), got {:?}",
            outcome.events
        );
        assert_eq!(
            outcome.text,
            b"before duplicate SB\nafter duplicate SB\n".to_vec(),
            "text survives the duplicate SB intact"
        );
        assert!(session.decomp.is_some(), "the live decompressor must survive the duplicate SB");
    }

    #[test]
    fn test_telnet_session_mccp2_reactivate_after_stream_end() {
        // Guards T1.2's gate against over-tightening: once MCCP2 has legitimately
        // negotiated to Yes and then cleanly ended (Z_STREAM_END / CompressionEnded), a
        // SECOND activation must still work - no repeat WILL/DO needed, since
        // options_him[MCCP2] is already Yes and decomp is None again after the first
        // stream ended.
        let first_plaintext = b"first burst\n";
        let first_compressed = zlib_compress(first_plaintext);
        let second_plaintext = b"second burst\n";
        let second_compressed = zlib_compress(second_plaintext);

        let mut session = TelnetSession::new(TelnetConfig::default());
        let negotiate = session.feed(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]);
        assert_eq!(negotiate.wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2]);

        let mut first_data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE];
        first_data.extend_from_slice(&first_compressed);
        let first_outcome = session.feed(&first_data);
        assert!(first_outcome.events.contains(&TelnetEvent::CompressionStarted));
        assert!(first_outcome.events.contains(&TelnetEvent::CompressionEnded));
        assert_eq!(first_outcome.text, first_plaintext);

        // Second activation, no fresh WILL/DO needed - options_him[MCCP2] is still Yes.
        let mut second_data = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE];
        second_data.extend_from_slice(&second_compressed);
        let second_outcome = session.feed(&second_data);
        assert!(second_outcome.events.contains(&TelnetEvent::CompressionStarted));
        assert!(second_outcome.events.contains(&TelnetEvent::CompressionEnded));
        assert_eq!(second_outcome.text, second_plaintext);
    }

    #[test]
    fn test_telnet_session_mccp2_error_does_not_retain_pending_compressed_forever() {
        // T1.1's exact bug: before this fix, an inflate error left `consumed` short of the
        // whole buffer (sometimes 0), so the ENTIRE unconsumed remainder piled into
        // `pending_compressed` forever, re-failing the same way on every later feed() call
        // while producing no output ever again. Feed 1000 chunks of clearly-non-zlib
        // garbage after a real activation and confirm the leftover never grows past zero.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let negotiate = session.feed(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]);
        assert_eq!(negotiate.wire, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2]);
        let activate = session.feed(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        assert!(activate.events.contains(&TelnetEvent::CompressionStarted));
        assert_eq!(session.pending_compressed_len(), 0);

        // 0xAA's low nibble (0xA = 10) is not zlib's deflate method (8), so
        // Decompress::decompress rejects it as an invalid header on the very first call.
        let garbage = vec![0xAAu8; 8192];
        let first = session.feed(&garbage);
        assert!(
            first.events.iter().any(|e| matches!(e, TelnetEvent::CompressionFailed(_))),
            "expected CompressionFailed on the first garbage chunk, got {:?}",
            first.events
        );
        assert_eq!(session.pending_compressed_len(), 0);

        for _ in 0..999 {
            let outcome = session.feed(&garbage);
            assert!(outcome.text.is_empty(), "poisoned session parses nothing further (D1)");
            assert!(outcome.wire.is_empty());
            assert!(outcome.events.is_empty());
            assert_eq!(session.pending_compressed_len(), 0, "T1.1: leftover must never grow");
        }
    }

    #[test]
    fn test_telnet_session_mccp2_error_split_invariance() {
        // T1.1's split-invariance companion: an inflate error must produce the exact same
        // aggregate text/wire/events no matter where the 64 garbage bytes are chopped up
        // across feed() calls - the same property test_telnet_session_split_invariance
        // established for ordinary parsing, now covering the poisoned-stream path. The
        // CompressionFailed reason string is deliberately never compared for exact
        // equality across chunkings by anything other than this same aggregate-equality
        // check (see CompressionFailed's doc comment on why it must be split-invariant).
        let mut fixture = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2];
        fixture.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE]);
        fixture.extend_from_slice(&[0xAAu8; 64]);

        let (base_text, base_wire, base_events) = run_one_shot(&fixture);
        assert!(
            base_events.iter().any(|e| matches!(e, TelnetEvent::CompressionFailed(_))),
            "fixture must actually poison the session, got {base_events:?}"
        );

        for k in 0..=fixture.len() {
            let mut session = TelnetSession::new(TelnetConfig::default());
            let mut o1 = session.feed(&fixture[..k]);
            let o2 = session.feed(&fixture[k..]);
            o1.text.extend_from_slice(&o2.text);
            o1.wire.extend_from_slice(&o2.wire);
            o1.events.extend(o2.events);
            assert_eq!(o1.text, base_text, "split at {k}: text");
            assert_eq!(o1.wire, base_wire, "split at {k}: wire");
            assert_eq!(o1.events, base_events, "split at {k}: events");
        }
    }

    // ==================================================================
    // Job 12 (plan Phase 4, 4.1) — MTTS terminal type cycling.
    // ==================================================================

    /// One `IAC SB TTYPE SEND IAC SE` request/response round trip.
    fn ttype_send() -> Vec<u8> {
        vec![TELNET_IAC, TELNET_SB, TELNET_OPT_TTYPE, TTYPE_SEND, TELNET_IAC, TELNET_SE]
    }

    #[test]
    fn test_mtts_cycle_three_requests_then_repeats() {
        // The plan's required test: three consecutive requests produce three
        // different answers, byte for byte, and a fourth repeats the third.
        let mut session = TelnetSession::new(TelnetConfig::default());

        let o1 = session.feed(&ttype_send());
        assert_eq!(o1.wire, build_ttype_response("CLAY"), "1st answer: client name");

        let o2 = session.feed(&ttype_send());
        assert_eq!(o2.wire, build_ttype_response("ANSI"), "2nd answer: terminal type");

        let o3 = session.feed(&ttype_send());
        let mtts_answer = build_ttype_response("MTTS 269"); // ANSI(1)+UTF8(4)+256COLOR(8)+TRUECOLOR(256)
        assert_eq!(o3.wire, mtts_answer, "3rd answer: MTTS <bitmask>");

        let o4 = session.feed(&ttype_send());
        assert_eq!(o4.wire, mtts_answer, "4th answer must repeat the 3rd exactly");

        // All three/four requests must still fire TtypeRequested every time -
        // Job 12 changes what goes on the wire, not the event vocabulary.
        for o in [&o1, &o2, &o3, &o4] {
            assert!(o.events.contains(&TelnetEvent::TtypeRequested));
        }
    }

    #[test]
    fn test_mtts_uses_configured_client_name_and_term_type() {
        // The first two cycle positions come straight from TelnetConfig, not
        // hardcoded "CLAY"/"ANSI" - and both are uppercased per the MTTS
        // convention even when the config isn't.
        let cfg = TelnetConfig {
            term_type: "xterm-256color".to_string(),
            client_name: "clay-test".to_string(),
            ..TelnetConfig::default()
        };
        let mut session = TelnetSession::new(cfg);

        let o1 = session.feed(&ttype_send());
        assert_eq!(o1.wire, build_ttype_response("CLAY-TEST"));

        let o2 = session.feed(&ttype_send());
        assert_eq!(o2.wire, build_ttype_response("XTERM-256COLOR"));
    }

    #[test]
    fn test_mtts_ssl_bit_follows_config_flag() {
        // The plan's required test: the SSL bit follows cfg.is_tls, not a
        // hardcoded assumption either way.
        let mut plain = TelnetSession::new(TelnetConfig { is_tls: false, ..TelnetConfig::default() });
        plain.feed(&ttype_send());
        plain.feed(&ttype_send());
        let plain_mtts = plain.feed(&ttype_send());
        assert_eq!(plain_mtts.wire, build_ttype_response("MTTS 269"));

        let mut tls = TelnetSession::new(TelnetConfig { is_tls: true, ..TelnetConfig::default() });
        tls.feed(&ttype_send());
        tls.feed(&ttype_send());
        let tls_mtts = tls.feed(&ttype_send());
        assert_eq!(tls_mtts.wire, build_ttype_response("MTTS 2317")); // 269 | MTTS_SSL(2048)
    }

    #[test]
    fn test_mtts_fresh_session_restarts_the_cycle() {
        // The plan's required test: a fresh session (i.e. a reconnect, which
        // always spawns a brand new TelnetSession) starts the cycle over
        // rather than continuing wherever a previous connection left off.
        let mut session = TelnetSession::new(TelnetConfig::default());
        session.feed(&ttype_send());
        session.feed(&ttype_send());
        let third = session.feed(&ttype_send());
        assert_eq!(third.wire, build_ttype_response("MTTS 269"));
        drop(session);

        let mut fresh = TelnetSession::new(TelnetConfig::default());
        let first_again = fresh.feed(&ttype_send());
        assert_eq!(
            first_again.wire,
            build_ttype_response("CLAY"),
            "a fresh session must answer with the client name again, not continue the old cycle"
        );
    }

    // ==================================================================
    // Job 12 (plan Phase 4, 4.2) — MSSP.
    // ==================================================================

    #[test]
    fn test_parse_mssp_pairs_realistic_multi_field_payload() {
        let mut data = Vec::new();
        for (name, value) in [
            ("NAME", "Clay Test MUD"),
            ("PLAYERS", "3"),
            ("UPTIME", "1234567890"),
            ("CODEBASE", "Clay"),
            ("HOSTNAME", "mud.example.com"),
            ("PORT", "4000"),
        ] {
            data.push(MSSP_VAR);
            data.extend_from_slice(name.as_bytes());
            data.push(MSSP_VAL);
            data.extend_from_slice(value.as_bytes());
        }
        let pairs = parse_mssp_pairs(&data);
        assert_eq!(
            pairs,
            vec![
                ("NAME".to_string(), "Clay Test MUD".to_string()),
                ("PLAYERS".to_string(), "3".to_string()),
                ("UPTIME".to_string(), "1234567890".to_string()),
                ("CODEBASE".to_string(), "Clay".to_string()),
                ("HOSTNAME".to_string(), "mud.example.com".to_string()),
                ("PORT".to_string(), "4000".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_mssp_pairs_repeated_name_keeps_both_values() {
        // A name may legitimately carry more than one value (e.g. GENRE) -
        // this must never collapse into one entry or concatenate the values.
        let mut data = Vec::new();
        data.push(MSSP_VAR);
        data.extend_from_slice(b"GENRE");
        data.push(MSSP_VAL);
        data.extend_from_slice(b"Adventure");
        data.push(MSSP_VAL);
        data.extend_from_slice(b"Fantasy");
        let pairs = parse_mssp_pairs(&data);
        assert_eq!(
            pairs,
            vec![
                ("GENRE".to_string(), "Adventure".to_string()),
                ("GENRE".to_string(), "Fantasy".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_mssp_pairs_truncated_and_malformed_input_does_not_panic_or_half_populate() {
        // VAR with no VAL at all, cut off mid-name.
        assert_eq!(parse_mssp_pairs(&[MSSP_VAR]), vec![]);
        assert_eq!(parse_mssp_pairs(&[MSSP_VAR, b'N', b'A']), vec![]);

        // A stray VAL before any VAR is dropped, not paired with an invented name.
        assert_eq!(parse_mssp_pairs(&[MSSP_VAL, b'x']), vec![]);

        // VAR name VAL with nothing after it - a legitimate empty value, not
        // a crash and not a dropped pair (both markers are actually present).
        let mut trailing_empty = vec![MSSP_VAR];
        trailing_empty.extend_from_slice(b"NAME");
        trailing_empty.push(MSSP_VAL);
        assert_eq!(parse_mssp_pairs(&trailing_empty), vec![("NAME".to_string(), String::new())]);

        // A well-formed pair followed by a truncated second VAR must still
        // return the first pair intact - a later failure never un-produces
        // an earlier success.
        let mut mixed = vec![MSSP_VAR];
        mixed.extend_from_slice(b"NAME");
        mixed.push(MSSP_VAL);
        mixed.extend_from_slice(b"Foo");
        mixed.push(MSSP_VAR); // second VAR, no VAL, no name bytes even
        assert_eq!(parse_mssp_pairs(&mixed), vec![("NAME".to_string(), "Foo".to_string())]);

        // Completely empty payload.
        assert_eq!(parse_mssp_pairs(&[]), vec![]);
    }

    #[test]
    fn test_telnet_session_emits_mssp_data_event() {
        let mut sb = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSSP];
        sb.push(MSSP_VAR);
        sb.extend_from_slice(b"NAME");
        sb.push(MSSP_VAL);
        sb.extend_from_slice(b"Test MUD");
        sb.extend_from_slice(&[TELNET_IAC, TELNET_SE]);

        let mut session = TelnetSession::new(TelnetConfig::default());
        let outcome = session.feed(&sb);
        assert!(outcome.wire.is_empty(), "MSSP needs no reply, got {:?}", outcome.wire);
        assert!(
            outcome.events.contains(&TelnetEvent::MsspData(vec![("NAME".to_string(), "Test MUD".to_string())])),
            "expected MsspData event, got {:?}",
            outcome.events
        );
    }

    #[test]
    fn test_telnet_session_mssp_split_invariance() {
        // The plan's highest-value test, applied to MSSP specifically: feed
        // the whole IAC SB MSSP ... IAC SE payload one byte at a time and
        // confirm the parsed pairs match a one-shot feed exactly, at every
        // possible split point.
        let mut sb = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSSP];
        for (name, value) in [("NAME", "Test MUD"), ("GENRE", "Adventure")] {
            sb.push(MSSP_VAR);
            sb.extend_from_slice(name.as_bytes());
            sb.push(MSSP_VAL);
            sb.extend_from_slice(value.as_bytes());
        }
        sb.push(MSSP_VAR);
        sb.extend_from_slice(b"GENRE");
        sb.push(MSSP_VAL);
        sb.extend_from_slice(b"Fantasy"); // second value for the same name
        sb.extend_from_slice(&[TELNET_IAC, TELNET_SE]);

        let (base_text, base_wire, base_events) = run_one_shot(&sb);
        assert!(
            base_events.iter().any(|e| matches!(e, TelnetEvent::MsspData(_))),
            "sanity: fixture must actually produce an MsspData event"
        );

        for k in 0..=sb.len() {
            let mut session = TelnetSession::new(TelnetConfig::default());
            let mut o1 = session.feed(&sb[..k]);
            let o2 = session.feed(&sb[k..]);
            o1.text.extend_from_slice(&o2.text);
            o1.wire.extend_from_slice(&o2.wire);
            o1.events.extend(o2.events);
            assert_eq!(o1.text, base_text, "split at {k}: text");
            assert_eq!(o1.wire, base_wire, "split at {k}: wire");
            assert_eq!(o1.events, base_events, "split at {k}: events");
        }

        // Feed it one byte at a time across the whole session (not just
        // two-way splits) for good measure - same property, finer grain.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let mut byte_by_byte_events = Vec::new();
        for b in &sb {
            byte_by_byte_events.extend(session.feed(std::slice::from_ref(b)).events);
        }
        assert_eq!(byte_by_byte_events, base_events, "byte-at-a-time feed must match one-shot exactly");
    }

    // ==================================================================
    // Job 12 (plan Phase 4, 4.3) — MSDP outbound (LIST/REPORT/UNREPORT/SEND).
    // ==================================================================

    #[test]
    fn test_msdp_value_containing_0xff_byte_is_iac_doubled() {
        // The plan's required test: a value containing 0xFF must be
        // IAC-doubled (push_escaped, Job 1), not passed through raw - a raw
        // 0xFF in a subnegotiation payload would be misread as the start of
        // an escape sequence by the far end.
        //
        // Exercised directly against `push_escaped` - the actual escaping
        // primitive `build_msdp_request`/`build_msdp_set` both call for
        // their name/value payload bytes - rather than through those
        // `&str`-typed builders themselves: 0xFF is not a valid byte in any
        // position of any UTF-8 sequence, so no real `&str` can ever contain
        // one (a `String::from_utf8`/`str::from_utf8` over bytes including
        // it fails to validate, and constructing one via
        // `from_utf8_unchecked` anyway would be genuine undefined behavior,
        // not a shortcut worth taking for a test). Every byte
        // `build_msdp_request`/`build_msdp_set` ever hand to `push_escaped`
        // comes from a `&str`'s `.as_bytes()`, so this is the real reachable
        // shape of "MSDP binary values make push_escaped load-bearing" -
        // `test_build_msdp_request_framing_and_payload`/
        // `test_build_msdp_set_framing_and_payload` already cover the
        // `&str` builders byte-for-byte for ordinary (including
        // multi-byte UTF-8) values.
        let mut out = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
        push_escaped(&mut out, &[b'A', 0xFF, b'B']);
        out.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        assert_eq!(
            out,
            vec![
                TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR,
                b'A', TELNET_IAC, TELNET_IAC, b'B', // 0xFF doubled (TELNET_IAC == 0xFF)
                TELNET_IAC, TELNET_SE,
            ]
        );

        // Same guarantee on the two-argument shape (build_msdp_set), with
        // the value carrying the 0xFF byte.
        let mut out2 = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR];
        push_escaped(&mut out2, b"REPORT");
        out2.push(MSDP_VAL);
        push_escaped(&mut out2, &[b'H', 0xFF]);
        out2.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        assert_eq!(
            out2,
            vec![
                TELNET_IAC, TELNET_SB, TELNET_OPT_MSDP, MSDP_VAR,
                b'R', b'E', b'P', b'O', b'R', b'T',
                MSDP_VAL,
                b'H', TELNET_IAC, TELNET_IAC,
                TELNET_IAC, TELNET_SE,
            ]
        );
    }

    // ==================================================================
    // Job 14 (plan Phase 4) — MSP (`!!SOUND(...)` / `!!MUSIC(...)`).
    // ==================================================================

    #[test]
    fn test_msp_marker_lengths_match() {
        // extract_msp_triggers relies on this to use one MSP_MARKER_LEN
        // constant for both markers.
        assert_eq!(MSP_SOUND_MARKER.len(), MSP_MUSIC_MARKER.len());
    }

    #[test]
    fn test_parse_msp_trigger_bare_name_uses_defaults() {
        let t = parse_msp_trigger(false, b"bell.wav");
        assert_eq!(t, MspTrigger {
            is_music: false, off: false, name: "bell.wav".to_string(),
            volume: 100, loops: 1, priority: 50, media_type: None, url: None,
        });
    }

    #[test]
    fn test_parse_msp_trigger_all_parameters() {
        let t = parse_msp_trigger(false, b"bell.wav V=50 L=3 P=10 T=combat U=http://example.com/sounds/");
        assert_eq!(t, MspTrigger {
            is_music: false, off: false, name: "bell.wav".to_string(),
            volume: 50, loops: 3, priority: 10,
            media_type: Some("combat".to_string()),
            url: Some("http://example.com/sounds/".to_string()),
        });
    }

    #[test]
    fn test_parse_msp_trigger_music_flag_and_infinite_loop() {
        let t = parse_msp_trigger(true, b"theme.mp3 L=-1");
        assert!(t.is_music);
        assert_eq!(t.loops, -1);
    }

    #[test]
    fn test_parse_msp_trigger_off_is_case_insensitive_and_carries_no_name() {
        for spelling in ["Off", "off", "OFF", "oFF"] {
            let t = parse_msp_trigger(false, spelling.as_bytes());
            assert!(t.off, "{spelling} should parse as Off");
            assert_eq!(t.name, "");
        }
    }

    #[test]
    fn test_msp_volume_and_loops_clamp_at_both_ends() {
        // Volume: below 0 and above 100.
        assert_eq!(parse_msp_trigger(false, b"x.wav V=-10").volume, 0);
        assert_eq!(parse_msp_trigger(false, b"x.wav V=500").volume, 100);
        // Loops: 0 and negative-but-not--1 clamp up to the default (1); a
        // huge value clamps down to the cap; -1 (infinite) passes through
        // unchanged.
        assert_eq!(parse_msp_trigger(false, b"x.wav L=0").loops, 1);
        assert_eq!(parse_msp_trigger(false, b"x.wav L=-5").loops, 1);
        assert_eq!(parse_msp_trigger(false, b"x.wav L=99999").loops, 100);
        assert_eq!(parse_msp_trigger(false, b"x.wav L=-1").loops, -1);
        // Priority, same shape as volume.
        assert_eq!(parse_msp_trigger(false, b"x.wav P=-1").priority, 0);
        assert_eq!(parse_msp_trigger(false, b"x.wav P=1000").priority, 100);
    }

    #[test]
    fn test_msp_trigger_stripped_from_text_surrounding_text_preserved() {
        let mut session = TelnetSession::new(TelnetConfig::default());
        let input = b"You hear a bell. !!SOUND(bell.wav V=50)\r\nMore text\r\n";
        let outcome = session.feed(input);
        assert_eq!(outcome.text, b"You hear a bell. \r\nMore text\r\n");
        assert_eq!(outcome.events.len(), 1);
        match &outcome.events[0] {
            TelnetEvent::MspTrigger(t) => {
                assert!(!t.is_music);
                assert_eq!(t.name, "bell.wav");
                assert_eq!(t.volume, 50);
            }
            other => panic!("expected MspTrigger, got {other:?}"),
        }
    }

    #[test]
    fn test_msp_multiple_triggers_on_one_line() {
        let mut session = TelnetSession::new(TelnetConfig::default());
        let input = b"!!SOUND(a.wav) and !!MUSIC(b.mp3)\r\n";
        let outcome = session.feed(input);
        assert_eq!(outcome.text, b" and \r\n");
        assert_eq!(outcome.events.len(), 2);
        assert!(matches!(&outcome.events[0], TelnetEvent::MspTrigger(t) if t.name == "a.wav" && !t.is_music));
        assert!(matches!(&outcome.events[1], TelnetEvent::MspTrigger(t) if t.name == "b.mp3" && t.is_music));
    }

    #[test]
    fn test_msp_off_triggers_stop_playback_event() {
        let mut session = TelnetSession::new(TelnetConfig::default());
        let outcome = session.feed(b"!!SOUND(Off)\r\n!!MUSIC(Off)\r\n");
        assert_eq!(outcome.text, b"\r\n\r\n");
        assert_eq!(outcome.events.len(), 2);
        assert!(matches!(&outcome.events[0], TelnetEvent::MspTrigger(t) if t.off && !t.is_music));
        assert!(matches!(&outcome.events[1], TelnetEvent::MspTrigger(t) if t.off && t.is_music));
    }

    #[test]
    fn test_msp_malformed_trigger_no_closing_paren_displays_as_text() {
        // The line ends before a ')' ever appears - per the plan, this must
        // display as ordinary (if ugly) text, not vanish.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let input = b"broken !!SOUND(bell.wav\r\nnext line\r\n";
        let outcome = session.feed(input);
        assert_eq!(outcome.text, input.to_vec());
        assert!(
            outcome.events.iter().all(|e| !matches!(e, TelnetEvent::MspTrigger(_))),
            "malformed trigger must not produce an MspTrigger event: {:?}", outcome.events
        );
    }

    #[test]
    fn test_msp_runaway_unclosed_trigger_eventually_flushes_as_text() {
        // No ')' and no '\n' at all, well past MAX_MSP_TRIGGER_HOLDBACK - the
        // give-up path must flush it as text rather than holding forever.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let mut input = b"!!SOUND(".to_vec();
        input.extend(std::iter::repeat_n(b'x', MAX_MSP_TRIGGER_HOLDBACK + 100));
        let outcome = session.feed(&input);
        assert_eq!(outcome.text, input, "runaway trigger must flush as literal text");
        assert!(outcome.events.is_empty());
        assert!(!session.has_pending_text(), "nothing should still be held back");
    }

    #[test]
    fn test_msp_disabled_config_leaves_trigger_as_plain_text() {
        let mut session = TelnetSession::new(TelnetConfig { msp_enabled: false, ..TelnetConfig::default() });
        let input = b"!!SOUND(bell.wav)\r\n";
        let outcome = session.feed(input);
        assert_eq!(outcome.text, input.to_vec(), "msp_enabled=false must pass the trigger through untouched");
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn test_msp_split_invariance_every_offset() {
        // Highest-value test in the suite, per the plan: a trigger split
        // across two reads (mid-marker, mid-parameter, mid-filename, right
        // at the closing paren, ...) must still be recognised identically to
        // a one-shot feed, at every possible split point - and it can appear
        // mid-line, unlike a whole-line MCP-style message.
        //
        // T2.1: the second and third fixtures put the marker more than
        // MAX_TEXT_HOLDBACK (32) bytes into the line, so a split landing
        // inside the marker itself (e.g. right after "!!SOU") used to flush
        // that fragment as text once the ordinary by-line hold-back gave up
        // — extract_msp_triggers's own incomplete-marker-*prefix* detection
        // (`msp_holdback_start`) is what keeps holding it instead now.
        let fixtures: &[&[u8]] = &[
            b"You see a sign. !!SOUND(bell.wav V=50 L=3 T=ambient)\r\nNext line here.\r\n",
            b"A line comfortably longer than thirty-two bytes before the marker !!SOUND(bell.wav)\r\n",
            b"A line comfortably longer than thirty-two bytes before the marker !!MUSIC(theme.mp3)\r\n",
        ];
        for &input in fixtures {
            let (base_text, base_wire, base_events) = run_one_shot(input);
            assert!(
                base_events.iter().any(|e| matches!(e, TelnetEvent::MspTrigger(_))),
                "sanity: fixture must actually produce an MspTrigger event: {:?}",
                String::from_utf8_lossy(input)
            );
            let mut combinations = 0usize;
            for k in 0..=input.len() {
                combinations += 1;
                let mut session = TelnetSession::new(TelnetConfig::default());
                let mut o1 = session.feed(&input[..k]);
                let o2 = session.feed(&input[k..]);
                o1.text.extend_from_slice(&o2.text);
                o1.wire.extend_from_slice(&o2.wire);
                o1.events.extend(o2.events);
                assert_eq!(
                    o1.text, base_text,
                    "fixture {:?}, split at {k}: text", String::from_utf8_lossy(input)
                );
                assert_eq!(
                    o1.wire, base_wire,
                    "fixture {:?}, split at {k}: wire", String::from_utf8_lossy(input)
                );
                assert_eq!(
                    o1.events, base_events,
                    "fixture {:?}, split at {k}: events", String::from_utf8_lossy(input)
                );
            }
            assert!(combinations > 50, "expected many split-point combinations, got {combinations}");
        }
    }

    #[test]
    fn test_msp_marker_split_mid_marker_across_two_feeds() {
        // T2.1's direct case: the read boundary falls inside the marker
        // itself ("!!SOU" | "ND(bell.wav)\r\n"), not merely inside the
        // parameters. Before the fix, "!!SOU" wasn't recognised as the start
        // of anything (the loop's `starts_with` check needs the full 8-byte
        // marker) and got flushed as ordinary text on the first feed, so the
        // second feed's "ND(bell.wav)\r\n" matched no marker at all.
        let mut session = TelnetSession::new(TelnetConfig::default());
        let o1 = session.feed(b"Look !!SOU");
        let o2 = session.feed(b"ND(bell.wav)\r\n");

        let mut text = o1.text.clone();
        text.extend_from_slice(&o2.text);
        assert!(
            !text.windows(5).any(|w| w == b"!!SOU"),
            "marker fragment must not leak into displayed text: {:?}",
            String::from_utf8_lossy(&text)
        );
        assert_eq!(text, b"Look \r\n");

        let mut events = o1.events;
        events.extend(o2.events);
        assert_eq!(events.len(), 1, "expected exactly one MspTrigger, got {events:?}");
        assert!(matches!(&events[0], TelnetEvent::MspTrigger(t) if t.name == "bell.wav" && !t.is_music));
    }

    #[test]
    fn test_ordinary_prompt_ending_in_bang_split_before_ga_still_yields_whole_prompt() {
        // T2.1 makes a lone trailing '!' count as a possible MSP marker
        // prefix (see msp_holdback_start's min_prefix doc comment) so it
        // survives a chunk boundary. That must not regress the *ordinary*
        // GA-prompt case: "Enter your name!" arriving in one read, then
        // IAC GA in the next, must still produce the whole prompt - not just
        // the trailing "!" that happens to look like a marker's start (see
        // feed()'s hold-decision comment / D2 in the plan).
        let mut input = b"Enter your name!".to_vec();
        input.extend_from_slice(&[TELNET_IAC, TELNET_GA]);
        let (_, _, base_events) = run_one_shot(&input);
        assert!(
            base_events.contains(&TelnetEvent::Prompt(b"Enter your name!".to_vec())),
            "sanity: one-shot feed must produce the whole prompt: {base_events:?}"
        );

        for k in 0..=input.len() {
            let mut session = TelnetSession::new(TelnetConfig::default());
            let o1 = session.feed(&input[..k]);
            let o2 = session.feed(&input[k..]);
            let mut events = o1.events;
            events.extend(o2.events);
            assert_eq!(events, base_events, "split at {k}");
        }
    }

    // ==================================================================
    // T2.2 — idle_release_len and its helpers (D2 in the plan).
    // ==================================================================

    #[test]
    fn test_incomplete_utf8_tail_len_cases() {
        assert_eq!(incomplete_utf8_tail_len(b"caf\xc3"), 1, "2-byte lead with 0 continuation bytes");
        assert_eq!(incomplete_utf8_tail_len(b"caf\xc3\xa9"), 0, "complete 2-byte sequence");
        assert_eq!(incomplete_utf8_tail_len(b"x \xff y"), 0, "literal 0xFF is never a valid UTF-8 lead byte");
        assert_eq!(incomplete_utf8_tail_len(b"hello"), 0, "pure ASCII");
        assert_eq!(incomplete_utf8_tail_len(b""), 0, "empty");
        assert_eq!(incomplete_utf8_tail_len(&[0xE0]), 1, "3-byte lead with 0 continuation bytes");
        assert_eq!(incomplete_utf8_tail_len(&[0xE0, 0x80]), 2, "3-byte lead with 1 continuation byte");
        assert_eq!(incomplete_utf8_tail_len(&[0xE0, 0x80, 0x80]), 0, "complete 3-byte sequence");
    }

    #[test]
    fn test_unterminated_escape_start_cases() {
        assert_eq!(unterminated_escape_start(b"prompt> \x1b[3"), Some(8), "incomplete CSI");
        assert_eq!(unterminated_escape_start(b"\x1b[0m"), None, "complete CSI");
        assert_eq!(unterminated_escape_start(b"no escape here"), None);
        assert_eq!(unterminated_escape_start(b"trailing esc \x1b"), Some(13), "lone trailing ESC");
    }

    #[test]
    fn test_msp_holdback_start_cases() {
        assert_eq!(msp_holdback_start(b"Look !!SOUND(bell", 2), Some(5), "open marker, no ')' yet");
        assert_eq!(msp_holdback_start(b"x !!SOU", 2), Some(2), "trailing marker prefix");
        assert_eq!(msp_holdback_start(b"Hello!", 2), None, "lone '!' below min_prefix 2 is not held");
        assert_eq!(msp_holdback_start(b"Hello!", 1), Some(5), "lone '!' held when min_prefix is 1");
        assert_eq!(
            msp_holdback_start(b"broken !!SOUND(bell.wav\r\nmore\r\n", 1), None,
            "a marker already resolved as malformed (newline, no ')') must not be held forever"
        );
    }

    #[test]
    fn test_idle_release_len_cases() {
        // Every case named in the plan for T2.2's idle_release_len.
        assert_eq!(idle_release_len(b"prompt> \x1b[3"), 8, "incomplete CSI held, prompt text released");
        assert_eq!(idle_release_len(b"caf\xc3"), 3, "incomplete UTF-8 tail held");
        assert_eq!(idle_release_len(b"x \xff y"), 5, "literal 0xFF never held");
        assert_eq!(idle_release_len(b"Look !!SOUND(bell"), 5, "open MSP marker held");
        assert_eq!(idle_release_len(b"Hello!"), 6, "lone trailing '!' after a pause is an ordinary prompt");
        assert_eq!(idle_release_len(b"x !!SOU"), 2, "MSP marker prefix held");
        assert_eq!(idle_release_len(b"\x1b[0m"), 4, "complete CSI released in full");
    }

    #[test]
    fn test_msp_split_invariance_byte_at_a_time() {
        // Same property, finer grain - feed the whole fixture one byte at a
        // time and confirm the accumulated events still match a one-shot
        // feed exactly.
        let input = b"!!SOUND(bell.wav V=50)\r\n";
        let (_, _, base_events) = run_one_shot(input);
        let mut session = TelnetSession::new(TelnetConfig::default());
        let mut events = Vec::new();
        for b in input {
            events.extend(session.feed(std::slice::from_ref(b)).events);
        }
        assert_eq!(events, base_events);
    }

}
