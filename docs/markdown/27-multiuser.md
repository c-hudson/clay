# The Multiuser Server (clay_multi)

`clay_multi` is one Clay process serving many people, each with **a full Clay of their
own**: their settings, worlds, actions and triggers, TinyFugue scripts, scrollback archive,
maps, notes, theme and keybindings, kept in `~/.clay/users/<name>/` on the server. Users
log in with a username and password, from the web page, the Android app, an SSH console
(`clay --console=host:port`) or the native GUI (`clay --gui=host:port`), and from then on
have Clay as the operator does - with the few exceptions below.

It is a build of its own, made by `build.sh` beside `clay` (`./clay_multi`, or
`./build.sh --only multi`), never a release download, and it contains no shell worlds.

## Accounts: password.dat

`~/.clay/password.dat` holds one line per user: the username, a space, the password.
Spaces are not allowed in a username (letters, digits, `_`, `.` and `-`, up to 32
characters, not starting with `.`); a password may contain spaces. Blank lines and lines
starting with `#` are ignored.

```
# clay_multi accounts
alice correct horse battery staple
bob hunter2
```

Clay only ever reads the file - at every login, so adding a user needs no restart - and
never writes it. Make it readable by the server's user only (`chmod 600`). A bad line is
reported in `~/.clay/debug.log` by its number, never by its content. Without the file,
`clay_multi` prints what it needs and exits.

## Running it

```bash
./clay_multi
```

The server's own settings - the port, the stealth path, the certificate and the allow list
- come from the operator's `~/.clay/settings.dat` (`/web` in a regular Clay sets them), as
for `-D`. The operator's worlds and web password are not used.

Ctrl+C (or SIGTERM) stops it: every user's Clay saves and ends. `kill -USR1 <pid>` reloads
it: every connected client is told and reconnects, the process starts afresh with the
settings on disk and keeps its port. MUD connections are not carried across; the
scrollback archive is, in each user's directory.

## Each user's Clay

A user's Clay starts at their first login and stays up until the server stops, so their
worlds stay connected while they are away, as a Clay's do. Two logins by the same user
share it, as two windows share one Clay. The first login creates
`~/.clay/users/<name>/` with its own `settings.dat`, `secure.key`, `scrollback.db`,
`maps.db`, `logs/`, `theme.dat` and `keybindings.dat`.

Everything in Settings, the world editor, the actions editor and TinyFugue is the user's
own. The Web tab of Settings shows only their **Auth Key** and Modify Key: a user's auth
key logs their phone or GUI in without the password, and knocks at the server as it would
at a Clay of their own; the rest of that tab (port, path, password, certificate, allow
list, remote access) is the operator's.

## What a user cannot do

- Read or write the server's files, or start a process: `/sh`, `/sys`, `/quote 'file`,
  `/quote !command`, `/load`, `/save`, `/lcd`, `/log <file>`, `/note <file>`, `fwrite()`
  and `tfopen()` are refused (TinyFugue's `/restrict file` is on from the start and cannot
  be lowered). `/quote #` of the history works; a world's own session log (the Log switch
  in Edit World) is written in the user's directory.
- `/reload` and `/update`: the operator reloads with `kill -USR1`.
- `/web`, bans, the port mapping, the firewall, `/reach`: the server's.
- Change Password: the operator edits password.dat.
- Play sound or speech on the server: music, media and the voice go to the user's own
  device only.

## Security

Logins are challenge-response, as on every Clay; an unknown user and a wrong password get
the same answer, and five failures ban the address. Each user's Clay has its own
certificate pins (`known_hosts.dat`) and its own machine key, and one user's Clay never
sees another's. See the Security Notes for the connection-flood limit and the ban rules.
