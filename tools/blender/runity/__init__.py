"""runity for Blender (docs/blender.md).

Keep a .blend in a runity project's assets/ and work in it: the engine
imports it on every save — models, materials, the object tree as a prefab,
what is marked as an asset as prefabs of their own. This add-on gives every
object, mesh, material and collection an ID on save, kept through renames,
so what the engine's scenes say about a part keeps landing on it.

The exporter the engine runs is export.py beside this file.
"""

import bpy
from bpy.app.handlers import persistent

from . import export


@persistent
def _stamp(*_args):
    export.stamp()


def register():
    if _stamp not in bpy.app.handlers.save_pre:
        bpy.app.handlers.save_pre.append(_stamp)


def unregister():
    if _stamp in bpy.app.handlers.save_pre:
        bpy.app.handlers.save_pre.remove(_stamp)
