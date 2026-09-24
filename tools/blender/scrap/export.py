"""scrap for Blender: the exporter (docs/blender.md).

Sends what Blender evaluated — meshes with their modifiers applied, the
object tree, materials, images and the objects' scrap properties — to the
engine's importer, which builds the assets. Nothing is written to disk: the
importer listens on a port of this machine and the stream goes there.

Run by the importer as

    blender --background --factory-startup level.blend \
        --python-expr "<this file>" -- --port 50123

and imported by the add-on in this folder, which stamps IDs on save.

The stream: b"SCRAP_BL", a u32 version, a u32 length and that much JSON,
then a u32 count of blobs, each a u64 length and its bytes, little-endian.
The JSON names blobs by index. Coordinates stay Blender's (Z up); the
importer turns them.
"""

import array
import json
import os
import random
import socket
import struct
import sys

import bpy
from mathutils import Matrix

VERSION = 1
MAGIC = b"SCRAP_BL"
PROP = "scrap.id"
COMPONENT = "scrap."
# An object's scrap settings that are not the game's components.
COLLIDER = "scrap.collider"
MATERIAL = "scrap.material"
RESERVED = {PROP, COLLIDER, MATERIAL}


def hints():
    """What the editor wrote for this project: its materials and the
    game's components with their shapes (library/blender.json). Empty when
    the file is in no project, or no editor has written it yet."""
    root = project_root(bpy.data.filepath) if bpy.data.filepath else None
    if root is None:
        return {}
    try:
        with open(os.path.join(root, "library", "blender.json")) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def ron_value(value, shape):
    """A field's value as RON, by what the game says it holds: a text is
    quoted, an enum's variant is bare, a number keeps its kind."""
    if isinstance(value, bool) or shape == "Bool":
        return "true" if value else "false"
    if shape == "Int":
        return str(int(value))
    if shape == "Float" or isinstance(value, float):
        text = repr(float(value))
        return text if ("." in text or "e" in text or "inf" in text or "nan" in text) else text + ".0"
    if isinstance(value, int):
        return str(value)
    if shape == "Text" or (shape is None and isinstance(value, str)):
        return json.dumps(str(value))
    # An enum's variant, a list, a nested struct: written as RON already.
    return str(value)


def struct_fields(shape):
    """A struct shape's fields as {name: shape}, else None."""
    if isinstance(shape, dict) and "Struct" in shape:
        return {name: field for name, field in shape["Struct"]}
    return None


def components_of(obj, shapes):
    """The game's components on an object, as {name: RON}. A component is
    either one property holding its whole value (scrap.door =
    "(open_angle: 90.0)") or a property per field (scrap.door.open_angle
    = 90.0), which is what the scrap panel writes: Blender draws each with
    the widget its type wants."""
    out = {}
    fields = {}
    for key in obj.keys():
        if not key.startswith(COMPONENT) or key in RESERVED:
            continue
        rest = key[len(COMPONENT):]
        if "." in rest:
            name, field = rest.split(".", 1)
            fields.setdefault(name, {})[field] = obj[key]
        else:
            value = obj[key]
            out[rest] = value if isinstance(value, str) else ron_value(value, None)
    for name, values in fields.items():
        shape = struct_fields((shapes.get(name) or {}).get("shape")) or {}
        body = ", ".join(
            "%s: %s" % (field, ron_value(values[field], shape.get(field)))
            for field in sorted(values)
        )
        out[name] = "(%s)" % body
    return out


def stamp():
    """Give every object, mesh, material and collection of this file an ID
    it keeps through renames: what the engine's scenes point at. A copy
    that carried its original's ID along (Shift+D) gets its own."""
    for things in (bpy.data.objects, bpy.data.meshes, bpy.data.materials,
                   bpy.data.collections):
        seen = set()
        for thing in things:
            if thing.library is not None:
                continue
            value = thing.get(PROP)
            if not isinstance(value, str) or len(value) != 16 or value in seen:
                value = "%016x" % random.getrandbits(64)
                while value in seen or value == "0" * 16:
                    value = "%016x" % random.getrandbits(64)
                thing[PROP] = value
            seen.add(value)


class Blobs:
    def __init__(self):
        self.items = []

    def add(self, data):
        self.items.append(bytes(data))
        return len(self.items) - 1


class Exporter:
    def __init__(self):
        self.depsgraph = bpy.context.evaluated_depsgraph_get()
        self.blobs = Blobs()
        self.images = []
        self.image_index = {}
        self.materials = []
        self.material_index = {}
        self.meshes = []
        self.mesh_index = {}
        self.shapes = hints().get("components", {})

    # --- images and materials -----------------------------------------

    def image(self, image):
        if image is None:
            return None
        if image.name in self.image_index:
            return self.image_index[image.name]
        width, height = image.size
        if width == 0 or height == 0:
            return None
        pixels = array.array("f", [0.0]) * (width * height * 4)
        image.pixels.foreach_get(pixels)
        self.images.append({
            "key": image.get(PROP) or image.name,
            "name": image.name.rsplit(".", 1)[0] if "." in image.name else image.name,
            "width": width,
            "height": height,
            "pixels": self.blobs.add(pixels.tobytes()),
        })
        index = len(self.images) - 1
        self.image_index[image.name] = index
        return index

    def linked_image(self, socket):
        """The image a socket reads, and from which channel: a picture
        straight in (red, for a grey map), or one channel of it through
        Separate Color."""
        if socket is None or not socket.is_linked:
            return None
        link = socket.links[0]
        node = link.from_node
        if node.type == "TEX_IMAGE":
            index = self.image(node.image)
            return None if index is None else (index, 0)
        if node.type in ("SEPARATE_COLOR", "SEPRGB"):
            channel = {"Red": 0, "R": 0, "Green": 1, "G": 1, "Blue": 2, "B": 2}.get(
                link.from_socket.name, 0)
            source = self.linked_image(node.inputs[0])
            return None if source is None else (source[0], channel)
        return None

    def material(self, material):
        if material is None:
            return None
        if material.name in self.material_index:
            return self.material_index[material.name]
        out = {
            "key": material.get(PROP) or material.name,
            "name": material.name,
            "base_color": list(material.diffuse_color)[:3],
            "alpha": material.diffuse_color[3],
            "metallic": material.metallic,
            "roughness": material.roughness,
            "transparent": getattr(material, "surface_render_method", "") == "BLENDED",
            "double_sided": not material.use_backface_culling,
        }
        bsdf = None
        if material.node_tree is not None:
            for node in material.node_tree.nodes:
                if node.type == "BSDF_PRINCIPLED":
                    bsdf = node
                    break
        if bsdf is not None:
            def value(name, default):
                socket = bsdf.inputs.get(name)
                return default if socket is None else socket.default_value

            base = bsdf.inputs.get("Base Color")
            out["base_color"] = list(value("Base Color", (0.8, 0.8, 0.8, 1.0)))[:3]
            base_map = self.linked_image(base)
            if base_map is not None:
                out["base_map"] = base_map[0]
                out["base_color"] = [1.0, 1.0, 1.0]
            out["alpha"] = value("Alpha", 1.0)
            out["metallic"] = value("Metallic", 0.0)
            out["roughness"] = value("Roughness", 0.5)
            metallic_map = self.linked_image(bsdf.inputs.get("Metallic"))
            if metallic_map is not None:
                out["metallic_map"] = list(metallic_map)
                out["metallic"] = 1.0
            roughness_map = self.linked_image(bsdf.inputs.get("Roughness"))
            if roughness_map is not None:
                out["roughness_map"] = list(roughness_map)
                out["roughness"] = 1.0
            strength = value("Emission Strength", 0.0)
            colour = list(value("Emission Color", (0.0, 0.0, 0.0, 1.0)))[:3]
            out["emission"] = [c * strength for c in colour]
            emission_map = self.linked_image(bsdf.inputs.get("Emission Color"))
            if emission_map is not None:
                out["emission_map"] = emission_map[0]
                out["emission"] = [strength] * 3
            normal = bsdf.inputs.get("Normal")
            if normal is not None and normal.is_linked:
                node = normal.links[0].from_node
                if node.type == "NORMAL_MAP":
                    found = self.linked_image(node.inputs.get("Color"))
                    if found is not None:
                        out["normal_map"] = found[0]
                        out["normal_scale"] = node.inputs["Strength"].default_value
        # EEVEE draws alpha below one see-through under either render
        # method: Blended, and Dithered, the default since Blender 4.2.
        out["transparent"] = out["transparent"] or out["alpha"] < 1.0
        self.materials.append(out)
        index = len(self.materials) - 1
        self.material_index[material.name] = index
        return index

    # --- meshes --------------------------------------------------------

    def mesh(self, obj):
        """The mesh an object draws, once per mesh block: linked duplicates
        share it, and so share the engine's model. An object whose
        modifiers change it has one of its own."""
        if obj.type not in ("MESH", "CURVE", "SURFACE", "META", "FONT"):
            return None
        shared = obj.type == "MESH" and len(obj.modifiers) == 0
        if shared:
            key = "data:" + (obj.data.get(PROP) or obj.data.name)
            name = obj.data.name
        else:
            key = "object:" + (obj.get(PROP) or obj.name)
            name = obj.name
        if key in self.mesh_index:
            return self.mesh_index[key]
        evaluated = obj.evaluated_get(self.depsgraph)
        me = evaluated.to_mesh()
        try:
            if me is None or len(me.polygons) == 0:
                self.mesh_index[key] = None
                return None
            me.calc_loop_triangles()
            n_vertices, n_loops, n_tris = len(me.vertices), len(me.loops), len(me.loop_triangles)
            positions = array.array("f", [0.0]) * (n_vertices * 3)
            me.vertices.foreach_get("co", positions)
            corners = array.array("i", [0]) * n_loops
            me.loops.foreach_get("vertex_index", corners)
            normals = array.array("f", [0.0]) * (n_loops * 3)
            me.corner_normals.foreach_get("vector", normals)
            triangles = array.array("i", [0]) * (n_tris * 3)
            me.loop_triangles.foreach_get("loops", triangles)
            slots = array.array("i", [0]) * n_tris
            me.loop_triangles.foreach_get("material_index", slots)
            uvs = None
            if me.uv_layers.active is not None:
                data = array.array("f", [0.0]) * (n_loops * 2)
                me.uv_layers.active.data.foreach_get("uv", data)
                uvs = self.blobs.add(data.tobytes())
            materials = [self.material(slot.material) for slot in obj.material_slots]
            self.meshes.append({
                "key": key,
                "name": name,
                "positions": self.blobs.add(positions.tobytes()),
                "corners": self.blobs.add(corners.tobytes()),
                "normals": self.blobs.add(normals.tobytes()),
                "uvs": uvs,
                "triangles": self.blobs.add(triangles.tobytes()),
                "slots": self.blobs.add(slots.tobytes()),
                "materials": materials,
            })
        finally:
            evaluated.to_mesh_clear()
        index = len(self.meshes) - 1
        self.mesh_index[key] = index
        return index

    # --- the tree ------------------------------------------------------

    def node(self, obj, matrix, allowed):
        location, rotation, scale = matrix.decompose()
        out = {
            "name": obj.name,
            "id": obj.get(PROP),
            "location": list(location),
            "rotation": [rotation.w, rotation.x, rotation.y, rotation.z],
            "scale": list(scale),
            "mesh": self.mesh(obj),
            "prefab": None,
            "components": {},
            "collider": obj.name.endswith("-col") or bool(obj.get(COLLIDER, False)),
            "material": obj.get(MATERIAL) or None,
            "children": [],
        }
        out["components"] = components_of(obj, self.shapes)
        collection = obj.instance_collection if obj.instance_type == "COLLECTION" else None
        if collection is not None:
            if collection.asset_data is not None or collection.library is not None:
                # An asset's instance: that asset's prefab, not a copy.
                out["prefab"] = collection.name
            else:
                out["children"].extend(self.collection_roots(collection))
        for child in obj.children:
            if child in allowed:
                out["children"].append(self.node(child, child.matrix_local, allowed))
        return out

    def collection_roots(self, collection):
        """A collection's objects as a tree, placed from its instance
        offset — where an instance of it puts its origin."""
        members = set(collection.all_objects)
        offset = Matrix.Translation(-collection.instance_offset)
        return [
            self.node(obj, offset @ obj.matrix_world, members)
            for obj in collection.all_objects
            if obj.parent is None or obj.parent not in members
        ]

    def scene(self):
        assets = []
        in_assets = set()
        for collection in bpy.data.collections:
            if collection.asset_data is not None and collection.library is None:
                in_assets.update(collection.all_objects)
                assets.append({
                    "key": collection.get(PROP) or collection.name,
                    "name": collection.name,
                    "roots": self.collection_roots(collection),
                })
        for obj in bpy.data.objects:
            if obj.asset_data is not None and obj.library is None:
                _, rotation, scale = obj.matrix_world.decompose()
                own = Matrix.LocRotScale(None, rotation, scale)
                members = set(obj.children_recursive)
                assets.append({
                    "key": obj.get(PROP) or obj.name,
                    "name": obj.name,
                    "roots": [self.node(obj, own, members)],
                })
        visible = set(bpy.context.view_layer.objects)
        level_objects = {o for o in visible if o not in in_assets}
        level = []
        for obj in sorted(level_objects, key=lambda o: o.name):
            if obj.parent is not None and obj.parent in level_objects:
                continue
            node = self.node(obj, obj.matrix_world, level_objects)
            if obj.asset_data is not None and obj.library is None:
                # Placed where it stands; what it is comes from its prefab.
                node["prefab"] = obj.name
                node["mesh"] = None
                node["children"] = []
            level.append(node)
        return {
            "version": VERSION,
            "images": self.images,
            "materials": self.materials,
            "meshes": self.meshes,
            "level": level,
            "assets": assets,
        }


def stream():
    exporter = Exporter()
    header = json.dumps(exporter.scene()).encode("utf-8")
    parts = [MAGIC, struct.pack("<I", VERSION), struct.pack("<I", len(header)), header,
             struct.pack("<I", len(exporter.blobs.items))]
    for blob in exporter.blobs.items:
        parts.append(struct.pack("<Q", len(blob)))
        parts.append(blob)
    return b"".join(parts)


def send(port):
    data = stream()
    with socket.create_connection(("127.0.0.1", port)) as connection:
        connection.sendall(data)


# --- the live link with an open editor ---------------------------------

LINK = b"SCRAP_LK"
SCENE, MOVES = 1, 2


def project_root(path):
    """The scrap project a file is in: the folder above it holding
    scrap.ron."""
    folder = os.path.dirname(os.path.abspath(path))
    while True:
        if os.path.isfile(os.path.join(folder, "scrap.ron")):
            return folder
        parent = os.path.dirname(folder)
        if parent == folder:
            return None
        folder = parent


def link_target():
    """Where an open editor listens for this file, and the file's path in
    its project: None when the file is in no project or no editor has it
    open."""
    path = bpy.data.filepath
    if not path:
        return None
    root = project_root(path)
    if root is None:
        return None
    try:
        with open(os.path.join(root, "library", "blender-link")) as f:
            port = int(f.read().strip())
    except (OSError, ValueError):
        return None
    return port, os.path.relpath(path, root).replace(os.sep, "/")


def send_link(kind, payload):
    """One message to the editor; False when none is listening."""
    target = link_target()
    if target is None:
        return False
    port, source = target
    source = source.encode("utf-8")
    message = LINK + struct.pack("<I", kind) + struct.pack("<I", len(source)) + source + payload
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5) as connection:
            connection.sendall(message)
        return True
    except OSError:
        return False


def moves(objects):
    """Where objects stand now, for the editor's preview: in their
    parent's space, or the world's for one without a parent — as the
    exporter places them."""
    out = []
    for obj in objects:
        stamp_id = obj.get(PROP)
        if not isinstance(stamp_id, str):
            continue
        matrix = obj.matrix_local if obj.parent is not None else obj.matrix_world
        location, rotation, scale = matrix.decompose()
        out.append({
            "id": stamp_id,
            "location": list(location),
            "rotation": [rotation.w, rotation.x, rotation.y, rotation.z],
            "scale": list(scale),
        })
    return json.dumps({"moves": out}).encode("utf-8")


def main(argv):
    args = argv[argv.index("--") + 1:] if "--" in argv else []
    if "--port" in args:
        send(int(args[args.index("--port") + 1]))


if "--port" in sys.argv:
    main(sys.argv)
