Fixtures for the engine's own tests.

They are deliberately not game assets. The engine has to be testable without
a game, and a render test needs a shape with a silhouette, a lit side and a
shaded one — nothing more. `conifer.obj` is that shape: an octagonal trunk
and two stacked skirts, wound counter-clockwise seen from outside so that
back-face culling keeps it visible.
