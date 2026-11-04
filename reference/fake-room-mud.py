#!/usr/bin/env python3
"""A tiny MUD with rooms, for the map (src/map): it offers GMCP, sends Room.Info for the
room you are in, and moves you when you type a direction.

    fake-room-mud.py <port> [--format ire|aard|generic|nocoords] [--lag MS]
                     [--teleport-after N] [--wrongdir]

The world: a 5x5 town (ids 1-25, North Gate at the top left, Town Square in the middle,
Far Corner at the bottom right) with one wall (no way between the Square and the room west
of it), a Bell Tower up from the top right (id 100, a level above), and Mines down from the
bottom left (ids 200-202, another area). `look` sends the room again; anything else is
"Huh?".

  --format    the Room.Info shape: IRE's (coords "area,x,y,z", y grows NORTH), Aardwolf's
              (coord {id,x,y}, y grows south), generic (x, y on the room), or no coordinates
  --lag MS    wait this long before answering a move
  --teleport-after N   the Nth move lands you in the Town Square whatever you typed
  --wrongdir  a move with no exit is answered with GMCP Room.WrongDir too
"""
import argparse
import json
import socket
import threading
import time

IAC, SE, SB, WILL, WONT, DO, DONT, GA = 255, 240, 250, 251, 252, 253, 254, 249
GMCP = 201

ap = argparse.ArgumentParser()
ap.add_argument("port", type=int)
ap.add_argument("--format", choices=["ire", "aard", "generic", "nocoords"], default="ire")
ap.add_argument("--lag", type=int, default=0)
ap.add_argument("--teleport-after", type=int, default=0)
ap.add_argument("--wrongdir", action="store_true")
args = ap.parse_args()

DIRS = {"n": (0, -1), "s": (0, 1), "e": (1, 0), "w": (-1, 0)}
WORDS = {"north": "n", "south": "s", "east": "e", "west": "w", "up": "u", "down": "d"}
SPECIAL = {(0, 0): "North Gate", (2, 2): "Town Square", (4, 4): "Far Corner", (2, 4): "South Market", (4, 0): "Bell Street"}


def town_id(x, y):
    return 1 + y * 5 + x


ROOMS = {}
for y in range(5):
    for x in range(5):
        rid = town_id(x, y)
        exits = {}
        for d, (dx, dy) in DIRS.items():
            nx, ny = x + dx, y + dy
            if 0 <= nx < 5 and 0 <= ny < 5:
                exits[d] = town_id(nx, ny)
        ROOMS[rid] = {"name": SPECIAL.get((x, y), f"Lane {x},{y}"), "area": "Town", "aid": 7, "x": x, "y": y, "z": 0, "exits": exits}
# A wall: no way between the Square and the room west of it.
del ROOMS[town_id(2, 2)]["exits"]["w"]
del ROOMS[town_id(1, 2)]["exits"]["e"]
ROOMS[town_id(4, 0)]["exits"]["u"] = 100
ROOMS[100] = {"name": "Bell Tower", "area": "Town", "aid": 7, "x": 4, "y": 0, "z": 1, "exits": {"d": town_id(4, 0)}}
ROOMS[town_id(0, 4)]["exits"]["d"] = 200
for i, name in enumerate(["Mine Entrance", "Mine Shaft", "Deep Mine"]):
    exits = {}
    if i == 0:
        exits["u"] = town_id(0, 4)
    if i > 0:
        exits["w"] = 200 + i - 1
    if i < 2:
        exits["e"] = 200 + i + 1
    ROOMS[200 + i] = {"name": name, "area": "Mines", "aid": 8, "x": i, "y": 0, "z": 0, "exits": exits}
# An exit the game shows but doesn't say where to: unexplored.
ROOMS[202]["exits"]["s"] = -1


def gmcp(package, obj):
    return bytes([IAC, SB, GMCP]) + f"{package} {json.dumps(obj)}".encode() + bytes([IAC, SE])


def room_info(rid):
    r = ROOMS[rid]
    base = {"num": rid, "name": r["name"], "exits": dict(r["exits"])}
    if args.format == "ire":
        base.update({"area": r["area"], "environment": "Urban", "coords": f"{r['aid']},{r['x']},{-r['y']},{r['z']}"})
    elif args.format == "aard":
        base.update({"zone": r["area"].lower(), "terrain": "city", "coord": {"id": r["aid"], "x": r["x"], "y": r["y"], "cont": 0}})
    elif args.format == "generic":
        base.update({"area": r["area"], "x": r["x"], "y": r["y"], "z": r["z"]})
    else:
        base.update({"area": r["area"]})
    return gmcp("Room.Info", base)


def describe(rid):
    r = ROOMS[rid]
    exits = ", ".join(sorted(r["exits"])) or "none"
    return f"\r\n{r['name']}\r\nYou see the usual sights.\r\nObvious exits: {exits}.\r\n".encode()


def handle(c):
    state = {"room": town_id(2, 2), "gmcp": False, "moves": 0}
    buf = b""

    def send_room(prefix=b""):
        out = prefix + describe(state["room"])
        if state["gmcp"]:
            out += room_info(state["room"])
        out += b"> " + bytes([IAC, GA])
        c.sendall(out)

    try:
        c.sendall(bytes([IAC, WILL, GMCP]) + b"Welcome to the Fake Room MUD.\r\n")
        while True:
            data = c.recv(4096)
            if not data:
                return
            buf += data
            # Telnet: answer DO GMCP, skip any other negotiation and subnegotiation.
            while IAC in buf:
                i = buf.index(IAC)
                if i + 1 >= len(buf):
                    break
                cmd = buf[i + 1]
                if cmd == SB:
                    end = buf.find(bytes([IAC, SE]), i)
                    if end < 0:
                        break
                    buf = buf[:i] + buf[end + 2:]
                    continue
                if cmd in (WILL, WONT, DO, DONT) and i + 2 < len(buf):
                    if cmd == DO and buf[i + 2] == GMCP and not state["gmcp"]:
                        state["gmcp"] = True
                        send_room()
                    buf = buf[:i] + buf[i + 3:]
                    continue
                buf = buf[:i] + buf[i + 2:]
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                cmd = line.decode(errors="replace").strip().lower()
                cmd = WORDS.get(cmd, cmd)
                if not cmd:
                    continue
                if cmd == "look":
                    send_room()
                    continue
                if cmd not in ("n", "s", "e", "w", "u", "d"):
                    c.sendall(b"Huh?\r\n> " + bytes([IAC, GA]))
                    continue
                to = ROOMS[state["room"]]["exits"].get(cmd)
                if args.lag:
                    time.sleep(args.lag / 1000)
                if not to or to < 0:
                    out = b"You can't go that way.\r\n"
                    if args.wrongdir and state["gmcp"]:
                        out += gmcp("Room.WrongDir", cmd)
                    c.sendall(out + b"> " + bytes([IAC, GA]))
                    continue
                state["moves"] += 1
                if args.teleport_after and state["moves"] == args.teleport_after:
                    state["room"] = town_id(2, 2)
                    send_room(b"A gust of wind carries you away!\r\n")
                    continue
                state["room"] = to
                send_room()
    except OSError:
        pass
    finally:
        c.close()


s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", args.port))
s.listen(8)
print(f"fake-room-mud on {args.port} ({args.format})", flush=True)
while True:
    conn, _ = s.accept()
    threading.Thread(target=handle, args=(conn,), daemon=True).start()
