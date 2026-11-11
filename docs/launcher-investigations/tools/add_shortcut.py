"""Adds a test shortcut to a Steam shortcuts.vdf (binary VDF). Usage: add_shortcut.py <vdf> <name> <exe> <start_dir> <launch_options>"""
import struct
import sys
import zlib


def parse_map(data, i):
    items = []
    while True:
        kind = data[i]
        i += 1
        if kind == 0x08:
            return items, i
        end = data.index(b"\x00", i)
        key = data[i:end]
        i = end + 1
        if kind == 0x00:
            value, i = parse_map(data, i)
        elif kind == 0x01:
            end = data.index(b"\x00", i)
            value = data[i:end]
            i = end + 1
        elif kind == 0x02:
            value = data[i:i + 4]
            i += 4
        elif kind == 0x07:
            value = data[i:i + 8]
            i += 8
        else:
            raise ValueError(f"unknown type {kind:#x} at {i}")
        items.append((kind, key, value))


def write_map(items):
    out = b""
    for kind, key, value in items:
        out += bytes([kind]) + key + b"\x00"
        if kind == 0x00:
            out += write_map(value)
        elif kind == 0x01:
            out += value + b"\x00"
        else:
            out += value
    return out + b"\x08"


path, name, exe, start_dir, options = sys.argv[1:6]
data = open(path, "rb").read()
root, _ = parse_map(data, 0)
shortcuts = root[0][2]
if any(dict((k, v) for _, k, v in entry[2]).get(b"AppName") == name.encode() for entry in shortcuts):
    sys.exit(f"{name} is already there")
app_id = zlib.crc32((exe + name).encode()) | 0x80000000
entry = [
    (0x02, b"appid", struct.pack("<I", app_id)),
    (0x01, b"AppName", name.encode()),
    (0x01, b"Exe", exe.encode()),
    (0x01, b"StartDir", start_dir.encode()),
    (0x01, b"icon", b""),
    (0x01, b"ShortcutPath", b""),
    (0x01, b"LaunchOptions", options.encode()),
    (0x02, b"IsHidden", struct.pack("<I", 0)),
    (0x02, b"AllowDesktopConfig", struct.pack("<I", 1)),
    (0x02, b"AllowOverlay", struct.pack("<I", 1)),
    (0x02, b"OpenVR", struct.pack("<I", 0)),
    (0x02, b"Devkit", struct.pack("<I", 0)),
    (0x01, b"DevkitGameID", b""),
    (0x02, b"DevkitOverrideAppID", struct.pack("<I", 0)),
    (0x02, b"LastPlayTime", struct.pack("<I", 0)),
    (0x01, b"FlatpakAppID", b""),
    (0x00, b"tags", []),
]
shortcuts.append((0x00, str(len(shortcuts)).encode(), entry))
open(path, "wb").write(write_map(root))
print(app_id, (app_id << 32) | 0x02000000)
