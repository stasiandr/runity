"""scrap for Blender (docs/blender.md).

Keep a .blend in a scrap project's assets/ and work in it: the engine
imports it on every save — models, materials, the object tree as a prefab,
what is marked as an asset as prefabs of their own.

This add-on:

* gives every object, mesh, material and collection an ID on save, kept
  through renames, so what the engine's scenes say about a part keeps
  landing on it;
* sends the saved file to the scrap editor, when one has the project
  open, so it is imported at once without starting another Blender;
* sends where objects are while they are being moved, so the editor shows
  the drag as it happens;
* has a scrap tab in the 3D view's sidebar (N): the game's components on
  the active object, a field per value; whether it collides; and a
  material of the project's to draw it with instead of Blender's.

The exporter the engine runs is export.py beside this file.
"""

import re

import bpy
from bpy.app.handlers import persistent

if "export" in locals():
    # Reload Scripts runs this file again but keeps export.py as it was
    # first imported: without this, a new exporter needs a new Blender.
    import importlib

    importlib.reload(export)
else:
    from . import export

_moved = set()
_flush_pending = False


@persistent
def _stamp(*_args):
    export.stamp()


@persistent
def _saved(*_args):
    if export.link_target() is not None:
        export.send_link(export.SCENE, export.stream())


def _flush():
    global _flush_pending
    _flush_pending = False
    objects = [bpy.data.objects.get(name) for name in _moved]
    _moved.clear()
    objects = [o for o in objects if o is not None]
    if objects:
        export.send_link(export.MOVES, export.moves(objects))
    return None


@persistent
def _changed(_scene, depsgraph):
    global _flush_pending
    for update in depsgraph.updates:
        if update.is_updated_transform and isinstance(update.id, bpy.types.Object):
            _moved.add(update.id.original.name)
    if _moved and not _flush_pending and export.link_target() is not None:
        # At most twenty a second, however fast Blender redraws.
        _flush_pending = True
        bpy.app.timers.register(_flush, first_interval=0.05)


# --- the scrap tab -------------------------------------------------------

# Blender keeps only a pointer to a dynamic enum's strings: they must live
# somewhere Python does not collect them.
_enum_items = {}

# The Game material menu's "Blender's own". Not "": an enum item with an
# empty identifier is a separator in a menu, and cannot be picked.
_OWN = "__blender__"


def _shape_default(shape):
    """A field's first value, by its shape, as a Blender property holds it."""
    if shape == "Bool":
        return False
    if shape == "Int":
        return 0
    if shape == "Float":
        return 0.0
    if shape == "Text":
        return ""
    return _example(shape)


def _example(shape):
    """The shortest RON of a shape: what the editor's Add Component starts
    from, spelled the same way."""
    if isinstance(shape, str):
        return {"Bool": "false", "Int": "0", "Float": "0.0", "Char": "'a'",
                "Text": '""', "Unit": "()", "Any": "()", "Entity": 'Entity("")'}.get(shape, "()")
    kind, value = next(iter(shape.items()))
    if kind == "Option":
        return "None"
    if kind == "List":
        return "[]"
    if kind == "Map":
        return "{}"
    if kind == "Tuple":
        return "(%s)" % ", ".join(_example(v) for v in value)
    if kind == "Struct":
        return "(%s)" % ", ".join("%s: %s" % (n, _example(v)) for n, v in value)
    if kind == "Enum":
        return value[0] if value else ""
    return "()"


def _components(obj):
    """The components on an object: {name: [field, …]}, an empty list for
    one held whole as RON."""
    out = {}
    for key in obj.keys():
        if not key.startswith(export.COMPONENT) or key in export.RESERVED:
            continue
        rest = key[len(export.COMPONENT):]
        name, _, field = rest.partition(".")
        fields = out.setdefault(name, [])
        if field:
            fields.append(field)
    return {name: sorted(fields) for name, fields in out.items()}


def _file_overrides():
    """The file's materials the game swaps for its own: `materials:` in the
    .scrimport beside it, which the engine keeps (docs/blender.md). Read, not
    written: that choice lives in the project, not in the .blend."""
    path = bpy.data.filepath
    if not path:
        return {}
    try:
        with open(path + ".scrimport") as f:
            text = f.read()
    except OSError:
        return {}
    block = re.search(r"materials:\s*\{([^}]*)\}", text)
    if block is None:
        return {}
    return dict(re.findall(r'"([^"]*)"\s*:\s*"([^"]*)"', block.group(1)))


def _component_items(_self, _context):
    names = sorted(export.hints().get("components", {}))
    items = [(n, n, "") for n in names]
    _enum_items["components"] = items
    return items or [("", "(open the project in the scrap editor)", "")]


def _material_items(_self, _context):
    names = export.hints().get("materials", [])
    items = [(_OWN, "Blender's own", "Draw with the material the file gives it")]
    items += [(n, n, "") for n in names]
    _enum_items["materials"] = items
    return items


class SCRAP_OT_add_component(bpy.types.Operator):
    """Add one of the game's components to the active object"""
    bl_idname = "scrap.add_component"
    bl_label = "Add Component"
    bl_options = {"REGISTER", "UNDO"}
    name: bpy.props.EnumProperty(name="Component", items=_component_items)

    def execute(self, context):
        obj = context.object
        if obj is None or not self.name:
            return {"CANCELLED"}
        info = export.hints().get("components", {}).get(self.name, {})
        fields = export.struct_fields(info.get("shape"))
        if fields:
            for field, shape in fields.items():
                key = "%s%s.%s" % (export.COMPONENT, self.name, field)
                if key not in obj:
                    obj[key] = _shape_default(shape)
        else:
            obj[export.COMPONENT + self.name] = info.get("example", "()")
        return {"FINISHED"}


class SCRAP_OT_remove_component(bpy.types.Operator):
    """Take this component off the active object"""
    bl_idname = "scrap.remove_component"
    bl_label = "Remove Component"
    bl_options = {"REGISTER", "UNDO"}
    name: bpy.props.StringProperty()

    def execute(self, context):
        obj = context.object
        if obj is None:
            return {"CANCELLED"}
        whole = export.COMPONENT + self.name
        for key in list(obj.keys()):
            if key == whole or key.startswith(whole + "."):
                del obj[key]
        return {"FINISHED"}


class SCRAP_OT_toggle_collider(bpy.types.Operator):
    """Collide as its own shape in the game: a static body"""
    bl_idname = "scrap.toggle_collider"
    bl_label = "Collider"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        obj = context.object
        if obj is None:
            return {"CANCELLED"}
        obj[export.COLLIDER] = not bool(obj.get(export.COLLIDER, False))
        return {"FINISHED"}


class SCRAP_OT_set_material(bpy.types.Operator):
    """Draw the active object in the game with a material of the project's"""
    bl_idname = "scrap.set_material"
    bl_label = "Game Material"
    bl_options = {"REGISTER", "UNDO"}
    material: bpy.props.EnumProperty(name="Material", items=_material_items)

    def execute(self, context):
        obj = context.object
        if obj is None:
            return {"CANCELLED"}
        if self.material and self.material != _OWN:
            obj[export.MATERIAL] = self.material
        elif export.MATERIAL in obj:
            del obj[export.MATERIAL]
        return {"FINISHED"}


class SCRAP_PT_panel(bpy.types.Panel):
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "scrap"
    bl_label = "scrap"

    def draw(self, context):
        layout = self.layout
        path = bpy.data.filepath
        root = export.project_root(path) if path else None
        if not path:
            layout.label(text="Save the file in a project's assets/", icon="INFO")
        elif root is None:
            layout.label(text="Not in a scrap project", icon="ERROR")
        elif export.link_target() is None:
            layout.label(text="Editor closed: imported when it opens", icon="UNLINKED")
        else:
            layout.label(text="Linked to the scrap editor", icon="LINKED")

        obj = context.object
        if obj is None:
            return
        box = layout.box()
        box.label(text=obj.name, icon="OBJECT_DATA")
        stamp = obj.get(export.PROP)
        box.label(text="ID " + (stamp if isinstance(stamp, str) else "given on save"))
        box.operator(
            "scrap.toggle_collider",
            text="Collider",
            icon="MOD_PHYSICS",
            depress=bool(obj.get(export.COLLIDER, False) or obj.name.endswith("-col")),
        )
        material = obj.get(export.MATERIAL)
        row = box.row()
        row.label(text="Game material")
        row.operator_menu_enum(
            "scrap.set_material", "material",
            text=material if isinstance(material, str) else "Blender's own",
        )
        if isinstance(material, str):
            row.operator("scrap.set_material", text="", icon="X").material = _OWN

        swapped = _file_overrides()
        slots = [slot.material for slot in obj.material_slots if slot.material is not None]
        if slots and not isinstance(material, str):
            for mat in slots:
                row = box.row()
                row.label(text=mat.name, icon="MATERIAL")
                row.label(text=("in the game: " + swapped[mat.name]) if mat.name in swapped else "as here")

        for name, fields in sorted(_components(obj).items()):
            component = layout.box()
            header = component.row()
            header.label(text=name, icon="PROPERTIES")
            header.operator("scrap.remove_component", text="", icon="X").name = name
            if fields:
                for field in fields:
                    component.prop(obj, '["%s%s.%s"]' % (export.COMPONENT, name, field), text=field)
            else:
                component.prop(obj, '["%s%s"]' % (export.COMPONENT, name), text="")
        layout.operator_menu_enum("scrap.add_component", "name", text="Add Component", icon="ADD")


CLASSES = (
    SCRAP_OT_add_component,
    SCRAP_OT_remove_component,
    SCRAP_OT_toggle_collider,
    SCRAP_OT_set_material,
    SCRAP_PT_panel,
)


HANDLERS = (
    (bpy.app.handlers.save_pre, _stamp),
    (bpy.app.handlers.save_post, _saved),
    (bpy.app.handlers.depsgraph_update_post, _changed),
)


def register():
    for cls in CLASSES:
        bpy.utils.register_class(cls)
    for handlers, handler in HANDLERS:
        if handler not in handlers:
            handlers.append(handler)


def unregister():
    for handlers, handler in HANDLERS:
        if handler in handlers:
            handlers.remove(handler)
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)
