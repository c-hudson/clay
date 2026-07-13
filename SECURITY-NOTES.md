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
