"""Write examples/valley/scenes/bazaar.ron: a desert market in the wind.

Awnings bellying over two stalls, flags and banners, a washing line with
sheets, bunting between the stalls, and a mud-brick wall that comes down:
cloth, ropes and a wall that comes down together (the soft module's
cloth and rope, the destruction module's fracture).

    python3 tools/example-assets/bazaar.py
"""
import hashlib
import pathlib

lines = []


def ident(name):
    return hashlib.sha1(("bazaar/" + name).encode()).hexdigest()[:16]


def f(v):
    return "(%s)" % ", ".join("%.3f" % c for c in v)


def thing(name, rest):
    lines.append('        (id: "%s", name: "%s", %s),' % (ident(name), name, rest))


WOOD = (0.36, 0.24, 0.14)
MUD = (0.66, 0.5, 0.36)
CANVAS = [(0.72, 0.16, 0.12), (0.18, 0.24, 0.52), (0.82, 0.62, 0.22), (0.9, 0.86, 0.76)]


def post(name, at, high, thick=0.12):
    thing(name, 'model: "builtin:cylinder", transform: (position: %s, scale: %s), material: (base_color: %s), body: Static, collider: Box(half: (0.5, 0.5, 0.5), center: (0.0, 0.0, 0.0))'
          % (f((at[0], high / 2, at[1])), f((thick, high, thick)), f(WOOD)))


# The ground, and what the blocks and heaps land on.
thing("ground", 'model: "builtin:plane", transform: (scale: (80.0, 1.0, 80.0)), material: (base_color: (0.78, 0.62, 0.42), shading: Sand)')
thing("ground body", 'transform: (position: (0.0, -0.5, 0.0)), body: Static, collider: Box(half: (40.0, 0.5, 40.0), center: (0.0, 0.0, 0.0))')

# Two stalls, each four posts and an awning pinned at its corners.
for s, (x, z, c) in enumerate([(-4.0, -2.0, 0), (3.5, -3.0, 1)]):
    w, d, h = 3.0, 2.4, 2.4
    for k, (dx, dz) in enumerate([(-w / 2, 0), (w / 2, 0), (-w / 2, -d), (w / 2, -d)]):
        post("stall %d post %d" % (s, k), (x + dx, z + dz), h)
    thing("stall %d awning" % s, 'transform: (position: %s), material: (base_color: %s, smoothness: 0.15), cloth: (size: (%.2f, %.2f), cells: (18, 14), pinned: Corners, catch: 0.7, stiffness: 10)'
          % (f((x, h, z)), f(CANVAS[c]), w, d))
    thing("stall %d counter" % s, 'model: "builtin:cube", transform: (position: %s, scale: (2.6, 0.9, 0.6)), material: (base_color: %s), body: Static, collider: Box(half: (0.5, 0.5, 0.5), center: (0.0, 0.0, 0.0))'
          % (f((x, 0.45, z - 0.4)), f(WOOD)))

# Flags on tall poles, and a banner hung from a crossbar.
for k, (x, z, c) in enumerate([(-8.0, -6.0, 0), (-1.0, -8.0, 2), (7.5, -7.0, 1)]):
    post("flag pole %d" % k, (x, z), 5.0, 0.08)
    thing("flag %d" % k, 'transform: (position: %s), material: (base_color: %s, smoothness: 0.1), cloth: (size: (1.8, 1.1), cells: (18, 11), pinned: Left, catch: 2.5)'
          % (f((x + 0.05, 4.9, z)), f(CANVAS[c])))
post("banner post left", (-0.4, -5.0), 3.6)
post("banner post right", (1.6, -5.0), 3.6)
thing("banner bar", 'model: "builtin:cube", transform: (position: (0.6, 3.55, -5.0), scale: (2.3, 0.08, 0.08)), material: (base_color: %s)' % f(WOOD))
thing("banner", 'transform: (position: (0.6, 3.5, -5.0)), material: (base_color: %s, smoothness: 0.1), cloth: (size: (1.8, 2.6), cells: (14, 20), pinned: Top, catch: 0.8)' % f(CANVAS[1]))

# A washing line with sheets, and bunting from stall to stall.
post("line post left", (-9.0, 2.0), 2.6, 0.09)
post("line post right", (-3.0, 3.2), 2.6, 0.09)
thing("washing line", 'transform: (position: (-9.0, 2.55, 2.0)), material: (base_color: (0.85, 0.8, 0.7)), rope: (to: (6.0, 0.0, 1.2), slack: 0.04, segments: 28, thickness: 0.02, catch: 0.5)')
for k in range(3):
    t = 0.2 + k * 0.28
    thing("sheet %d" % k, 'transform: (position: %s), material: (base_color: %s, smoothness: 0.1), cloth: (size: (1.1, 1.3), cells: (12, 14), pinned: Top, catch: 0.9)'
          % (f((-9.0 + 6.0 * t, 2.35 - 0.12 * (1 - abs(2 * t - 1)), 2.0 + 1.2 * t)), f(CANVAS[(k + 2) % 4])))
thing("bunting", 'transform: (position: (-2.5, 2.35, -2.0)), material: (base_color: (0.9, 0.3, 0.2)), rope: (to: (4.5, 0.0, -1.0), slack: 0.06, segments: 32, thickness: 0.018, catch: 0.6)')

# A mud-brick wall that comes down four seconds in, toward the camera.
thing("old wall", 'model: "builtin:cube", transform: (position: (6.0, 1.3, 2.5), rotation_deg: (0.0, -20.0, 0.0), scale: (4.0, 2.6, 0.45)), material: (base_color: %s), body: Static, collider: Box(half: (0.5, 0.5, 0.5), center: (0.0, 0.0, 0.0)), fracture: (pieces: 24, at: 4.0, from: (0.0, 0.3, -1.0), knock: 3.5, levels: 1)' % f(MUD))

head = """// Базар в пустыне на ветру (`cloth`, `rope` модуля soft, `fracture` модуля destruction): навесы
// над двумя лавками, флаги и знамя, бельевая верёвка с простынями,
// гирлянда между лавками и глинобитная стена, что рушится на четвёртой
// секунде. Сгенерирована tools/example-assets/bazaar.py.
(
    view: (position: (0.5, 3.4, 12.0), target: (-0.5, 1.4, -2.0), fov_deg: 60.0),
    sun: (hour: 16.2, intensity: 1.3, ground: (0.78, 0.62, 0.42)),
    sky: (mode: Physical),
    wind: (direction: (1.0, 0.0, 0.35), strength: 1.7),
    ambient_occlusion: (bounce: 1.0),
    entities: [
"""
out = pathlib.Path(__file__).resolve().parents[2] / "examples/valley/scenes/bazaar.ron"
out.write_text(head + "\n".join(lines) + "\n    ],\n)\n")
print(f"{out}: {len(lines)} entities")
