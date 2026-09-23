#!/usr/bin/env python3
"""Build `examples/valley/assets/{models,textures}` from the CC0 source kits.

This is the one-off conversion step behind card #65: the committed OBJ and PNG
files are what the game loads, and this script only exists so that the same
files can be produced again from the same sources.

It does four things the raw kits do not:

* flattens every material into one shared atlas — the output OBJ has no
  `mtllib`/`usemtl` at all, every face points at the center of an atlas cell;
* repaints those cells into the valley palette (muted northern tones, with the
  berry as the single saturated color — embers stay in code, see
  `crates/runity/examples/valley/look.rs`);
* puts models on a meter scale, Y up, resting on y = 0;
* composes and poses: a campfire is stones plus charred logs, a settler is the
  blocky character with its limbs swung around their joints.

The sources are three Kenney kits, all CC0 (see `examples/valley/assets/CREDITS.txt`):

    https://kenney.nl/assets/nature-kit
    https://kenney.nl/assets/survival-kit
    https://kenney.nl/assets/blocky-characters

Unzip them side by side and run:

    python3 tools/example-assets/build_assets.py --kits /path/to/kits

where `/path/to/kits` holds `nature/`, `survival/` and `blocky/`. Only the
output is committed; the kits themselves are not part of the repository.

No third-party Python packages, for the same reason the engine has no crates.
"""

import argparse
import math
import struct
import sys
import zlib
from pathlib import Path

# --------------------------------------------------------------------------
# Palette
# --------------------------------------------------------------------------

# Every surface color in the valley, as (dark, mid, light). The tiers are what
# the kits' own shading needs: a kit face that reads dark stays dark after the
# repaint. Saturation is kept below the threshold in `look.rs` for all of them
# except `berry`, which is deliberately the one loud color in the scene.
PALETTE = {
    "bark": ("#3F3327", "#4E4033", "#5E4E3E"),
    "wood_cut": ("#9C8668", "#B09877", "#C2AA86"),
    "plank": ("#7C633E", "#8E7349", "#A08356"),
    "needle": ("#2E4436", "#3A5443", "#46644F"),
    "leaf": ("#4C5F3A", "#5A6E45", "#687D50"),
    "grass": ("#5C6B3E", "#6B7A4A", "#7A8956"),
    "stone": ("#6C7076", "#7E8288", "#90949A"),
    "dirt": ("#5B4C3A", "#6B5B46", "#7B6A52"),
    "canvas": ("#A49474", "#B6A684", "#C8B896"),
    "charcoal": ("#1C1917", "#2B2724", "#3A3532"),
    "steel": ("#79818B", "#8A929C", "#9BA3AD"),
    "skin": ("#B89073", "#C9A183", "#D8B094"),
    "cloth": ("#999385", "#A8A294", "#B7B1A3"),
    "trouser": ("#403A2E", "#4E4638", "#5C5444"),
    # The berry is one color at every tier: the atlas test asks for a single
    # saturated spot, and a spread of reds would read as several.
    "berry": ("#C42A2A", "#C42A2A", "#C42A2A"),
}

ATLAS_CELLS = 16  # cells per side
ATLAS_CELL_PX = 16  # pixels per cell
ATLAS_SIZE = ATLAS_CELLS * ATLAS_CELL_PX

# Row order in the atlas. One family per row, tier in the first three columns;
# the rest of the row repeats the mid tier so no cell is left undefined.
ATLAS_ROWS = list(PALETTE)


def hex_rgb(text):
    return tuple(int(text[i : i + 2], 16) for i in (1, 3, 5))


def cell_uv(family, tier):
    """The OBJ-space UV of the center of a palette cell (V axis up)."""
    row = ATLAS_ROWS.index(family)
    u = (tier + 0.5) / ATLAS_CELLS
    v = 1.0 - (row + 0.5) / ATLAS_CELLS
    return (u, v)


def build_atlas():
    """The atlas image, as rows of `(r, g, b)` tuples."""
    pixels = []
    for y in range(ATLAS_SIZE):
        row_index = y // ATLAS_CELL_PX
        # Rows past the last family are spare: stone, so an accidental UV in
        # empty space lands on something dull rather than on the berry.
        family = ATLAS_ROWS[row_index] if row_index < len(ATLAS_ROWS) else "stone"
        row = []
        for x in range(ATLAS_SIZE):
            col = x // ATLAS_CELL_PX
            tier = col if col < 3 else 1
            row.append(hex_rgb(PALETTE[family][tier]))
        pixels.append(row)
    return pixels


# --------------------------------------------------------------------------
# PNG (8-bit RGB, non-interlaced — what `runity_render::decode_png` accepts)
# --------------------------------------------------------------------------


def write_png(path, pixels):
    height = len(pixels)
    width = len(pixels[0])
    raw = bytearray()
    for row in pixels:
        raw.append(0)  # filter: none
        for r, g, b in row:
            raw += bytes((r, g, b))

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    blob = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )
    path.write_bytes(blob)


def read_png(path):
    """Decode an 8-bit PNG (grey, RGB, palette, with or without alpha)."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    pos, idat, plte = 8, b"", None
    width = height = color = 0
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        tag = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if tag == b"IHDR":
            width, height, depth, color, _, _, interlace = struct.unpack(
                ">IIBBBBB", body
            )
            if depth != 8 or interlace != 0:
                raise ValueError(f"{path}: only 8-bit non-interlaced PNGs are read")
        elif tag == b"PLTE":
            plte = body
        elif tag == b"IDAT":
            idat += body
        pos += 12 + length

    raw = zlib.decompress(idat)
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color]
    stride = width * channels
    prev = bytearray(stride)
    rows, p = [], 0
    for _ in range(height):
        filt = raw[p]
        p += 1
        line = bytearray(raw[p : p + stride])
        p += stride
        for i in range(stride):
            a = line[i - channels] if i >= channels else 0
            b = prev[i]
            c = prev[i - channels] if i >= channels else 0
            x = line[i]
            if filt == 1:
                x += a
            elif filt == 2:
                x += b
            elif filt == 3:
                x += (a + b) // 2
            elif filt == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                x += a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
            line[i] = x & 0xFF
        rows.append(bytes(line))
        prev = line

    out = []
    for line in rows:
        row = []
        for i in range(width):
            if color == 3:
                idx = line[i]
                row.append((plte[idx * 3], plte[idx * 3 + 1], plte[idx * 3 + 2]))
            elif color == 2:
                row.append(tuple(line[i * 3 : i * 3 + 3]))
            elif color == 6:
                row.append(tuple(line[i * 4 : i * 4 + 3]))
            else:
                g = line[i * channels]
                row.append((g, g, g))
        out.append(row)
    return out


# --------------------------------------------------------------------------
# Wavefront OBJ in
# --------------------------------------------------------------------------


class SourceMesh:
    def __init__(self):
        self.positions = []
        self.uvs = []
        self.normals = []
        self.faces = []  # (group, material, [(pi, ti, ni), ...])


def load_obj(path):
    mesh = SourceMesh()
    group, material = "", ""
    for line in path.read_text().splitlines():
        parts = line.split()
        if not parts:
            continue
        tag = parts[0]
        if tag == "v":
            mesh.positions.append(tuple(float(x) for x in parts[1:4]))
        elif tag == "vt":
            mesh.uvs.append(tuple(float(x) for x in parts[1:3]))
        elif tag == "vn":
            mesh.normals.append(tuple(float(x) for x in parts[1:4]))
        elif tag in ("g", "o"):
            group = " ".join(parts[1:])
        elif tag == "usemtl":
            material = parts[1]
        elif tag == "f":
            idx = []
            for token in parts[1:]:
                fields = (token.split("/") + ["", ""])[:3]
                pi = int(fields[0])
                ti = int(fields[1]) if fields[1] else None
                ni = int(fields[2]) if fields[2] else None
                idx.append((pi, ti, ni))
            mesh.faces.append((group, material, idx))
    return mesh


def load_mtl_colors(path):
    colors, name = {}, None
    if not path.exists():
        return colors
    for line in path.read_text().splitlines():
        parts = line.split()
        if not parts:
            continue
        if parts[0] == "newmtl":
            name = parts[1]
        elif parts[0] == "Kd" and name:
            colors[name] = tuple(float(x) for x in parts[1:4])
    return colors


# --------------------------------------------------------------------------
# Geometry helpers
# --------------------------------------------------------------------------


def rotate(point, axis, degrees):
    a = math.radians(degrees)
    c, s = math.cos(a), math.sin(a)
    x, y, z = point
    if axis == "x":
        return (x, y * c - z * s, y * s + z * c)
    if axis == "y":
        return (x * c + z * s, y, -x * s + z * c)
    return (x * c - y * s, x * s + y * c, z)


def normalize(v):
    length = math.sqrt(sum(c * c for c in v))
    if length == 0.0:
        return (0.0, 1.0, 0.0)
    return tuple(c / length for c in v)


def face_normal(points):
    (ax, ay, az), (bx, by, bz), (cx, cy, cz) = points[0], points[1], points[2]
    u = (bx - ax, by - ay, bz - az)
    v = (cx - ax, cy - ay, cz - az)
    return normalize(
        (
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        )
    )


def bounds(points):
    return [
        (min(p[k] for p in points), max(p[k] for p in points)) for k in range(3)
    ]


# --------------------------------------------------------------------------
# Color resolution: kit face -> palette cell
# --------------------------------------------------------------------------

# Nature kit: one flat `Kd` material per surface, and the names say what they
# are — so the repaint is a table.
NATURE_MATERIALS = {
    "stone": ("stone", 1),
    "wood": ("plank", 1),
    "woodDark": ("plank", 0),
    "woodBark": ("bark", 1),
    "woodBarkDark": ("bark", 0),
    "woodInner": ("wood_cut", 2),
    "leafsDark": ("needle", 1),
    "leafsGreen": ("leaf", 1),
    "grass": ("grass", 1),
    "dirt": ("dirt", 1),
    "colorRed": ("canvas", 2),
    "colorRedDark": ("canvas", 1),
}

# Blocky characters: one skin texture, but the parts are separate groups, so
# the settler is repainted limb by limb.
CHARACTER_PARTS = {
    "head": ("skin", 1),
    "torso": ("cloth", 1),
    "arm-left": ("skin", 1),
    "arm-right": ("skin", 1),
    "leg-left": ("trouser", 1),
    "leg-right": ("trouser", 1),
}


def rgb_to_hsv(rgb):
    r, g, b = (c / 255.0 for c in rgb)
    high, low = max(r, g, b), min(r, g, b)
    span = high - low
    if span == 0.0:
        hue = 0.0
    elif high == r:
        hue = 60.0 * (((g - b) / span) % 6.0)
    elif high == g:
        hue = 60.0 * ((b - r) / span + 2.0)
    else:
        hue = 60.0 * ((r - g) / span + 4.0)
    return hue, (0.0 if high == 0.0 else span / high), high


def classify_colormap(rgb):
    """Survival kit: its colormap is a shaded gradient, so the repaint goes by
    hue family and keeps the shade as a palette tier."""
    hue, sat, value = rgb_to_hsv(rgb)
    if sat < 0.12:
        return ("stone", 0 if value < 0.45 else 1 if value < 0.75 else 2)
    if 180.0 <= hue <= 280.0:
        return ("steel", 0 if value < 0.60 else 1 if value < 0.80 else 2)
    if 60.0 <= hue <= 170.0:
        return ("grass", 0 if value < 0.45 else 1 if value < 0.70 else 2)
    if 25.0 <= hue <= 60.0 and sat > 0.55:
        return ("canvas", 0 if value < 0.75 else 1 if value < 0.92 else 2)
    # Everything else in these kits is wood, from dark bark to a cut end.
    if value < 0.75:
        return ("bark", 0 if value < 0.70 else 1)
    if value < 0.88:
        return ("plank", 1)
    return ("wood_cut", 2)


# --------------------------------------------------------------------------
# The content manifest
# --------------------------------------------------------------------------
#
# Each entry is one committed model. `parts` are source pieces placed in
# meters relative to each other; the finished model is then dropped onto
# y = 0 and centered on the XZ plane, so "the support is at the bottom".
#
# Part keys:
#   kit, name     which source OBJ
#   fit           (axis, meters) — uniform scale so that axis measures this
#   scale         uniform scale, when `fit` would be the wrong handle
#   pose          [(group, axis, degrees, pivot)] — swung before scaling
#   rotate        [(axis, degrees)] — applied after scaling
#   at            (x, y, z) offset in meters
#   repaint       {source group or material: (family, tier)} overrides
#   only          keep only these groups
#   berries       scatter N berry octahedra over the part's top

MANIFEST = [
    {
        "file": "campfire.obj",
        "what": "костёр: кольцо камней и обугленные поленья",
        "parts": [
            {"kit": "nature", "name": "campfire_stones", "fit": ("x", 1.20)},
            {
                "kit": "nature",
                "name": "campfire_logs",
                "fit": ("x", 0.70),
                "repaint": {"wood": ("charcoal", 2), "woodDark": ("charcoal", 1)},
                "at": (0.0, 0.02, 0.0),
            },
        ],
    },
    {
        "file": "shelter.obj",
        "what": "шалаш: остов и полотнище",
        "parts": [{"kit": "nature", "name": "tent_smallOpen", "fit": ("y", 2.00)}],
    },
    {
        "file": "wall_unfinished.obj",
        "what": "недостроенная стена: начатый остов и куча брёвен рядом",
        "parts": [
            {
                "kit": "survival",
                "name": "structure",
                "fit": ("y", 2.20),
                "at": (0.0, 0.0, 0.0),
            },
            {
                "kit": "nature",
                "name": "log_stack",
                "fit": ("z", 2.20),
                "rotate": [("y", 90.0)],
                "at": (1.75, 0.0, -0.20),
            },
            {
                "kit": "nature",
                "name": "log",
                "fit": ("z", 2.00),
                "rotate": [("y", 74.0)],
                "at": (1.55, 0.0, 1.00),
            },
        ],
    },
    {
        "file": "logs_loose.obj",
        "what": "брёвна россыпью",
        "parts": [
            {"kit": "nature", "name": "log", "fit": ("z", 2.10), "at": (0.0, 0.0, 0.0)},
            {
                "kit": "nature",
                "name": "log",
                "fit": ("z", 1.70),
                "rotate": [("y", 28.0)],
                "at": (0.75, 0.0, 0.35),
            },
            {
                "kit": "survival",
                "name": "tree-log-small",
                "fit": ("z", 1.20),
                "rotate": [("y", -47.0)],
                "at": (-0.55, 0.0, 0.60),
            },
        ],
    },
    {
        "file": "logs_bundle.obj",
        "what": "брёвна в связке",
        "parts": [{"kit": "nature", "name": "log_stackLarge", "fit": ("z", 2.20)}],
    },
    {
        "file": "axe.obj",
        "what": "топор",
        "parts": [{"kit": "survival", "name": "tool-axe", "fit": ("y", 0.80)}],
    },
    {
        "file": "settler_stand.obj",
        "what": "поселенец: стоит",
        "parts": [
            {
                "kit": "blocky",
                "name": "character-a",
                "fit": ("y", 1.80),
                "pose": [
                    ("arm-left", "z", 4.0, (0.6, 1.9, 0.0)),
                    ("arm-right", "z", -4.0, (-0.6, 1.9, 0.0)),
                ],
            }
        ],
    },
    {
        "file": "settler_carry.obj",
        "what": "поселенец: несёт груз перед собой",
        "parts": [
            {
                "kit": "blocky",
                "name": "character-a",
                "fit": ("y", 1.80),
                "pose": [
                    ("arm-left", "x", -78.0, (0.6, 1.9, 0.0)),
                    ("arm-right", "x", -78.0, (-0.6, 1.9, 0.0)),
                ],
            }
        ],
    },
    {
        "file": "settler_walk.obj",
        "what": "поселенец: шагает",
        "parts": [
            {
                "kit": "blocky",
                "name": "character-a",
                "fit": ("y", 1.80),
                "pose": [
                    ("arm-left", "x", 32.0, (0.6, 1.9, 0.0)),
                    ("arm-right", "x", -32.0, (-0.6, 1.9, 0.0)),
                    ("leg-left", "x", -26.0, (0.2, 1.0, 0.0)),
                    ("leg-right", "x", 26.0, (-0.2, 1.0, 0.0)),
                ],
            }
        ],
    },
    {
        "file": "hands_first_person.obj",
        "what": "руки-предплечья от первого лица",
        "parts": [
            {
                "kit": "blocky",
                "name": "character-a",
                "only": ["arm-left"],
                "fit": ("y", 0.62),
                "rotate": [("x", -48.0), ("y", -12.0)],
                "at": (0.17, 0.0, 0.0),
            },
            {
                "kit": "blocky",
                "name": "character-a",
                "only": ["arm-right"],
                "fit": ("y", 0.62),
                "rotate": [("x", -48.0), ("y", 12.0)],
                "at": (-0.17, 0.0, 0.0),
            },
        ],
    },
    {
        "file": "pine_small.obj",
        "what": "ель, малая",
        "parts": [{"kit": "nature", "name": "tree_pineSmallA", "fit": ("y", 2.20)}],
    },
    {
        "file": "pine_medium.obj",
        "what": "ель, средняя",
        "parts": [{"kit": "nature", "name": "tree_pineDefaultA", "fit": ("y", 5.00)}],
    },
    {
        "file": "pine_large.obj",
        "what": "ель, большая",
        "parts": [{"kit": "nature", "name": "tree_pineTallA", "fit": ("y", 8.50)}],
    },
    {
        "file": "broadleaf_small.obj",
        "what": "лиственное дерево, малое",
        "parts": [{"kit": "nature", "name": "tree_small", "fit": ("y", 2.60)}],
    },
    {
        "file": "broadleaf_medium.obj",
        "what": "лиственное дерево, среднее",
        "parts": [{"kit": "nature", "name": "tree_default", "fit": ("y", 5.20)}],
    },
    {
        "file": "broadleaf_large.obj",
        "what": "лиственное дерево, большое",
        "parts": [{"kit": "nature", "name": "tree_tall", "fit": ("y", 7.60)}],
    },
    {
        "file": "grass_tuft.obj",
        "what": "трава",
        "parts": [{"kit": "nature", "name": "grass", "fit": ("y", 0.35)}],
    },
    {
        "file": "bush.obj",
        "what": "куст",
        "parts": [
            {
                "kit": "nature",
                "name": "plant_bushLarge",
                "fit": ("y", 0.80),
                "repaint": {"grass": ("leaf", 1)},
            }
        ],
    },
    {
        "file": "berry_bush.obj",
        "what": "ягодный куст (единственное насыщенное пятно в атласе)",
        "parts": [
            {
                "kit": "nature",
                "name": "plant_bush",
                "fit": ("y", 0.70),
                "repaint": {"grass": ("leaf", 0)},
                "berries": 9,
            }
        ],
    },
    {
        "file": "boulder.obj",
        "what": "валун",
        "parts": [{"kit": "nature", "name": "stone_largeA", "fit": ("x", 1.60)}],
    },
    {
        "file": "boulder_small.obj",
        "what": "валун, малый",
        "parts": [{"kit": "nature", "name": "stone_smallB", "fit": ("x", 0.70)}],
    },
]

KIT_DIRS = {
    "nature": "nature/Models/OBJ format",
    "survival": "survival/Models/OBJ format",
    "blocky": "blocky/Models/OBJ format",
}

KIT_CREDITS = {
    "nature": "Kenney Nature Kit (CC0) — https://kenney.nl/assets/nature-kit",
    "survival": "Kenney Survival Kit (CC0) — https://kenney.nl/assets/survival-kit",
    "blocky": (
        "Kenney Blocky Characters (CC0) — https://kenney.nl/assets/blocky-characters"
    ),
}

MAX_TRIANGLES = 1500


# --------------------------------------------------------------------------
# Building one model
# --------------------------------------------------------------------------


def berry_shape(center, radius):
    """An octahedron: eight triangles is all a berry needs at this scale."""
    cx, cy, cz = center
    r = radius
    tips = [
        (cx + r, cy, cz),
        (cx - r, cy, cz),
        (cx, cy + r, cz),
        (cx, cy - r, cz),
        (cx, cy, cz + r),
        (cx, cy, cz - r),
    ]
    faces = [
        (0, 2, 4),
        (4, 2, 1),
        (1, 2, 5),
        (5, 2, 0),
        (0, 4, 3),
        (4, 1, 3),
        (1, 5, 3),
        (5, 0, 3),
    ]
    return [[tips[i] for i in f] for f in faces]


def build_part(kits, part):
    """Return the part's triangles as `[(points, normals, (family, tier))]`."""
    kit = part["kit"]
    directory = kits / KIT_DIRS[kit]
    source = load_obj(directory / f"{part['name']}.obj")
    colormap = None
    if kit == "survival":
        colormap = read_png(directory / "Textures" / "colormap.png")

    keep = part.get("only")
    faces = [f for f in source.faces if keep is None or f[0] in keep]
    if not faces:
        raise SystemExit(f"{part['name']}: no faces left after `only` filter")

    positions = list(source.positions)
    normals = list(source.normals)

    # 1. Pose: swing whole groups around their joint, in source units.
    for group, axis, degrees, pivot in part.get("pose", []):
        touched = {
            pi for g, _, idx in faces if g == group for pi, _, _ in idx
        }
        for pi in touched:
            p = positions[pi - 1]
            local = tuple(p[k] - pivot[k] for k in range(3))
            spun = rotate(local, axis, degrees)
            positions[pi - 1] = tuple(spun[k] + pivot[k] for k in range(3))
        spun_normals = {
            ni for g, _, idx in faces if g == group for _, _, ni in idx if ni
        }
        for ni in spun_normals:
            normals[ni - 1] = rotate(normals[ni - 1], axis, degrees)

    used = {pi for _, _, idx in faces for pi, _, _ in idx}
    extent = bounds([positions[pi - 1] for pi in used])

    # 2. Uniform scale, from either an explicit factor or a target size.
    if "scale" in part:
        scale = part["scale"]
    else:
        axis, meters = part["fit"]
        k = "xyz".index(axis)
        span = extent[k][1] - extent[k][0]
        if span <= 0.0:
            raise SystemExit(f"{part['name']}: nothing to measure along {axis}")
        scale = meters / span

    rotations = part.get("rotate", [])
    offset = part.get("at", (0.0, 0.0, 0.0))
    # A part is placed by its own footprint: centered on X and Z, standing on
    # y = 0, so `at` reads as "put this piece here", in meters.
    if part.get("center", True):
        origin = (
            (extent[0][0] + extent[0][1]) * 0.5,
            extent[1][0],
            (extent[2][0] + extent[2][1]) * 0.5,
        )
    else:
        origin = (0.0, 0.0, 0.0)

    def place(point):
        p = tuple((point[k] - origin[k]) * scale for k in range(3))
        for axis, degrees in rotations:
            p = rotate(p, axis, degrees)
        return tuple(p[k] + offset[k] for k in range(3))

    def place_normal(normal):
        n = normal
        for axis, degrees in rotations:
            n = rotate(n, axis, degrees)
        return normalize(n)

    repaint = part.get("repaint", {})
    mtl_colors = load_mtl_colors(directory / f"{part['name']}.mtl")
    triangles = []
    for group, material, idx in faces:
        if group in repaint:
            cell = repaint[group]
        elif material in repaint:
            cell = repaint[material]
        elif kit == "blocky":
            cell = CHARACTER_PARTS[group]
        elif kit == "survival":
            uvs = [source.uvs[ti - 1] for _, ti, _ in idx if ti]
            if not uvs:
                raise SystemExit(f"{part['name']}: a face with no UV to sample")
            u = sum(p[0] for p in uvs) / len(uvs)
            v = sum(p[1] for p in uvs) / len(uvs)
            height = len(colormap)
            width = len(colormap[0])
            x = min(width - 1, max(0, int(u * width)))
            y = min(height - 1, max(0, int((1.0 - v) * height)))
            cell = classify_colormap(colormap[y][x])
        else:
            if material not in NATURE_MATERIALS:
                raise SystemExit(
                    f"{part['name']}: material {material!r} "
                    f"(Kd {mtl_colors.get(material)}) has no palette entry"
                )
            cell = NATURE_MATERIALS[material]

        points = [place(positions[pi - 1]) for pi, _, _ in idx]
        if all(ni for _, _, ni in idx):
            face_normals = [place_normal(normals[ni - 1]) for _, _, ni in idx]
        else:
            face_normals = [face_normal(points)] * len(points)
        # Fan-triangulate: the kits ship quads, `Mesh::from_obj` would do the
        # same thing on load, and doing it here keeps the file explicit.
        for i in range(1, len(points) - 1):
            triangles.append(
                (
                    [points[0], points[i], points[i + 1]],
                    [face_normals[0], face_normals[i], face_normals[i + 1]],
                    cell,
                )
            )

    count = part.get("berries", 0)
    if count:
        top = bounds([p for tri in triangles for p in tri[0]])
        radius = (top[0][1] - top[0][0]) * 0.055
        for i in range(count):
            angle = i * 2.399963  # golden angle, so berries do not line up
            spread = 0.34 + 0.11 * ((i * 7) % 5)
            cx = (top[0][0] + top[0][1]) * 0.5 + math.cos(angle) * spread * (
                top[0][1] - top[0][0]
            ) * 0.5
            cz = (top[2][0] + top[2][1]) * 0.5 + math.sin(angle) * spread * (
                top[2][1] - top[2][0]
            ) * 0.5
            cy = top[1][0] + (top[1][1] - top[1][0]) * (0.45 + 0.13 * ((i * 3) % 4))
            for points in berry_shape((cx, cy, cz), radius):
                normal = face_normal(points)
                triangles.append((points, [normal] * 3, ("berry", 1)))

    return triangles


def build_model(kits, entry):
    triangles = []
    for part in entry["parts"]:
        triangles += build_part(kits, part)

    extent = bounds([p for tri in triangles for p in tri[0]])
    shift = (
        -(extent[0][0] + extent[0][1]) * 0.5,
        -extent[1][0],  # the support sits on y = 0
        -(extent[2][0] + extent[2][1]) * 0.5,
    )
    return [
        ([tuple(p[k] + shift[k] for k in range(3)) for p in points], normals, cell)
        for points, normals, cell in triangles
    ]


def write_obj(path, entry, triangles):
    positions, normals, uvs = {}, {}, {}
    lines = []

    def index(table, key, out, fmt):
        if key not in table:
            table[key] = len(table) + 1
            out.append(fmt)
        return table[key]

    vlines, vtlines, vnlines, flines = [], [], [], []
    for points, face_normals, cell in triangles:
        idx = []
        for point, normal in zip(points, face_normals):
            pkey = tuple(round(c, 6) + 0.0 for c in point)
            pi = index(
                positions, pkey, vlines, "v %.6f %.6f %.6f" % pkey
            )
            nkey = tuple(round(c, 6) + 0.0 for c in normal)
            ni = index(normals, nkey, vnlines, "vn %.6f %.6f %.6f" % nkey)
            ti = index(uvs, cell, vtlines, "vt %.6f %.6f" % cell_uv(*cell))
            idx.append((pi, ti, ni))
        flines.append("f " + " ".join(f"{p}/{t}/{n}" for p, t, n in idx))

    sources = sorted({KIT_CREDITS[p["kit"]] for p in entry["parts"]})
    lines.append(f"# {entry['file']} — {entry['what']}")
    lines.append("# Generated by tools/example-assets/build_assets.py; see")
    lines.append("# examples/valley/assets/CREDITS.txt. Colors are cells of")
    lines.append("# examples/valley/assets/textures/valley_atlas.png — no materials here.")
    for source in sources:
        lines.append(f"# Source: {source}")
    lines += vlines + vtlines + vnlines + flines
    path.write_text("\n".join(lines) + "\n")
    return len(positions), len(triangles)


def write_credits(path, rows, atlas_name):
    lines = [
        "examples/valley/assets — где что взято и на каких условиях.",
        "",
        "Все исходники — наборы Kenney под CC0 (public domain): их можно",
        "использовать и переделывать без ограничений, указание автора",
        "желательно, но не обязательно. Ниже — строка на каждый",
        "закоммиченный файл: чем он был до конвертации и что с ним сделано.",
        "Конвертация: tools/example-assets/build_assets.py.",
        "",
        "Наборы:",
    ]
    for credit in KIT_CREDITS.values():
        lines.append(f"  {credit}")
    lines += ["", "models/:"]
    for file, what, sources in rows:
        lines.append(f"  models/{file} — {what}; из {sources}; CC0 (Kenney).")
    lines += [
        "",
        "textures/:",
        f"  textures/{atlas_name} — атлас плоских цветов долины; оригинальная",
        "    работа для этого репозитория по палитре карточки #33 и",
        "    crates/runity/examples/valley/look.rs; CC0.",
        "",
    ]
    path.write_text("\n".join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--kits",
        type=Path,
        required=True,
        help="directory holding the unzipped nature/, survival/ and blocky/ kits",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[2] / "examples" / "valley" / "assets",
        help="where to write models/, textures/ and CREDITS.txt",
    )
    args = parser.parse_args()

    models_dir = args.out / "models"
    textures_dir = args.out / "textures"
    models_dir.mkdir(parents=True, exist_ok=True)
    textures_dir.mkdir(parents=True, exist_ok=True)

    atlas_name = "valley_atlas.png"
    write_png(textures_dir / atlas_name, build_atlas())
    print(f"{atlas_name}: {ATLAS_SIZE}x{ATLAS_SIZE}, {len(ATLAS_ROWS)} palette rows")

    rows = []
    for entry in MANIFEST:
        triangles = build_model(args.kits, entry)
        if len(triangles) > MAX_TRIANGLES:
            raise SystemExit(f"{entry['file']}: {len(triangles)} triangles is too many")
        vertices, count = write_obj(models_dir / entry["file"], entry, triangles)
        extent = bounds([p for tri in triangles for p in tri[0]])
        size = tuple(round(hi - lo, 2) for lo, hi in extent)
        names = ", ".join(f"{p['kit']}/{p['name']}" for p in entry["parts"])
        rows.append((entry["file"], entry["what"], names))
        print(f"{entry['file']}: {count} tris, {vertices} verts, {size} m — {names}")

    write_credits(args.out / "CREDITS.txt", rows, atlas_name)
    print(f"{len(rows)} models, CREDITS.txt written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
