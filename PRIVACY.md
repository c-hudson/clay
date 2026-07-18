# Privacy Policy — Clay

Clay is a MUD (Multi-User Dungeon) client. This policy covers the Clay app, including the
Android app, in all its run modes (standalone on-device server, and remote client connecting
to a separately-hosted Clay instance).

## Summary

Clay does not collect, transmit, or sell any personal data to its developer or any third
party. There is no analytics, telemetry, advertising, or crash-reporting SDK anywhere in the
app. There is no Clay account, sign-up flow, or centralized service operated by the
developer that the app talks to.

## What Clay connects to

Every network connection Clay makes is one you explicitly configure:

- **MUD servers.** You choose which MUD/game servers to connect to, by hostname and port.
  Clay sends whatever you type to that server and displays whatever it sends back, the same
  way any terminal or telnet client would. Clay has no visibility into, and does not
  moderate, filter, or store elsewhere, the content of those sessions beyond your own local
  scrollback/logs (see below).
- **Your own Clay server.** If you run Clay as a remote-accessible server (desktop, or your
  own hosting) and use another device (including the Android app) to view it, that
  connection is between your devices only — it is not relayed through, or visible to, the
  developer.
- **The Android standalone/local-server mode.** When the Android app runs Clay entirely
  on-device, the embedded server binds to the phone's own loopback address (`127.0.0.1`)
  only. It is not reachable from your network or the internet.

The only network destination that ships as a default (not a requirement) is a placeholder
example MUD world, which you can freely change or delete.

## Data stored on your device

- **Credentials.** Any username/password/auto-login credentials you enter for a MUD server,
  and any auth key or password you configure for connecting to your own Clay server, are
  stored locally on your device (in Clay's settings file on desktop, or Android
  SharedPreferences on the Android app) and are never sent anywhere except to the server
  you configured them for.
- **Session logs / scrollback.** If you enable per-world logging or scrollback archiving,
  that data is written to local storage on the device running the Clay server. It is not
  uploaded anywhere.
- **Certificate pins.** For TLS connections to a Clay server, Clay pins the server's
  certificate the first time it connects (trust-on-first-use) so it can detect a later
  substitution. This pin is stored locally and is not shared with anyone.

## Permissions (Android)

The Android app requests only the permissions it needs to function:

- **Internet / network state** — to make the connections described above.
- **Notifications** — to show connection-status notifications you can see and dismiss.
- **Foreground service** — to keep a MUD connection alive while the app is backgrounded, with
  a persistent notification while it's running so you always know it's active.
- **Battery optimization exemption** — requested only the first time you actually start a
  background connection, so Android doesn't kill it prematurely.

None of these permissions are used to collect or transmit data about you or your device usage.

## Third parties

Clay has no ad network, analytics vendor, or data broker integrations. The only third-party
network destinations are the ones you yourself configure (MUD servers, your own Clay server)
and, in the desktop/web UI, an optional URL-shortening service you can choose to use when
sharing links — which is only contacted when you explicitly invoke that feature.

## Changes to this policy

If Clay's data practices change, this file will be updated accordingly.

## Contact

Clay is an open-source project. Questions or concerns can be raised via the project's GitHub
repository issue tracker.
