Fixtures for the engine's own tests.

They are deliberately not game assets. The engine has to be testable without
a game, and a render test needs a shape with a silhouette, a lit side and a
shaded one — nothing more. `conifer.obj` is that shape: an octagonal trunk
and two stacked skirts, wound counter-clockwise seen from outside so that
back-face culling keeps it visible.

`floating_quad.gltf` is a hand-written glTF: one node, translated three
metres up, holding one quad at the origin. An importer that reads meshes
without walking the scene graph puts the quad back at zero, and that is the
mistake the fixture exists to catch — a model exported as placed parts
collapses into a heap in exactly this way.
