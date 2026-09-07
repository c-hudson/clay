// The one async telnet writer (plan Phase 4, Job 13 of
// investigate-differences-between-tinyfugu-fluffy-stallman.md). Mirrors
// `telnet_reader.rs`'s Phase 2 consolidation of the read side:
// `spawn_telnet_writer` is now the sole production writer task — Job 12's
// handover found thirteen hand-copied writer loops (one next to each of the
// thirteen reader-spawn sites in main.rs/daemon.rs/commands.rs, plus
// testharness.rs), each an almost-identical `match cmd { Text | Raw |
// Shutdown }` block. All fourteen (thirteen production + testharness.rs)
// migrated onto this function.
//
// # Anti-drift
//
// Design commitment 5 sealed the read side by making the *old* primitive
// (`process_telnet`/`find_safe_split_point`) private — there is no equivalent
// old primitive to seal here, because the write side was never a shared
// function in the first place, just the same block of code typed out
// fourteen times by hand. The lock used instead: `spawn_telnet_writer` is the
// ONLY place in the codebase that constructs an `mpsc::channel::<WriteCommand>`
// — channel creation is entirely internal, and the function hands the caller
// back only the `Sender` half. A production `mpsc::Receiver<WriteCommand>` is
// therefore never constructible anywhere else: there is no channel for a
// fourteenth hand-rolled consumption loop to read from. Writing one requires
// reimplementing channel creation *and* the match arms — at which point it
// is not silent drift, it is visibly a second copy of this file, which is
// the bar design commitment 5 set for the read side too ("structurally
// impossible", not "discouraged by convention"). Test code is free to build
// its own throwaway `mpsc::channel::<WriteCommand>` for a canned-script
// harness (as `telnet_reader.rs`'s own tests do) — that observes what a
// production `spawn_telnet_writer` call would receive, it does not compete
// with one.
//
// # Encoding switch design (plan Job 13, Phase 4 / 4.4, finding 7)
//
// `WriteCommand::SetEncoding` (`telnet.rs`) is sent through this same
// channel rather than the encoding living as shared mutable state read by
// the writer (e.g. an `Arc<Mutex<Encoding>>` on `World`, polled or pushed on
// every write). Two reasons, both from the plan:
//
// - **Ordering is right by construction.** `mpsc` is FIFO: any `Text`
//   command already queued ahead of a `SetEncoding` — typed by the user
//   while a CHARSET negotiation was still in flight — is necessarily
//   *processed* by this loop before the `SetEncoding` is, so it is written
//   in the old encoding, and everything queued after switches. A shared-cell
//   design would need its own synchronization to guarantee the writer reads
//   the new value only after every byte queued under the old one has already
//   gone out — this gets that "for free" from the channel's own ordering
//   guarantee, no separate proof obligation.
// - **No lock on the hot path.** Every `Text`/`Raw` write already goes
//   through this same channel; a shared cell would add a lock acquisition
//   (or at least an atomic load) to every single outbound line for a value
//   that changes at most a handful of times per connection (once, in the
//   overwhelmingly common case). Threading the value through as `let mut
//   encoding` local to this task's loop costs nothing extra.
//
// The alternative — `Arc<Mutex<Encoding>>` (or an `AtomicU8` discriminant)
// shared between `App`, the reader, and the writer — was considered and
// rejected for the ordering reason above: `App::handle_telnet_event` runs on
// a different task than this writer loop, so "set the shared cell, then hope
// the writer's next write picks it up after, not before, whatever's still
// queued ahead of it" is exactly the kind of race the in-band command
// avoids by construction.

use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::encoding::Encoding;
use crate::telnet::{push_escaped, StreamWriter, WriteCommand};

/// Matches the `mpsc::channel::<WriteCommand>(100)` capacity every one of
/// the pre-Job-13 writer loops used.
const WRITE_CHANNEL_CAPACITY: usize = 100;

/// Spawn the one async telnet writer task for a connection and return the
/// `Sender` half callers use to queue `WriteCommand`s — `Text` (a typed
/// command line: encoded per the currently-active `Encoding`, IAC-escaped,
/// then terminated with `\r\n`), `Raw` (already-fully-formed wire bytes —
/// telnet negotiation replies and subnegotiations — written verbatim, no
/// encoding or escaping applied a second time), `SetEncoding` (switches the
/// active encoding for every `Text` command from this point on; see the
/// module doc comment for why this travels through the channel rather than
/// shared state), and `Shutdown` (a graceful `AsyncWrite::shutdown` — a real
/// TCP FIN / TLS close_notify, not just dropping the write half — matching
/// the richest of the pre-Job-13 loops rather than the ones that merely
/// `break`).
///
/// `initial_encoding` seeds the encoding used before any `SetEncoding`
/// arrives — callers pass the world's `effective_encoding()` at connect
/// time, which is the explicit per-world setting for a fresh connection, or
/// a previously-negotiated CHARSET encoding restored across a hot reload
/// (`negotiated_encoding` survives reload/crash recovery — see
/// `persistence.rs`), never a bare default that would silently discard
/// either.
pub fn spawn_telnet_writer(
    mut write_half: StreamWriter,
    initial_encoding: Encoding,
) -> mpsc::Sender<WriteCommand> {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(WRITE_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        let mut encoding = initial_encoding;
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                WriteCommand::Text(text) => {
                    // Encode first, then IAC-escape the *encoded* bytes: a
                    // Latin1/Fansi high-byte character can encode to a
                    // genuine 0xFF, which a UTF-8 &str could never produce
                    // (finding 6's "becomes load-bearing the moment any
                    // builder takes raw bytes" - this is that moment for the
                    // write side). \r\n is appended raw afterward: those two
                    // bytes (0x0D, 0x0A) are identical in every encoding
                    // Clay speaks and are line framing, not payload, so they
                    // are never escaped (see push_escaped's doc comment on
                    // what counts as payload vs. framing).
                    let mut bytes = Vec::with_capacity(text.len() + 2);
                    push_escaped(&mut bytes, &encoding.encode(&text));
                    bytes.extend_from_slice(b"\r\n");
                    if write_half.write_all(&bytes).await.is_err() {
                        break;
                    }
                    let _ = write_half.flush().await;
                }
                WriteCommand::Raw(raw) => {
                    // Already a complete, correctly-escaped wire payload
                    // (telnet negotiation, GMCP/MSDP/CHARSET/NAWS
                    // subnegotiations) - written verbatim.
                    if write_half.write_all(&raw).await.is_err() {
                        break;
                    }
                    let _ = write_half.flush().await;
                }
                WriteCommand::SetEncoding(new_encoding) => {
                    encoding = new_encoding;
                }
                WriteCommand::Shutdown => {
                    let _ = write_half.shutdown().await;
                    break;
                }
            }
        }
    });
    cmd_tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    /// Spawn a writer over an in-process `tokio::io::duplex` pair standing in
    /// for the socket, mirroring `telnet_reader.rs`'s own `Harness` — `server`
    /// is the peer end a real MUD/proxy socket would be; reading from it
    /// observes exactly the bytes production code would put on the wire.
    fn spawn(initial_encoding: Encoding) -> (mpsc::Sender<WriteCommand>, tokio::io::DuplexStream) {
        let (client, server) = tokio::io::duplex(4096);
        let (_read_half, write_half) = tokio::io::split(client);
        let cmd_tx = spawn_telnet_writer(StreamWriter::Duplex(write_half), initial_encoding);
        (cmd_tx, server)
    }

    /// Read exactly `n` bytes from `server`, timing out rather than hanging
    /// forever if the writer produced fewer (a test bug, not a real stall).
    async fn read_n(server: &mut tokio::io::DuplexStream, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        tokio::time::timeout(Duration::from_secs(5), server.read_exact(&mut buf))
            .await
            .expect("timed out waiting for writer output")
            .expect("read_exact failed");
        buf
    }

    // ==================================================================
    // One test per distinct target shape: the three `Encoding`s a `Text`
    // command can be written under. Unlike the reader's `TelnetTarget`
    // (`World` vs `Multiuser`, which changes *which* `AppEvent` comes out),
    // the writer is oblivious to what kind of connection it's on — its only
    // shape-changing axis is which encoding is active when a `Text` command
    // is processed, so that's what these three mirror.
    // ==================================================================

    #[tokio::test]
    async fn writer_utf8_shape_sends_text_as_utf8_plus_crlf() {
        let (cmd_tx, mut server) = spawn(Encoding::Utf8);
        cmd_tx.send(WriteCommand::Text("look".to_string())).await.unwrap();
        let mut expected = b"look".to_vec();
        expected.extend_from_slice(b"\r\n");
        assert_eq!(read_n(&mut server, expected.len()).await, expected);

        // A non-ASCII character stays multi-byte UTF-8 - Utf8::encode is the
        // identity, byte for byte, exactly like the pre-Job-13 `.as_bytes()`
        // every writer used to call directly.
        cmd_tx.send(WriteCommand::Text("café".to_string())).await.unwrap();
        let mut expected2 = "café".as_bytes().to_vec();
        expected2.extend_from_slice(b"\r\n");
        assert_eq!(read_n(&mut server, expected2.len()).await, expected2);
    }

    #[tokio::test]
    async fn writer_latin1_shape_encodes_high_byte_and_iac_escapes_0xff() {
        let (cmd_tx, mut server) = spawn(Encoding::Latin1);
        // 'é' (U+00E9) is one Latin1 byte (0xE9), not two UTF-8 bytes - proof
        // the writer is really encoding, not just relabeling UTF-8 as Latin1.
        cmd_tx.send(WriteCommand::Text("café".to_string())).await.unwrap();
        assert_eq!(read_n(&mut server, 6).await, vec![b'c', b'a', b'f', 0xE9, b'\r', b'\n']);

        // 'ÿ' (U+00FF) encodes to the literal byte 0xFF in Latin1 (see
        // encoding.rs's decode: 0xA0-0xFF is a direct cast) - a real IAC byte
        // that a UTF-8 &str could never have produced. This is the case the
        // plan predicted would make push_escaped load-bearing on the write
        // side: end to end through the writer, 0xFF must arrive doubled
        // (0xFF 0xFF), or the far end's own telnet parser would misread it
        // as the start of a command sequence instead of a character.
        cmd_tx.send(WriteCommand::Text("\u{FF}".to_string())).await.unwrap();
        assert_eq!(read_n(&mut server, 4).await, vec![0xFF, 0xFF, b'\r', b'\n']);
    }

    #[tokio::test]
    async fn writer_fansi_shape_encodes_high_byte_and_iac_escapes_0xff() {
        let (cmd_tx, mut server) = spawn(Encoding::Fansi);
        // NBSP (U+00A0) is CP437 byte 0xFF (encoding.rs: `255 => NBSP`) - a
        // second, independent route (Fansi rather than Latin1) to a real
        // outbound 0xFF, proving the escaping isn't specific to one encoding's
        // table.
        cmd_tx.send(WriteCommand::Text("\u{A0}".to_string())).await.unwrap();
        assert_eq!(read_n(&mut server, 4).await, vec![0xFF, 0xFF, b'\r', b'\n']);

        // An ordinary CP437 box-drawing character (0xB3, '│') round-trips as
        // a single unescaped byte.
        cmd_tx.send(WriteCommand::Text("\u{2502}".to_string())).await.unwrap();
        assert_eq!(read_n(&mut server, 3).await, vec![0xB3, b'\r', b'\n']);
    }

    // ==================================================================
    // Unrepresentable character: substitutes, does not panic, and the
    // substitute byte (ASCII '?') needs no IAC escaping of its own.
    // ==================================================================

    #[tokio::test]
    async fn writer_substitutes_unrepresentable_character_without_panicking() {
        let (cmd_tx, mut server) = spawn(Encoding::Latin1);
        cmd_tx.send(WriteCommand::Text("HP:100日MP:50".to_string())).await.unwrap();
        let mut expected = b"HP:100?MP:50".to_vec();
        expected.extend_from_slice(b"\r\n");
        assert_eq!(read_n(&mut server, expected.len()).await, expected);
    }

    // ==================================================================
    // The ordering property (plan Job 13's actual design question): text
    // queued before a mid-session SetEncoding is written in the old
    // encoding, text queued after is written in the new one - proven by
    // relying on nothing but the channel's own FIFO order, exactly as the
    // module doc comment argues.
    // ==================================================================

    #[tokio::test]
    async fn set_encoding_applies_only_to_text_queued_after_it() {
        let (cmd_tx, mut server) = spawn(Encoding::Utf8);

        // 'é' under Utf8: two bytes (0xC3 0xA9).
        cmd_tx.send(WriteCommand::Text("é".to_string())).await.unwrap();
        // Queue the switch and the next Text back to back, with no await in
        // between on the receiving end - if ordering were anything other
        // than strict FIFO, this is where it would show.
        cmd_tx.send(WriteCommand::SetEncoding(Encoding::Latin1)).await.unwrap();
        // Same character, now under Latin1: one byte (0xE9).
        cmd_tx.send(WriteCommand::Text("é".to_string())).await.unwrap();

        // First message: old encoding (Utf8), two-byte 'é' plus CRLF.
        assert_eq!(read_n(&mut server, 4).await, vec![0xC3, 0xA9, b'\r', b'\n']);
        // Second message: new encoding (Latin1), one-byte 'é' plus CRLF -
        // proves the switch took effect for everything queued after it.
        assert_eq!(read_n(&mut server, 3).await, vec![0xE9, b'\r', b'\n']);
    }

    #[tokio::test]
    async fn set_encoding_can_switch_more_than_once() {
        let (cmd_tx, mut server) = spawn(Encoding::Latin1);
        cmd_tx.send(WriteCommand::Text("\u{FF}".to_string())).await.unwrap(); // 0xFF (escaped)
        cmd_tx.send(WriteCommand::SetEncoding(Encoding::Fansi)).await.unwrap();
        cmd_tx.send(WriteCommand::Text("\u{2502}".to_string())).await.unwrap(); // 0xB3
        cmd_tx.send(WriteCommand::SetEncoding(Encoding::Utf8)).await.unwrap();
        cmd_tx.send(WriteCommand::Text("é".to_string())).await.unwrap(); // 0xC3 0xA9

        assert_eq!(read_n(&mut server, 4).await, vec![0xFF, 0xFF, b'\r', b'\n']);
        assert_eq!(read_n(&mut server, 3).await, vec![0xB3, b'\r', b'\n']);
        assert_eq!(read_n(&mut server, 4).await, vec![0xC3, 0xA9, b'\r', b'\n']);
    }

    // ==================================================================
    // Raw passthrough: never encoded, never re-escaped (it's already a
    // complete wire payload built by one of the telnet.rs builders, which
    // already escaped whatever needed it).
    // ==================================================================

    #[tokio::test]
    async fn writer_raw_command_bypasses_encoding_and_escaping() {
        let (cmd_tx, mut server) = spawn(Encoding::Latin1);
        // A raw payload that already contains an unescaped-looking single
        // 0xFF deliberately, to prove Raw is never run back through
        // push_escaped a second time (double-escaping would corrupt an
        // already-correct builder output).
        let raw = vec![0xFF, 0xFB, 0x01]; // IAC WILL ECHO, single IAC as framing
        cmd_tx.send(WriteCommand::Raw(raw.clone())).await.unwrap();
        assert_eq!(read_n(&mut server, raw.len()).await, raw);
    }

    // ==================================================================
    // Shutdown: a real graceful close (AsyncWrite::shutdown), not just
    // dropping the write half - matches the richest of the pre-Job-13
    // loops. On a duplex pair this surfaces as the peer's next read seeing
    // EOF.
    // ==================================================================

    #[tokio::test]
    async fn shutdown_closes_the_connection_gracefully() {
        let (cmd_tx, mut server) = spawn(Encoding::Utf8);
        cmd_tx.send(WriteCommand::Shutdown).await.unwrap();

        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(5), server.read(&mut buf))
            .await
            .expect("timed out waiting for EOF")
            .expect("read failed");
        assert_eq!(n, 0, "shutdown must close the stream (peer sees EOF)");
    }

    // ==================================================================
    // Anti-drift claim, made concrete: spawn_telnet_writer is the only
    // production constructor of an mpsc::channel::<WriteCommand> - checked
    // here the same way telnet_reader.rs's to_app_event exhaustiveness is
    // checked, by exercising the actual guarantee rather than only asserting
    // it in prose. This isn't a compiler-checked fact the way "no wildcard
    // arm" is, but the return type is: spawn_telnet_writer's signature hands
    // back a Sender only, so this test is really just pinning that signature.
    // ==================================================================

    #[test]
    fn spawn_telnet_writer_returns_sender_only() {
        fn assert_sender_only(_f: fn(StreamWriter, Encoding) -> mpsc::Sender<WriteCommand>) {}
        assert_sender_only(spawn_telnet_writer);
    }
}
