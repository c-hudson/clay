# Minimal MUD that emits numbered lines with a trailing prompt, plus blank/ANSI-only lines.
#
# Default behaviour is unchanged from the original 25-line version: `fake-mud-server.py <port>`
# emits plain numbered lines and a partial prompt, and answers nothing.
#
# Optional flags make it negotiate and emit structured data, which is what the status-display
# work needs in order to verify anything end to end:
#   --gmcp   answer DO GMCP with WILL GMCP, then send Char.Vitals / Char.Status periodically
#            -- but ONLY the Char.* sub-packages the client actually declared support for
#            via Core.Supports.Set (mud-status-display.md Job 6: a real spec-following
#            server never sends a package nobody asked for, and the previous unconditional
#            version of this file masked exactly that gap in Clay itself).
#   --msdp   answer DO MSDP with WILL MSDP, then send HEALTH / MANA / etc. periodically --
#            but ONLY the variables the client has actually REPORTed, same reasoning. A
#            `LIST REPORTABLE_VARIABLES` request gets an immediate reply naming everything
#            below that this server is willing to report.
#   --mssp   answer DO MSSP with WILL MSSP and send one MSSP block on connect
#   --quiet  suppress the numbered filler lines (structured data only)
#
# Bug-hunt scenario flags (plan: bug hunt after v1.6.1, Verification section). Each is a
# one-shot hostile or awkward behaviour a real server could exhibit; default off.
#   --mccp2-garbage      answer DO MCCP2 with WILL, send IAC SB MCCP2 IAC SE, then stream
#                        non-zlib bytes forever (T1.1: Clay must disconnect, not retain)
#   --mccp2-bomb         same activation, then a real zlib stream inflating to 16 MiB of
#                        zeros (T1.14: bomb guard must disconnect, not desync)
#   --mccp2-unrequested  answer DO MCCP2 with WONT, then send IAC SB MCCP2 IAC SE anyway
#                        followed by plaintext (T1.2: Clay must ignore it and stay up)
#   --mssp-hostile       MSSP NAME value carrying ESC sequences (T1.5: /mssp must not
#                        reach the terminal unsanitized)
#   --msp-split=MS       send "Hello !!SOU", sleep MS ms, send "ND(bell.wav)\r\n"
#                        (T2.1/T2.2: the trigger must still be recognised, never shown)
#   --csi-split=MS       send "prompt> \x1b[3", sleep MS ms, send "1m" (T2.2: no stray "[3")
#   --prompt-bang        send "Enter your name!" with no newline and no GA (T2.2: the "!"
#                        must be displayed after the idle flush, not held as an MSP prefix)
#   --will-echo          send IAC WILL ECHO before the prompt (T1.8: masking survives
#                        /reload)
#
# Note: payload bytes are not IAC-doubled here. A real server must double a 0xFF inside a
# subnegotiation; none of the fixtures below contain one, so this stays correct for its purpose.
import socket, threading, time, sys, json, argparse, zlib

IAC, SE, SB, WILL, WONT, DO, DONT = 255, 240, 250, 251, 252, 253, 254
OPT_TTYPE, OPT_NAWS, OPT_CHARSET, OPT_MSDP, OPT_MSSP = 24, 31, 42, 69, 70
OPT_MCCP2, OPT_MSP, OPT_GMCP, OPT_EOR, OPT_SGA, OPT_ECHO = 86, 90, 201, 25, 3, 1
MSDP_VAR, MSDP_VAL, MSDP_TABLE_OPEN, MSDP_TABLE_CLOSE, MSDP_ARRAY_OPEN, MSDP_ARRAY_CLOSE = 1, 2, 3, 4, 5, 6

# The variables this fake server is willing to report if asked - answered verbatim to a
# `LIST REPORTABLE_VARIABLES` request, and the only names the periodic push will ever
# include (and then only once actually REPORTed - see `handle_msdp_sb`).
MSDP_REPORTABLE = ["HEALTH", "HEALTH_MAX", "MANA", "MANA_MAX", "LEVEL", "ROOM_NAME"]

ap = argparse.ArgumentParser()
ap.add_argument("port", type=int)
ap.add_argument("--gmcp", action="store_true")
ap.add_argument("--msdp", action="store_true")
ap.add_argument("--mssp", action="store_true")
ap.add_argument("--quiet", action="store_true")
ap.add_argument("--mccp2-garbage", action="store_true")
ap.add_argument("--mccp2-bomb", action="store_true")
ap.add_argument("--mccp2-unrequested", action="store_true")
ap.add_argument("--mssp-hostile", action="store_true")
ap.add_argument("--msp-split", type=int, default=None, metavar="MS")
ap.add_argument("--csi-split", type=int, default=None, metavar="MS")
ap.add_argument("--prompt-bang", action="store_true")
ap.add_argument("--will-echo", action="store_true")
ap.add_argument("--offer", action="store_true", help="server-INITIATED negotiation: send WILL/DO offers on connect (models a real MUD; lets a fully-reactive client respond)")
ap.add_argument("--rxlog", default=None, metavar="PATH", help="append hex of every byte received from the client to PATH (to prove a reactive client sends nothing unsolicited)")
args = ap.parse_args()

s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', args.port)); s.listen(5)
print(f"mud listening on {args.port}", flush=True)


def sb(option, payload):
    return bytes([IAC, SB, option]) + payload + bytes([IAC, SE])


def gmcp(package, obj):
    return sb(OPT_GMCP, package.encode() + b" " + json.dumps(obj).encode())


def msdp(pairs):
    body = b""
    for name, value in pairs:
        body += bytes([MSDP_VAR]) + name.encode() + bytes([MSDP_VAL]) + str(value).encode()
    return sb(OPT_MSDP, body)


def msdp_array_reply(name, values):
    """Build a VAR <name> VAL <ARRAY_OPEN> (VAL <elem>)* <ARRAY_CLOSE> reply - the shape
    a real MSDP server uses to answer `LIST REPORTABLE_VARIABLES` (and its siblings)."""
    body = bytes([MSDP_VAR]) + name.encode() + bytes([MSDP_VAL, MSDP_ARRAY_OPEN])
    for v in values:
        body += bytes([MSDP_VAL]) + v.encode()
    body += bytes([MSDP_ARRAY_CLOSE])
    return sb(OPT_MSDP, body)


def mssp(pairs):
    body = b""
    for name, value in pairs:
        body += bytes([MSDP_VAR]) + name.encode() + bytes([MSDP_VAL]) + str(value).encode()
    return sb(OPT_MSSP, body)


def supports(state, want):
    """True if the client's most recent Core.Supports.Set declared a package family
    covering `want` (case-insensitively): an exact match, or a shorter family name that
    `want` is a dotted sub-package of (declaring "Char 1" covers "Char.Vitals"). Mirrors
    what a real GMCP server does: only push a package the client actually asked for."""
    want_lower = want.lower()
    for pkg in state.get("gmcp_supports", []):
        if not isinstance(pkg, str) or not pkg.strip():
            continue
        name = pkg.split()[0].lower()  # strip the trailing " <version>"
        if name == want_lower or want_lower.startswith(name + "."):
            return True
    return False


def handle_gmcp_sb(payload, state):
    """Parse an inbound GMCP subnegotiation from the client and remember
    Core.Supports.Set, so the periodic push only ever sends Char.* the client
    actually declared support for."""
    package, _, json_bytes = payload.partition(b" ")
    if package.decode(errors="replace").lower() == "core.supports.set":
        try:
            declared = json.loads(json_bytes.decode(errors="replace"))
        except (json.JSONDecodeError, UnicodeDecodeError):
            declared = []
        state["gmcp_supports"] = declared if isinstance(declared, list) else []


def parse_simple_msdp(payload):
    """Parse a flat `MSDP_VAR name MSDP_VAL value` pair - the only shape Clay's own
    build_msdp_request/build_msdp_set ever send on the wire. Not a general MSDP parser
    (no array/table support): client requests don't need one here."""
    if not payload or payload[0] != MSDP_VAR:
        return None, None
    i = 1
    start = i
    while i < len(payload) and payload[i] != MSDP_VAL:
        i += 1
    name = payload[start:i].decode(errors="replace")
    if i >= len(payload):
        return name, None
    value = payload[i + 1:].decode(errors="replace")
    return name, value


def handle_msdp_sb(payload, state):
    """Parse an inbound MSDP subnegotiation from the client: LIST REPORTABLE_VARIABLES
    gets an immediate array reply, REPORT/UNREPORT maintain the per-connection reported
    set that gates the periodic push (see the main loop below) - the whole point of
    Job 6's "make the test server strict" requirement."""
    name, value = parse_simple_msdp(payload)
    if name is None:
        return b""
    verb = name.upper()
    reported = state.setdefault("msdp_reported", set())
    if verb == "REPORT" and value:
        reported.add(value.upper())
    elif verb == "UNREPORT" and value:
        reported.discard(value.upper())
    elif verb == "LIST" and value and value.upper() == "REPORTABLE_VARIABLES":
        return msdp_array_reply("REPORTABLE_VARIABLES", MSDP_REPORTABLE)
    return b""


def negotiate(c, data, state):
    """Answer negotiation. Clay initiates now, so this must reply or nothing turns on."""
    out = b""
    i = 0
    while i < len(data):
        if data[i] != IAC or i + 1 >= len(data):
            i += 1
            continue
        cmd = data[i + 1]
        if cmd == SB:
            option = data[i + 2] if i + 2 < len(data) else None
            payload_start = i + 3
            j = payload_start
            while j + 1 < len(data) and not (data[j] == IAC and data[j + 1] == SE):
                j += 1
            payload = data[payload_start:j]
            i = j + 2
            if option == OPT_GMCP and args.gmcp:
                handle_gmcp_sb(payload, state)
            elif option == OPT_MSDP and args.msdp:
                out += handle_msdp_sb(payload, state)
            continue
        if cmd in (WILL, WONT, DO, DONT) and i + 2 < len(data):
            opt = data[i + 2]
            if cmd == DO:                  # peer asks us to enable `opt`
                # A DO for something WE already offered via WILL (e.g. ECHO in the
                # --will-echo scenario) is the client confirming, not requesting -
                # replying WONT here would tear the option straight back down.
                if opt in state.get("offered_will", set()):
                    i += 3
                    continue
                enabled = ((opt == OPT_GMCP and args.gmcp)
                           or (opt == OPT_MSDP and args.msdp)
                           or (opt == OPT_MSSP and args.mssp)
                           or (opt == OPT_MCCP2 and (args.mccp2_garbage or args.mccp2_bomb)))
                out += bytes([IAC, WILL if enabled else WONT, opt])
                if enabled:
                    state.setdefault("on", set()).add(opt)
            elif cmd == WILL:              # peer offers to do `opt` itself
                wanted = opt in (OPT_TTYPE, OPT_NAWS)
                out += bytes([IAC, DO if wanted else DONT, opt])
            i += 3
            continue
        i += 2
    return out


def rxlog(data):
    if args.rxlog and data:
        with open(args.rxlog, "ab") as f:
            f.write(data)

def server_offer():
    """Server-INITIATED negotiation (the standard MUD model): the server announces
    its options; a reactive client responds. TTYPE/NAWS are asked via DO; GMCP/MSDP/
    MSSP/MCCP2/MSP are announced via WILL."""
    out = bytes([IAC, DO, OPT_TTYPE, IAC, DO, OPT_NAWS])
    if args.gmcp: out += bytes([IAC, WILL, OPT_GMCP])
    if args.msdp: out += bytes([IAC, WILL, OPT_MSDP])
    if args.mssp: out += bytes([IAC, WILL, OPT_MSSP])
    return out

def handle(c):
    state = {}
    try:
        c.settimeout(0.2)
        n, hp, sent_mssp = 0, 1000, False
        if args.offer:
            # Give a reactive client a beat to prove it stays silent first, then offer.
            time.sleep(0.5)
            c.sendall(server_offer())
        for burst in range(400):
            try:
                data = c.recv(4096)
                if data:
                    rxlog(data)
                    reply = negotiate(c, data, state)
                    if reply:
                        c.sendall(reply)
            except socket.timeout:
                pass
            except OSError:
                break

            on = state.get("on", set())
            chunk = b""

            if args.mssp and OPT_MSSP in on and not sent_mssp:
                name = "Fake Test MUD"
                if args.mssp_hostile:
                    # Screen clear + cursor home + title rewrite: if any of this reaches the
                    # terminal from /mssp, the sanitizer is missing.
                    name = "\x1b[2J\x1b[1;1H\x1b]0;pwned\x07Fake Test MUD"
                chunk += mssp([("NAME", name), ("PLAYERS", 3),
                               ("UPTIME", int(time.time())), ("CODEBASE", "fake-mud-server.py")])
                sent_mssp = True

            # ---- one-shot bug-hunt scenarios, fired on the second burst so Clay's
            # opening negotiation has been answered first ----
            if burst == 1 and not state.get("scenario_done"):
                state["scenario_done"] = True
                if args.will_echo:
                    state.setdefault("offered_will", set()).add(OPT_ECHO)
                    c.sendall(bytes([IAC, WILL, OPT_ECHO]) + b"Password: ")
                if args.prompt_bang:
                    c.sendall(b"Enter your name!")          # no newline, no GA
                if args.msp_split is not None:
                    c.sendall(b"Hello !!SOU")
                    time.sleep(args.msp_split / 1000.0)
                    c.sendall(b"ND(bell.wav)\r\n")
                if args.csi_split is not None:
                    c.sendall(b"prompt> \x1b[3")
                    time.sleep(args.csi_split / 1000.0)
                    c.sendall(b"1m")
                if args.mccp2_unrequested:
                    # WONT was already answered above (flag not in the enabled set), so
                    # this SB is unrequested. Clay must ignore it and keep showing text.
                    c.sendall(sb(OPT_MCCP2, b"") + b"STILL PLAINTEXT after unrequested SB\r\n")

            # ---- compression takeover: after this, nothing plaintext is sent again ----
            if (args.mccp2_garbage or args.mccp2_bomb) and OPT_MCCP2 in on \
                    and not state.get("compressing"):
                state["compressing"] = True
                c.sendall(chunk + sb(OPT_MCCP2, b""))
                if args.mccp2_bomb:
                    # ~16 KiB on the wire, 16 MiB inflated: trips Clay's 4 MiB bomb guard.
                    c.sendall(zlib.compress(b"\0" * (16 * 1024 * 1024), 9))
                else:
                    # Not zlib. A real inflate error on the very first bytes.
                    import os
                    while True:
                        c.sendall(os.urandom(65536))
                        time.sleep(1.0)
                return

            if not args.quiet:
                for _ in range(5):
                    n += 1
                    chunk += f"MUDLINE {n:04d} the quick brown fox\r\n".encode()
                chunk += b"\x1b[0m\r\n"                       # ANSI-only line
                chunk += b"\r\n"                              # blank line

            # Structured data: vary hp so a gauge visibly moves.
            hp = 1000 if hp <= 100 else hp - 37
            if args.gmcp and OPT_GMCP in on:
                if supports(state, "Char.Vitals"):
                    chunk += gmcp("Char.Vitals", {"hp": str(hp), "maxhp": "1000",
                                                  "mp": "480", "maxmp": "500",
                                                  "ep": "15048", "maxep": "15048"})
                if supports(state, "Char.Status"):
                    chunk += gmcp("Char.Status", {"level": "42", "name": "Testchar"})
                # Must NOT feed the status model - a check that filtering works. Sent
                # unconditionally (not gated on `supports`): this is testing Clay's own
                # client-side Char.*-only filter, not GMCP capability negotiation.
                chunk += gmcp("Room.Info", {"num": "1234", "name": "A Test Room"})
            if args.msdp and OPT_MSDP in on:
                reported = state.get("msdp_reported", set())
                all_vars = [("HEALTH", hp), ("HEALTH_MAX", 1000),
                            ("MANA", 480), ("MANA_MAX", 500),
                            ("LEVEL", 42), ("ROOM_NAME", "A Test Room")]
                pairs = [(name, value) for name, value in all_vars if name in reported]
                if pairs:
                    chunk += msdp(pairs)

            if not args.quiet:
                chunk += f"HP:{hp} MP:50 [{n}]> ".encode()     # trailing partial prompt
            if chunk:
                c.sendall(chunk)
            time.sleep(0.15)
        time.sleep(30)
    except Exception as e:
        print("mud conn ended:", e, flush=True)


while True:
    c, _ = s.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
