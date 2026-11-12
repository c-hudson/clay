# The Web Gateway (clay_web)

`clay_web` is Clay as a public web page onto **one MUD world**. Anyone who opens the page is
connected to that world at once, with a connection of their own, and types straight into
the game; the game learns who they are from a **First Command** such as
`REMOTE_HOST %host%`, where `%host%` is the visitor's address. There is no Clay login, no
Clay command, no settings: the page is the game, with Clay's output, Find, Search
History, Toggle Tags and an emoji picker around it.

It is a build of its own, made by `build.sh` beside `clay` (`./clay_web`, or
`./build.sh --only web`), never a release download, and it contains no shell worlds.

## Setting it up

`clay_web` has one file of settings, **`web.dat` in the directory it is started from** (or
the file `--conf=/path/web.dat` names), and it never writes it. Started where there is no
`web.dat`, it writes a sample there - every key, each explained - and stops: set
`hostname=` and `port=` to your game's and start it again. The file is `key=value` lines
and `#` comments, nothing else, and these are all its keys:

- **The web server**: `http_port` (the page is at `http://host:port/clay/`, or `https://`
  with a certificate), and your certificate and private key as PEM files, `cert_file` and
  `key_file` (Let's Encrypt's `fullchain.pem` and `privkey.pem`; a relative path is from
  web.dat's directory). **clay_web makes no self-signed certificate.** With both set,
  visitors get https; with both empty it serves **plain http, open to all**: everything
  visitors type then crosses the network unencrypted. Fine for a game on your own
  network; a public page deserves a certificate. After a renewal,
  `kill -USR1 <pid>` loads the new files. (A browser that has seen the page with a
  certificate insists on https for a year, so keep one once you have it.)
- **The game**: `world_name` (shown on the page's bar), `hostname`, `port`, `ssl` (`true`
  for a game that speaks TLS on that port), `encoding` (`utf8`, `latin1` or `fansi`) and
  the **First Command**, `first_command=REMOTE_HOST %host%`, sent to the game first thing
  on every visitor's connection with `%host%` replaced by the visitor's address (`\n`
  separates lines). A visitor logs into the game as themself, so there is no user,
  password or Login Command.
- **Rarely needed**: `world_type` (`mud`, or `mud_timed_prompt` with `prompt_wait_ms`),
  the keepalive (`keep_alive_type` NOP/None/Generic/Custom, `keep_alive_cmd`,
  `keep_alive_gag_enabled`, `keep_alive_gag`) and the telnet options `msp_enabled`,
  `mcp_enabled`, `mccp2_enabled`. The sample's values suit most games.

Anything else - a key the sample doesn't have, a `[section]`, a value that isn't what the
key takes - stops clay_web at start, naming the line. Everything a visitor's Clay could
otherwise be set up with is fixed: Clay's defaults (more-mode on, spell check on, no text
reveal, three input rows), the status panel off, Import Picture off, GMCP off, no triggers,
and the **Dark - New** theme, which each visitor may switch to **Light - New** from the
page's menu. Everyone may connect; an address that probes other paths or floods
connections is banned (see below).

**Files.** The directory web.dat is in is the whole of clay_web's file world. Beside it
Clay keeps `known_hosts.dat` (the pin of an SSL game's certificate, written on the first
connection) and `remote.log` (connections, bans and floods). **Nothing is read from or
written to `~/.clay`**, and there is no `debug.log`: a crash or a fatal signal is reported
on stderr, where a service manager keeps it.

Start it with `./clay_web` in that directory (or `./clay_web --conf=/path/web.dat`). It
reports what it serves in three lines - the game and the page's scheme and port, the file
the settings came from, and the certificate in use:

    Clay Portal: MyMUD (mud.example.org:4000) on https/9000
       Loading settings: web.dat
       Certificate: /etc/letsencrypt/live/example.org/fullchain.pem

Without a certificate the first line says `http`, and the third line is `https disabled -
No certificate` - shown only when the game itself speaks TLS (`ssl=true`); a game reached
in the clear has nothing to point out. Ctrl+C stops it; `kill -USR1 <pid>` reloads it:
every visitor's page is told and reconnects, and the process starts afresh with web.dat as
it is on disk, keeping its port.

## What a visitor gets

Each page is a Clay of its own on the server, so a visitor has Clay's output handling -
colours, wrapping, more-mode, prompts, telnet negotiation, ANSI music played by the
browser - and the hamburger menu holds **Help**, **Emoji Picker**, **Light Theme** (or
**Dark Theme**: the page's colours, remembered in that browser), **Toggle Tags**, **Find**
and **Search History**. The text size slider on the bar sets the size for that browser and
is remembered there too. The bar shows the world's name and connection dot and the More
indicator; on a phone the row has PgUp and PgDn (no world arrows). Help describes just
this.

Every typed line goes to the game as it is, `/something` included. When the game ends the
connection or the server is lost, one notice appears with one button, **Reconnect**;
closing the page ends the session. A page's connection to the game ends when the page
closes.

## Limits and security

- At most **five** sessions from one address at a time, and 200 in all; the next visitor is
  told so.
- Everyone may connect; there is no allow list. More than 30 new connections in 10 seconds
  from one address is a strike, and so is asking for a path other than `/clay/`; strikes
  ban (see the Security Notes).
- The server plays no sound of its own; visitors' browsers play the game's music.
- Nothing a visitor does reaches the server's files: the settings are never written, there
  is no archive, map, log or picture upload for visitors, and Clay's commands are not
  available. A visitor's Clay refuses the game's GMCP.
- Without a certificate the page and everything typed into it travel unencrypted. A public
  page deserves a real one: `cert_file` and `key_file` in web.dat.
