# Chat Worlds (Discord and Slack)

A world doesn't have to be a MUD. A **Discord** world attaches Clay to one Discord
server and a **Slack** world to one Slack workspace, through a bot you create. Every
channel the bot can read shows up in the world's output, tagged with where it came from.
What you type goes to the world's *send target*, which appears as the prompt.

```
#general <Alice> anyone around?
#general <ClayBot> yep
#dev/bugfix <Bob> pushed the fix       (a Discord thread)
@alice <Alice> quick question           (a direct message to the bot)
#general * Carol joined the server
[#general]                              <- the prompt: where your typing goes
```

Chat lines are ordinary world output. They run through actions and TF triggers, show
up on every interface (console, web, GUI, Android), are logged, and go into the
scrollback archive. Your own messages are shown too, but they **never fire triggers**,
because a trigger answering itself would loop.

## Setting up a Discord bot

1. Open the [Discord Developer Portal](https://discord.com/developers/applications)
   and press **New Application**. Give it a name; this becomes the bot's name.
2. Open **Bot** and press **Reset Token**. Copy the token; you paste it into Clay
   next. Treat it like a password.
3. On the same **Bot** page, under **Privileged Gateway Intents**, turn on
   **MESSAGE CONTENT INTENT** and save. Without it Discord refuses to let the bot read
   messages, and Clay tells you exactly this when you connect.
4. In Clay, open the world editor (`/worlds -e MyDiscord`), set **Type** to
   **Discord**, paste the token into **Token**, and press **Fetch**.
5. Fetch shows the bot's name and every server it has joined. If it hasn't joined
   any yet, Fetch shows an **invite link**. Open it in a browser, choose your server,
   and press Fetch again. The link asks for exactly the permissions Clay needs: View
   Channels, Send Messages and Read Message History.
6. Pick the **Server** and a **Send To** channel from the lists. Save, then connect.

## Setting up a Slack app

Slack needs **two** tokens.

1. Create an app at [api.slack.com/apps](https://api.slack.com/apps) (From scratch).
2. **Socket Mode:** turn it on. Slack asks you to create an **App-Level Token** with
   the `connections:write` scope. That token starts with `xapp-` and is the **App
   Token** in Clay.
3. **OAuth & Permissions → Bot Token Scopes:** add `channels:read`,
   `channels:history`, `groups:read`, `groups:history`, `im:read`, `im:history`,
   `im:write`, `mpim:read`, `mpim:history`, `users:read` and `chat:write`.
4. **Event Subscriptions:** enable it, and under **Subscribe to bot events** add
   `message.channels`, `message.groups`, `message.im` and `message.mpim`.
5. **Install to Workspace.** Copy the **Bot User OAuth Token** (`xoxb-...`); that is
   the **Bot Token** in Clay.
6. In Slack, invite the bot to each channel it should read: `/invite @YourBot`.
7. In Clay, set **Type** to **Slack**, fill in both tokens, press **Fetch**, and pick
   a **Send To** channel. Channels the bot hasn't been invited to are marked in the
   list.

## World settings

| Setting | Meaning |
|---------|---------|
| Token (Discord) | The bot token. Paste it as-is, without a `Bot ` prefix (one is ignored if present). |
| Bot Token / App Token (Slack) | The `xoxb-` and `xapp-` tokens (see above). |
| Server (Discord) | Which server this world shows. A name works, as does whatever Fetch fills in (`name (id)`). It can be left empty when the bot is in exactly one server. |
| Workspace (Slack) | The workspace name, filled in by Fetch (informational; the token decides the workspace). |
| Send To | Where typing goes when the world connects: `#channel`, a channel name, or `@user` for a direct message. |
| Show Only | Blank shows every channel. A comma-separated list such as `#general,#dev` shows only those (threads of a listed channel included). Direct messages always show. |
| Reconnect | Seconds to wait before reconnecting after the connection is lost, as for MUD worlds. |

You never need to type a numeric id. Fetch fills them in for you, and names work
everywhere. If two channels share a name (in different categories), Clay says so and
lists the ids; the entry Fetch writes (`general (1234...)`) is always unambiguous.

## The /chat command

`/chat` (also `/discord` and `/slack`) works on the current chat world, or on another
world with `-w<world>`.

| Command | What it does |
|---------|--------------|
| `/chat` | Status: server, bot, where typing goes, what is shown |
| `/chat channels [text]` | List channels by category; `*` marks where typing goes |
| `/chat users [text]` | People seen since connecting |
| `/chat to <target>` | Send typed text to `<target>` from now on (`#channel`, name, `@user`) |
| `/chat to <target> -d` | The same, and save it as the world's **Send To** |
| `/chat to -` | Reply where the most recent message came from |
| `/chat msg <target> <text>` | Send one message without changing the target |
| `/chat help` | Summary |

A `/chat to` change lasts for the session and survives `/reload`. Add `-d` to keep it
for future connections.

## Triggers on chat text

Every message line starts with its tag, so a trigger can match one channel:

```
/def -mregexp -t"^#general <(.*)> !roll" roll = /chat msg #general %{P1} rolled $[rand(1,6)]
```

A message spanning several lines arrives as several lines; the continuation lines are
indented two spaces. Your own messages never fire triggers.

## Sending

- Long text is split to fit each service's message limit (2000 characters on Discord).
- On Discord, `@everyone`, `@here` and role mentions in text you send (or a trigger
  sends) **never ping anyone**; only direct user mentions do.
- Rate limits are handled automatically. A message that can't be delivered gets a
  plain explanation in the world, for example that the bot isn't allowed to post in
  that channel or hasn't been invited to it.

## Connection behaviour

- A short network drop doesn't disconnect the world. Clay reconnects on its own, and
  on Discord it *resumes* the session, so messages sent while you were away still
  arrive.
- `/reload` resumes the session as well, keeping the send target.
- Problems retrying can't fix end the connection with instructions instead of
  retrying: an invalid token, the Message Content intent being off, or a missing Slack
  token. The world's Reconnect setting does not apply to these.
- In `--multiuser` mode a chat world belongs to its owner, and the owner's session is
  the world's one connection. `/chat` works there too.

## Troubleshooting

| Message | Fix |
|---------|-----|
| *Discord rejected the bot token* | Reset the token in the Developer Portal (Bot page) and paste the new one. |
| *...isn't allowed to read messages* / *MESSAGE CONTENT INTENT* | Developer Portal → your app → Bot → Privileged Gateway Intents → turn on Message Content Intent. |
| *The bot isn't in any Discord server yet* | Use the invite link Fetch shows. |
| *The bot isn't allowed to post in #x* | Give the bot's role View Channel and Send Messages in that channel. |
| *The bot isn't in #x* (Slack) | Type `/invite @YourBot` in that channel in Slack. |
| *missing a permission scope* (Slack) | Add the scopes listed above, then reinstall the app. |
| *Slack worlds need two tokens* | Add the App Token (`xapp-`), with Socket Mode turned on. |
| Nothing shows from a channel | Check Show Only, and that the bot can see that channel. |
