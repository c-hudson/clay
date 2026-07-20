# Connection Security — Release Notes

Clay's web server no longer answers unknown callers. Three changes, in order of how
likely they are to affect you.

## 1. The web UI moved to `/clay` (compatibility break)

The web interface is now served only under a stealth path prefix — by default
`http(s)://yourhost:9000/clay/`. Every other path is **silently dropped** for
non-localhost connections: no page, no 404, no response bytes at all. Port scanners
see a dead port instead of a login page.

**What breaks:**
- Bookmarks to `http(s)://yourhost:9000/` stop working from other machines. Use
  `http(s)://yourhost:9000/clay/` instead.
- Old Android APKs (built before knock support) can only connect from IPs on the
  allow list.

**Remedies, if you need the old behavior:**
- Set **Web Path** empty in Web Settings to restore legacy mode (UI at `/`, unknown
  paths get a 404 as before). This makes the server visible to scanners again.
- Or add the device's IP to the **WS Allow List**.
- Or update the Android app.

Localhost is unaffected: the GUI WebView and local browsers keep working at both `/`
and `/clay/` with zero configuration.

Repeatedly probing invalid paths still earns a ban under the existing two-strike rule —
**unless your address is on the WS Allow List.** Once an allow list is configured, the
server drops every non-listed connection before it can ever reach a probe-strike site
(see below), so a probe strike can only ever ban a legitimate, allow-listed caller —
never a scanner. Allow-listed addresses (and localhost) are therefore never banned for
bad paths, stale bookmarks, http/https typos, knock failures, or malformed requests;
they're still silently dropped, they just can't accumulate a ban from it. A bare `*` in
the allow list ("let anyone in") does **not** count as being "specifically listed" for
this purpose — it doesn't grant ban immunity. `/favicon.ico` and `/apple-touch-icon*`
are dropped without a strike for everyone, so browsers can't get you banned by asking
for an icon.

## 2. A configured allow list now hard-drops non-listed IPs at the TCP level

Previously the allow list only gated WebSocket password auth — the web pages were
still served to anyone. Now, when **WS Allow List** is non-empty, connections from
addresses that don't match it are dropped before anything is sent back: no TLS
handshake, no certificate, no HTTP redirect, no bytes. The event is recorded in
`~/.clay/remote.log` as `GATE-DROP` (it deliberately does *not* count as a ban strike).

Allow-list membership does **not** replace authentication — listed hosts still need a
password or auth key.

## 3. Android devices can knock in from anywhere with the auth key

An address that isn't on the allow list has exactly one way in: the Android app proves
possession of the **Auth Key** with an in-band preamble (CLAY-KNOCK v1) before any web
request. A successful knock admits the connection to the **WebSocket only** — the web
UI stays locked for non-listed addresses, and a knocked connection that asks for a page
gets dropped (`KNOCK-HTTP-DENIED`). The knock does not skip login: normal WebSocket auth
still follows.

The Android app knocks automatically whenever it has an auth key, over both `ws` and
`wss`, and falls back to a plain connection against older Clay servers. Regenerating or
revoking the key takes effect immediately, with no restart. A device that knocks
successfully but then fumbles the WebSocket password (e.g. the key was rotated on the
phone but the old key still knocks) is not struck for "not in allow list" — only the
login itself is rejected, so the device isn't banned out of its own recovery path.

**Auth summary:**

| Allow list | Password | Auth key |
|---|---|---|
| Not configured | Accepted from any address | Accepted |
| Configured, address matches | Accepted | Accepted |
| Configured, address doesn't match | Rejected | Accepted (knock → WebSocket only) |

Multiuser mode has no auth key, so unlisted addresses cannot connect there at all.

## Ban rules

- **Path/connection probes** (bad paths, malformed requests, knock failures, TLS
  timeouts, http/https typos): 2 strikes = banned, *except* for localhost and
  allow-listed addresses (see above), which are never banned for these — only silently
  dropped.
- **Failed WebSocket password login**: 5 attempts = banned, for **everyone**, including
  allow-listed addresses — this is the one strike that actually protects something, so
  it doesn't get the allow-list exemption, but it gives more room than 2 for a typo
  before locking a legitimate device out. Only localhost is exempt.
- Bans are in-memory only and last until the server restarts. A successful login clears
  a host's accumulated strikes.

## Debugging

`~/.clay/remote.log` gains these events: `HTTP-DROP` (stealth path probe),
`GATE-DROP` (not on allow list), `GATE-TIMEOUT` (connected, sent nothing),
`TLS-ON-PLAIN` (a browser sent a TLS ClientHello to a plain-HTTP server — logged, never
struck; happens when a browser remembers HTTPS from an earlier `web_secure=true` run),
`KNOCK-OK` / `KNOCK-FAIL` / `KNOCK-BAD-MAGIC`, `KNOCK-HTTP-DENIED` (knocked connection
asked for a page), and `WS-PATH-DROP` (WebSocket upgrade at the wrong path).

`cargo test` never writes to your real `~/.clay/remote.log` — logging is a no-op in test
builds.

Design record, including the wire protocol: `SECURITY-ROADMAP.md`.

---

# Security-audit hardening (v1.1.x)

A follow-up audit tightened several areas beyond the incoming-connection surface above.
The user-visible changes:

## TLS connections are now pinned (trust-on-first-use)

When Clay connects *out* over TLS — to a MUD, to another Clay via the remote console, or
the WebView proxy — it now remembers the server's certificate the first time and checks
it every time after. Previously it accepted any certificate, which meant a network
attacker could silently intercept the connection (including the one that carries your
password and commands).

- **First connection:** silent. Clay records the certificate's fingerprint in
  `~/.clay/known_hosts.dat` and connects.
- **Later connections:** if the certificate is unchanged, silent. If it *changed*, Clay
  **blocks the connection** and shows you the old vs new fingerprint with a "trust new
  certificate" button. Click it only if you know the server's cert was legitimately
  renewed; otherwise it may be an interception attempt.

This works with self-signed certificates (Clay's own server and most MUDs use them), so
there's nothing to configure. To forget a pin, remove its line from
`~/.clay/known_hosts.dat`.

## Other fixes

- **Web client:** text from a MUD can no longer inject scripts into the web/mobile
  interface (an emoji-rendering XSS hole is closed).
- **Multiuser mode:** one user can no longer connect to, or see the server/login details
  of, another user's worlds; and changing a user's password now actually works (it
  previously locked the account out on the next login).
- **File permissions:** all files holding secrets (`secure.key`, `settings.dat`,
  `multiuser.dat`, `key.pem`, `known_hosts.dat`, the reload-state and audit-log files)
  are now owner-only (0600), and `~/.clay` is 0700. The auto-generated TLS private key
  was previously world-readable.
- **Media auto-download:** a MUD can no longer make Clay fetch arbitrary URLs — only
  `http`/`https` media URLs are allowed, and internal/loopback/`file://` targets are
  refused.
- WebSocket frames are size-capped, credential comparisons are constant-time, and a few
  crash-resistance/logging issues were cleaned up.

**Known follow-up:** `cargo audit` flags a few dependencies (`rustls-webpki`, `idna`,
`time`) with advisories whose fixes require a larger dependency/toolchain upgrade,
tracked separately. Clay's certificate pinning does not exercise the vulnerable
`rustls-webpki` code paths.

Design record: `SECURITY-ROADMAP.md` (decision D7).

---

# Always-secure web server, no more Protocol setting

The `/web` settings no longer have a Protocol (Secure/Non-Secure) choice — the server is
now always TLS-encrypted for anyone connecting from outside this machine, automatically.

- **Nothing to configure.** On first use, Clay generates its own self-signed certificate
  and stores it (encrypted) in `settings.dat`, alongside your other settings.
- **Other Clay instances just work.** The remote console, another Clay's WebView, and
  the Android app all use the same trust-on-first-use pinning described above — they
  silently trust the certificate the first time and only ask you to confirm if it later
  changes. There's no extra step for this to work.
- **This machine (localhost) is never encrypted.** The desktop app's own window talks to
  the server over plain, unencrypted loopback — the same machine, so there's no network
  to intercept — which means it never shows you a certificate warning.
- **A plain web browser** connecting remotely will show a one-time "not secure" warning
  the first time, same as any self-signed certificate — that's expected. If you'd rather
  not see that, set **Custom Cert File** to Yes and point it at a CA-signed certificate.
- **The Auth Key field is now read-only** in `/web` settings; use the new **Modify Key**
  button to copy, regenerate, or delete it. Regenerating or deleting takes effect
  immediately, same as before.

Design record: `SECURITY-ROADMAP.md` (decision D8).

---

# Standalone (on-device) Android mode

The Android app can now run its own bundled Clay server on the phone instead of
connecting to one elsewhere — pick "Run on This Phone" at first launch, or switch later
in Settings.

- The on-device server binds **loopback only** (`127.0.0.1`), never the LAN — it's not
  reachable from other devices, so none of the allow-list/knock/ban machinery above
  applies to it or is needed for it.
- Its WebSocket password is a fresh random value generated by the app on every start and
  never leaves the device; there's nothing to configure or remember.
- **Not available in this mode:** hot reload and the TLS proxy (already unsupported on
  Android/Termux generally — see `CLAUDE.md`). Standalone mode has no separate process to
  reload into and no remote MUD connections that would need the outbound TLS proxy.
  Everything else — worlds, actions, TF scripting, scrollback — works the same as remote
  mode.
