"""Write examples/valley/scenes/lanterns.ron: a courtyard made for the rays.

A pergola of thin beams, lattice screens (mashrabiya), a colonnade and a
score of lanterns swinging on their hooks. Everything that makes shadow
maps struggle and rays shine: thin bars casting a pattern that is sharp
where it starts and soft further off, and many moving lamps, each with its
own shadow of every bar.

Deterministic: the same file each time.

    python3 tools/example-assets/lanterns.py
"""
import hashlib
import pathlib
import random

rng = random.Random(11)
lines = []


def ident(name):
    return hashlib.sha1(("lanterns/" + name).encode()).hexdigest()[:16]


def f(v):
    return "(%s)" % ", ".join("%.3f" % c for c in v)


def box(name, at, size, colour, extra=""):
    lines.append(
        '        (id: "%s", name: "%s", model: "builtin:cube", transform: (position: %s, scale: %s), '
        "material: (base_color: %s%s))," % (ident(name), name, f(at), f(size), f(colour), extra)
    )


STONE = (0.5, 0.42, 0.34)
WALL = (0.66, 0.58, 0.47)
WOOD = (0.34, 0.22, 0.13)

# The ground and two walls: the west left open, for the low sun to come in.
lines.append(
    '        (id: "%s", name: "floor", model: "builtin:plane", transform: (scale: (22.0, 1.0, 22.0)), '
    "material: (base_color: %s, smoothness: 0.35))," % (ident("floor"), f(STONE))
)
box("back wall", (0, 2.5, -8), (16, 5, 0.4), WALL)
box("right wall", (8, 2.5, 0), (0.4, 5, 16), WALL)

# The pergola: four posts, beams along x every half metre, rafters along z.
for i, (x, z) in enumerate([(-4, -5), (4, -5), (-4, 3), (4, 3)]):
    box("post %d" % i, (x, 1.6, z), (0.25, 3.2, 0.25), WOOD)
for i in range(17):
    x = -4 + i * 0.5
    box("beam %d" % i, (x, 3.3, -1), (0.09, 0.18, 8.6), WOOD)
for i in range(9):
    z = -5 + i * 1.0
    box("rafter %d" % i, (0, 3.5, z), (8.6, 0.12, 0.09), WOOD)

# Lattice screens: bars both ways, a gap between each.
screens = [((-2.5, 0, 1.0), 0), ((2.5, 0, 1.0), 0), ((-6.0, 0, -4.0), 90), ((6.0, 0, -4.0), 90)]
for s, ((sx, _, sz), turn) in enumerate(screens):
    across = 2.0
    high = 2.4
    for i in range(9):
        o = -across / 2 + (i + 0.5) * across / 9
        at = (sx + o, high / 2, sz) if turn == 0 else (sx, high / 2, sz + o)
        size = (0.05, high, 0.05)
        box("screen %d bar %d" % (s, i), at, size, WOOD)
    for j in range(11):
        y = 0.15 + j * (high - 0.3) / 10
        at = (sx, y, sz)
        size = (across, 0.05, 0.05) if turn == 0 else (0.05, 0.05, across)
        box("screen %d rail %d" % (s, j), at, size, WOOD)

# A colonnade along the back wall.
for i in range(7):
    x = -6 + i * 2
    lines.append(
        '        (id: "%s", name: "column %d", model: "builtin:cylinder", transform: (position: (%.1f, 2.0, -6.8), '
        "scale: (0.45, 4.0, 0.45)), material: (base_color: %s))," % (ident("column %d" % i), i, x, f(WALL))
    )

# A pool of water with a low rim: its waves bend the reflections and the
# tiled bottom seen through them. A chrome ball and a bronze one, and glass.
RIM = (0.6, 0.55, 0.48)
box("pool bottom", (0, 0.02, 1.8), (3.0, 0.04, 4.4), (0.18, 0.42, 0.46))
box("pool rim north", (0, 0.22, -0.55), (3.6, 0.44, 0.3), RIM)
box("pool rim south", (0, 0.22, 4.15), (3.6, 0.44, 0.3), RIM)
box("pool rim west", (-1.65, 0.22, 1.8), (0.3, 0.44, 4.4), RIM)
box("pool rim east", (1.65, 0.22, 1.8), (0.3, 0.44, 4.4), RIM)
lines.append(
    '        (id: "%s", name: "water", model: "builtin:plane", transform: (position: (0.0, 0.36, 1.8), '
    "scale: (3.0, 1.0, 4.4)), material: (base_color: (0.04, 0.16, 0.2), shading: Water, wind: 0.6, clarity: 2.5, foam: 0.0)),"
    % ident("water")
)
lines.append(
    '        (id: "%s", name: "chrome ball", model: "builtin:sphere", transform: (position: (-2.6, 0.5, 2.8), '
    "scale: (1.0, 1.0, 1.0)), material: (base_color: (0.95, 0.95, 0.95), metallic: 1.0, smoothness: 0.98))," % ident("chrome ball")
)
lines.append(
    '        (id: "%s", name: "bronze ball", model: "builtin:sphere", transform: (position: (2.6, 0.4, 3.6), '
    "scale: (0.8, 0.8, 0.8)), material: (base_color: (0.8, 0.55, 0.3), metallic: 1.0, smoothness: 0.8))," % ident("bronze ball")
)
lines.append(
    '        (id: "%s", name: "glass ball", model: "builtin:sphere", transform: (position: (-2.9, 0.45, 4.9), '
    "scale: (0.9, 0.9, 0.9)), material: (base_color: (0.92, 0.97, 1.0), alpha: 0.12, surface: Transparent, smoothness: 0.98))," % ident("glass ball")
)
lines.append(
    '        (id: "%s", name: "glass block", model: "builtin:cube", transform: (position: (3.0, 0.5, 5.4), rotation_deg: (0.0, 30.0, 0.0), '
    "scale: (0.9, 1.0, 0.9)), material: (base_color: (0.85, 0.95, 0.9), alpha: 0.12, surface: Transparent, smoothness: 0.98))," % ident("glass block")
)

# Things on the floor for the shadows to fall over.
box("bench left", (-5.5, 0.25, 1.5), (0.6, 0.5, 2.4), WOOD)
box("bench right", (5.5, 0.25, 1.5), (0.6, 0.5, 2.4), WOOD)
lines.append(
    '        (id: "%s", name: "urn", model: "builtin:sphere", transform: (position: (0.0, 0.6, -2.0), '
    "scale: (1.2, 1.2, 1.2)), material: (base_color: (0.55, 0.3, 0.18), smoothness: 0.6))," % ident("urn")
)

# Lanterns on their hooks, each swinging a little, none in step.
spots = [(-3, -4), (0, -4), (3, -4), (-3, -1.5), (3, -1.5), (-1.5, 0.2), (1.5, 0.2),
         (-6.0, -1), (6.8, -1), (-6.0, 3), (6.8, 3), (0, 3.5), (-3.5, 5), (3.5, 5)]
for i, (x, z) in enumerate(spots):
    y = 2.3 + rng.uniform(-0.25, 0.25)
    warm = (1.0, 0.62 + rng.uniform(-0.06, 0.06), 0.3 + rng.uniform(-0.05, 0.05))
    swing = rng.uniform(0.18, 0.32)
    axis = rng.choice([(1, 0), (0, 1)])
    points = [(-swing * axis[0], -0.03, -swing * axis[1]), (0, 0, 0), (swing * axis[0], -0.03, swing * axis[1])]
    lines.append(
        '        (id: "%s", name: "lantern %d", model: "builtin:sphere", transform: (position: (%.2f, %.2f, %.2f), '
        "scale: (0.14, 0.14, 0.14)), material: (base_color: %s, shading: Unlit, emission: %s), "
        "light: (color: %s, intensity: 2.2, range: 4.5), "
        "route: (points: [%s], speed: %.2f, ends: Back, smooth: true)),"
        % (ident("lantern %d" % i), i, x, y, z, f(warm), f(tuple(c * 8 for c in warm)), f(warm),
           ", ".join(f(p) for p in points), rng.uniform(0.25, 0.45))
    )

head = """// Двор для лучей (`runity::ray`): пергола из тонких балок, резные ширмы
// (машрабия), колоннада и полтора десятка фонарей, качающихся на крюках.
// То, на чём карты теней сдаются, а лучи сияют: решётка кладёт узор, резкий
// у основания и мягкий дальше, и каждый фонарь — свою тень от каждой
// планки. Сгенерирована tools/example-assets/lanterns.py.
(
    view: (position: (0.0, 1.6, 7.0), target: (0.0, 1.2, -4.0), fov_deg: 60.0),
    sun: (hour: 16.0, intensity: 1.3),
    sky: (mode: Physical),
    post: (bloom: (intensity: 0.4)),
    ray_tracing: (sun_shadows: true, light_shadows: true, ambient_occlusion: true, sun_size: 0.6, sun_rays: 4, occlusion_rays: 6, occlusion_radius: 1.2, lamp_size: 0.07, reflections: true, reflection_roughness: 0.5, refractions: true),
    entities: [
"""
out = pathlib.Path(__file__).resolve().parents[2] / "examples/valley/scenes/lanterns.ron"
out.write_text(head + "\n".join(lines) + "\n    ],\n)\n")
print(f"{out}: {len(lines)} entities")
