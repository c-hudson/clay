#!/usr/bin/env python3
"""Writes tests/native_gui/corpus.ans: the rendering corpus for the native GUI's
Phase-0 comparison against the WebView. Every category of output the two renderers
must agree on, as raw MUD bytes (CRLF line endings, real ESC characters).

clay-test-server's `corpus` scenario serves the file. Regenerate with
    python3 tests/native_gui/make_corpus.py
"""
import os

E = "\x1b["
R = E + "0m"
out = []


def line(s=""):
    out.append(s)


def section(name):
    line()
    line(f"{E}1;37m== {name} =={R}")


section("Prose")
line("You are standing in a small clearing in the forest. Tall pines rise all around you, and a narrow")
line("path winds north toward the sound of running water. A weathered signpost leans against a stump.")
line("This line is deliberately long so that it has to wrap across several rows of the output window no matter how wide the window happens to be when you look at it, which tests ordinary word wrapping.")
line("    An indented line, four spaces in.   Three spaces between words.   Trailing spaces follow:      ")

section("16 colours")
names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"]
line(" ".join(f"{E}{30 + i}m{n}{R}" for i, n in enumerate(names)))
line(" ".join(f"{E}{90 + i}m{n}{R}" for i, n in enumerate(names)) + "   (90-97)")
line(" ".join(f"{E}1;{30 + i}m{n}{R}" for i, n in enumerate(names)) + "   (bold 30-37)")
line(" ".join(f"{E}{30 + i};1m{n}{R}" for i, n in enumerate(names)) + "   (30-37 then bold)")
line(" ".join(f"{E}{40 + i}m {n} {R}" for i, n in enumerate(names)))
line(" ".join(f"{E}30;{100 + i}m {n} {R}" for i, n in enumerate(names)))

section("256 colours")
line("".join(f"{E}48;5;{i}m  " for i in range(16)) + R)
for row in range(6):
    line("".join(f"{E}48;5;{16 + row * 36 + c}m  " for c in range(36)) + R)
line("".join(f"{E}48;5;{232 + i}m  " for i in range(24)) + R)
line("".join(f"{E}38;5;{16 + i * 6}m{i % 10}" for i in range(36)) + R + "  foreground cube steps")

section("Truecolour")
line("".join(f"{E}38;2;{int(255 * i / 59)};{int(128 + 127 * i / 59)};{255 - int(255 * i / 59)}m#" for i in range(60)) + R)
line("".join(f"{E}48;2;{int(255 * i / 59)};40;{255 - int(255 * i / 59)}m " for i in range(60)) + R)
line(f"{E}38;2;255;180;0;48;2;30;30;60m amber on navy {R} {E}38;2;10;10;10;48;2;240;240;200m ink on paper {R}")

section("Attributes")
line(f"{E}1mbold{R} {E}3mitalic{R} {E}4munderline{R} {E}5mblink{R} {E}7mreverse{R} {E}1;3mbold italic{R} {E}1;4;31mbold underline red{R}")
line(f"{E}7;32mreverse green{R} {E}7;33;44mreverse yellow on blue{R} {E}2mdim (ignored){R} {E}9mstrike (ignored){R}")
line(f"stray leftovers [0m[1;32mgone, but [match( and [menu] stay{R}")
line(f"descenders under a line: {E}4mjumpy gray pygmy hogs{R} and {E}4;36mgoing quietly, yes{R}")

section("Block art")
line(f"{E}31m▄▄▄{E}33m▄▄▄{E}32m▄▄▄{E}36m▄▄▄{E}34m▄▄▄{E}35m▄▄▄{R}  half blocks, default background")
line(f"{E}31;44m▀▀▀{E}33;41m▀▀▀{E}32;43m▀▀▀{E}36;42m▀▀▀{E}34;46m▀▀▀{E}35;44m▀▀▀{R}  half blocks on backgrounds")
line(f"{E}37;44m░░░▒▒▒▓▓▓███{R} {E}37m░░░▒▒▒▓▓▓███{R}  shades on blue / on default")
line(f"{E}33m▁▂▃▄▅▆▇█{R} {E}36m▏▎▍▌▋▊▉█{R} {E}35m▐▕▔{R}  eighths")
line(f"{E}32m▖▗▘▙▚▛▜▝▞▟{R}  quadrants")
art = [
    "  ████  ██      ▄██▄  ██  ██",
    " ██     ██     ██  ██  ████ ",
    " ██     ██     ██████   ██  ",
    "  ████  ██████ ██  ██   ██  ",
]
for i, a in enumerate(art):
    line(f"{E}{[91, 93, 92, 96][i]};40m{a}{R}")
line(f"{E}44m{' ' * 20}{E}41m{' ' * 20}{E}42m{' ' * 20}{R}  coloured spaces")

section("Box drawing")
line("┌──────────┬──────────┐  ╔══════════╗")
line("│ Name     │ Level    │  ║ Double   ║")
line("├──────────┼──────────┤  ╠══════════╣")
line("│ Aragorn  │ 87       │  ║ ┃ heavy  ║")
line("└──────────┴──────────┘  ╚══════════╝")
line("╭─rounded─╮ ━━━ ┅┅┅ ╱╲╳")

section("CJK")
line("中文：你好，世界！这是一行中文文本，用来测试换行和字体回退。")
line("日本語：こんにちは、世界。カタカナとひらがなと漢字が混ざっています。")
line("한국어: 안녕하세요, 세계! 한글 문장입니다.")
line("Mixed: Hello 你好 こんにちは 안녕 Ｆｕｌｌｗｉｄｔｈ")

section("Emoji")
line("single: 😀 🎉 🐉 🍺 ⚔️ 🛡️ 🏰 🔥 ❤️ ✅")
line("ZWJ: 🐈‍⬛ 👨‍👩‍👧 🏳️‍🌈 🧑‍💻 👩‍🚀")
line("skin tones: 👋🏻 👋🏽 👋🏿 👍🏼 🙏🏾")
line("flags: 🇺🇸 🇯🇵 🇬🇧 🇩🇪 🇧🇷 🏴\U000E0067\U000E0062\U000E0073\U000E0063\U000E0074\U000E007F")
line("keycaps: 1️⃣ 2️⃣ #️⃣ *️⃣   variation: ☺︎ ☺️ ♥ ♥️ ✌ ✌️")
line("Clay's markers: ✨ client line  ▶ new text  🛢️ archive")
line("squares: 🟥🟧🟨🟩🟦🟪🟫⬛⬜ circles: 🔴🟠🟡🟢🔵🟣")
line(f"{E}33memoji in colour: 🐉 dragon{R} {E}44memoji on blue 🔥{R}")

section("Long words and URLs")
line("Visit https://example.com/some/really/long/path/that/keeps/going/and/going?query=value&other=thing#section for details.")
line("www.example.org/a-page, (https://example.net/parens), and \"https://example.com/quoted\".")
line("/usr/local/share/clay/very/long/path/with/many/segments/and_underscores_too/file-name-here.txt")
line("Supercalifragilisticexpialidocious" * 4)
line("a" * 150)

section("Combining marks and scripts")
line("composed: café naïve résumé  decomposed: café naïve résumé")
line("Tiếng Việt có dấu. Ελληνικά. Русский текст. ‏עברית‏ مرحبا بالعالم")
line("symbols: → ⇒ ← ↑ ↓ ∞ ≠ ≤ ≥ ± × ÷ € £ ¥ © ® ™ ✓ ✗ ★ ☆ ♠ ♣ ♥ ♦ ⚑ ⠿")

section("Tabs and spacing")
line("col1\tcol2\tcol3\tcol4")
line("a\tbb\tccc\tdddd")

section("Discord and MUD tags")
# Real emoji (the examples in Discord's own message-formatting docs: a PNG and an
# animated GIF), then one whose id the CDN doesn't know.
line("[chat(ooc)] Bob: hello there <:mmLol:216154654256398347> and <a:b1nzy:392938283556143104>")
line("Carol: an emoji that no longer exists <:gone:123456789012345678>")
line("  [indented] tags are never stripped")
line(f"{E}36m[Newbie] Alice: colours after a tag{R}")

section("End")
line("Last line of the corpus.")

path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus.ans")
with open(path, "wb") as f:
    f.write(("\r\n".join(out) + "\r\n").encode("utf-8"))
print(f"wrote {path}: {len(out)} lines")
