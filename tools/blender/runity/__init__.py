"""runity for Blender (docs/blender.md).

Keep a .blend in a runity project's assets/ and work in it: the engine
imports it on every save — models, materials, the object tree as a prefab,
what is marked as an asset as prefabs of their own.

This add-on:

* gives every object, mesh, material and collection an ID on save, kept
  through renames, so what the engine's scenes say about a part keeps
  landing on it;
* sends the saved file to the runity editor, when one has the project
  open, so it is imported at once without starting another Blender;
* sends where objects are while they are being moved, so the editor shows
  the drag as it happens.

The exporter the engine runs is export.py beside this file.
"""

import bpy
from bpy.app.handlers import persistent

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


HANDLERS = (
    (bpy.app.handlers.save_pre, _stamp),
    (bpy.app.handlers.save_post, _saved),
    (bpy.app.handlers.depsgraph_update_post, _changed),
)


def register():
    for handlers, handler in HANDLERS:
        if handler not in handlers:
            handlers.append(handler)


def unregister():
    for handlers, handler in HANDLERS:
        if handler in handlers:
            handlers.remove(handler)
