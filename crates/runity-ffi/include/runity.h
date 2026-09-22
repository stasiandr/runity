/*
 * runity — the editor's side of the engine.
 *
 * Swift imports this header directly; there is no binding generator and no
 * generated code to keep in step. The boundary is ids, floats and paths on
 * purpose: nothing here mirrors an engine type, because the moment the
 * editor declares its own copy of a component, every new component becomes
 * work in two languages.
 *
 * Every call is safe on a NULL editor. An editor's UI outlives its document,
 * and a boundary that crashes when a panel repaints after a close is one
 * that gets wrapped in defensive code on the other side.
 *
 * Strings follow the usual C shape: pass a buffer and its capacity, get back
 * the length the value wanted. A return larger than the capacity means the
 * value was truncated — ask again with a bigger buffer. What was written is
 * always NUL-terminated.
 */

#ifndef RUNITY_H
#define RUNITY_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct RunityEditor RunityEditor;

/* --- lifetime ------------------------------------------------------- */

/* Open an editor that renders into its own image rather than a window.
 * Returns NULL on failure; runity_last_error says why. */
RunityEditor *runity_editor_create_offscreen(unsigned int width,
                                             unsigned int height);

/* Open an editor that draws into a layer the host already owns. On macOS
 * `layer` is a CAMetalLayer* — the layer of the view the editor put on
 * screen. The engine never makes a window; the host does. Returns NULL on
 * failure, and on platforms without this path.
 *
 * The layer must outlive the editor. Releasing the view while the editor
 * still holds a swapchain is a use after free nothing here can detect. */
RunityEditor *runity_editor_create_for_layer(void *layer,
                                             unsigned int width,
                                             unsigned int height);

/* Tell the editor its view changed size. */
bool runity_editor_resize(RunityEditor *editor,
                          unsigned int width,
                          unsigned int height);

/* Release an editor. NULL is a no-op. */
void runity_editor_free(RunityEditor *editor);

/* The last failure on this thread. Thread-local, so one document's error
 * never surfaces under another's. */
unsigned int runity_last_error(char *buffer, unsigned int capacity);

/* --- documents ------------------------------------------------------ */

bool runity_editor_set_library(RunityEditor *editor, const char *path);
bool runity_editor_open_scene(RunityEditor *editor, const char *path);

/* NULL path saves where the scene was opened from. */
bool runity_editor_save_scene(RunityEditor *editor, const char *path);

/* --- the scene tree, flattened -------------------------------------- */

/* Counts children too: an index here addresses the same entity as long as
 * the document is open. */
unsigned int runity_editor_entity_count(RunityEditor *editor);

unsigned int runity_editor_entity_name(RunityEditor *editor,
                                       unsigned int index,
                                       char *buffer,
                                       unsigned int capacity);

/* Nine floats: position xyz, rotation in Euler degrees xyz, scale xyz.
 * Degrees rather than a quaternion because this is what an inspector shows
 * and what a person types. */
bool runity_editor_get_transform(RunityEditor *editor,
                                 unsigned int index,
                                 float *out_nine);

bool runity_editor_set_transform(RunityEditor *editor,
                                 unsigned int index,
                                 const float *nine);

/* Where the entity actually is, in world space. Not the same as its
 * transform, which is local and belongs to the file: this is where it ends
 * up once its parents — and, while play is running, the simulation — have
 * had their say. */
bool runity_editor_world_position(RunityEditor *editor,
                                  unsigned int index,
                                  float *out_three);

/* --- prefabs -------------------------------------------------------- */

/* A prefab is one entity subtree in its own file, and a scene points at it
 * by name: editing the file changes every instance, everywhere. Opening a
 * scene loads the prefabs/ directory beside it — the convention every tool
 * follows — so this is only for a host that keeps them elsewhere. */
bool runity_editor_set_prefabs(RunityEditor *editor, const char *path);

/* What there is to place. Sorted, so the list does not reshuffle. */
unsigned int runity_editor_prefab_count(RunityEditor *editor);
unsigned int runity_editor_prefab_name(RunityEditor *editor,
                                       unsigned int index,
                                       char *buffer,
                                       unsigned int capacity);

/* What prefab this entity is an instance of, or "" for a plain entity. The
 * tree needs to say so: an instance is one row whose insides belong to a
 * file, and a row that looks like every other row hides that until someone
 * moves a stone and moves twelve. */
unsigned int runity_editor_entity_prefab(RunityEditor *editor,
                                         unsigned int index,
                                         char *buffer,
                                         unsigned int capacity);

/* Place an instance, under `parent` or at the top with -1. Returns its
 * index, or -1 — including when there is no prefab by that name, which is
 * refused rather than left as an entity with nothing in it. */
int runity_editor_add_instance(RunityEditor *editor,
                               int parent,
                               const char *prefab);

/* Save an entity's subtree as a prefab and make it an instance of it: the
 * move that turns a thing arranged once into a thing placed many times. One
 * call and not two, because doing it by hand leaves the scene holding a copy
 * that drifts from the file. */
bool runity_editor_make_prefab(RunityEditor *editor,
                               unsigned int index,
                               const char *name);

/* --- materials ------------------------------------------------------ */

/* Four floats: r, g, b in LINEAR space, then 1 or 0 for unlit.
 *
 * Resolved, not raw: an entity naming "stone" reports the colour stone
 * actually is, so a swatch shows what is on screen rather than the word.
 * runity_editor_material_name tells the two apart.
 *
 * Linear because that is what the engine holds and what survives a round
 * trip. A picker working in sRGB converts with the two helpers below
 * instead of carrying its own formula. */
bool runity_editor_get_material(RunityEditor *editor,
                                unsigned int index,
                                float *out_four);

/* Give one entity a colour of its own, breaking any link to a named
 * material — which is what dragging a slider on one object means. */
bool runity_editor_set_material(RunityEditor *editor,
                                unsigned int index,
                                const float *four);

/* The material this entity points at, or "" when it carries its own
 * colour. */
unsigned int runity_editor_material_name(RunityEditor *editor,
                                         unsigned int index,
                                         char *buffer,
                                         unsigned int capacity);

/* Point an entity at a material by name: a .rmat in the library, or a
 * builtin. An empty name is refused — clearing the link means giving the
 * entity a colour. An unknown name is accepted and draws grey, so a
 * material can be named before it is imported. */
bool runity_editor_set_material_name(RunityEditor *editor,
                                     unsigned int index,
                                     const char *name);

/* What the editor can offer: the library's materials, then the builtins it
 * does not shadow. */
unsigned int runity_editor_palette_count(RunityEditor *editor);
unsigned int runity_editor_palette_name(RunityEditor *editor,
                                        unsigned int index,
                                        char *buffer,
                                        unsigned int capacity);
bool runity_editor_palette_color(RunityEditor *editor,
                                 unsigned int index,
                                 float *out_four);

/* One channel, either way. Here so a host does not carry its own copy of
 * the curve: the home-made version is usually powf(2.2), close enough to
 * look right and wrong enough that a colour picked in the editor is not the
 * colour the engine draws. */
float runity_srgb_to_linear(float channel);
float runity_linear_to_srgb(float channel);

/* --- editing, and taking it back ------------------------------------ */

/* Add an entity with a model, under `parent` or at the top with -1.
 * Returns the new entity's index, or -1. */
int runity_editor_add(RunityEditor *editor, int parent, const char *model);

/* Delete an entity and everything under it. */
bool runity_editor_delete(RunityEditor *editor, unsigned int index);

/* Copy an entity beside itself; returns the copy's index or -1. */
int runity_editor_duplicate(RunityEditor *editor, unsigned int index);

/* Move an entity under another, or to the top with -1. Refuses to make
 * something its own ancestor. */
bool runity_editor_reparent(RunityEditor *editor,
                            unsigned int index,
                            int new_parent);

/* A gizmo drag is one step however many frames it lasts: the snapshot is
 * taken when the drag begins. A value typed into an inspector is its own
 * step. */
bool runity_editor_undo(RunityEditor *editor);
bool runity_editor_redo(RunityEditor *editor);
bool runity_editor_can_undo(RunityEditor *editor);
bool runity_editor_can_redo(RunityEditor *editor);

/* --- play mode ------------------------------------------------------ */

/* Simulate the scene instead of editing it. The document is kept aside and
 * put back when play stops, so a thing that fell over stays fallen only
 * while you are watching it.
 *
 * The engine owns no loop: step it with however long your frame took, in
 * seconds, and it decides how many fixed steps that is worth — zero on a
 * fast frame, several on a slow one. The count is what comes back.
 *
 * Editing while playing is refused with a message rather than allowed and
 * thrown away on stop, which is the version people lose an hour to. */
bool runity_editor_play(RunityEditor *editor);
unsigned int runity_editor_step(RunityEditor *editor, float seconds);
bool runity_editor_stop(RunityEditor *editor);
bool runity_editor_is_playing(RunityEditor *editor);

/* --- the view ------------------------------------------------------- */

bool runity_editor_set_camera(RunityEditor *editor,
                              const float *eye_xyz,
                              const float *target_xyz);

bool runity_editor_get_camera(RunityEditor *editor,
                              float *out_eye_xyz,
                              float *out_target_xyz);

/* Write where the editor is looking into the scene, as one undoable step.
 * Flying around is not an edit — a scene that changed every time someone
 * looked at it from another angle would produce a diff on every open — so
 * keeping a viewpoint is a decision, and a call. Opening a scene puts the
 * camera where the file says. */
bool runity_editor_capture_camera(RunityEditor *editor);

bool runity_editor_render(RunityEditor *editor);

unsigned int runity_editor_width(RunityEditor *editor);
unsigned int runity_editor_height(RunityEditor *editor);

/* The last frame as RGBA8. Pass NULL to ask how many bytes it needs. A host
 * with a real surface never calls this; it exists so an editor can show
 * something before it has a view. */
unsigned int runity_editor_frame_pixels(RunityEditor *editor,
                                        uint8_t *buffer,
                                        unsigned int capacity);

/* --- selection and the move gizmo ----------------------------------- */

/* Put the gizmo on an entity, or pass -1 to clear the selection. */
bool runity_editor_select(RunityEditor *editor, int index);
int runity_editor_selected(RunityEditor *editor);

/* What the gizmo does: 0 move, 1 rotate, 2 scale. Anything else is refused
 * rather than quietly treated as move. All three work in world axes; a
 * local-axis toggle is not here yet. */
bool runity_editor_set_tool(RunityEditor *editor, int tool);
int runity_editor_tool(RunityEditor *editor);

/* Which handle is under a point: 0 X, 1 Y, 2 Z, -1 none — an arm for move
 * and scale, a ring for rotate. `hover` only looks; `begin` grabs. */
int runity_editor_gizmo_hover(RunityEditor *editor,
                              unsigned int x,
                              unsigned int y);
int runity_editor_gizmo_begin(RunityEditor *editor,
                              unsigned int x,
                              unsigned int y);

/* Follow a point with the held arm. No-op without a grab. */
bool runity_editor_gizmo_drag(RunityEditor *editor,
                              unsigned int x,
                              unsigned int y);

void runity_editor_gizmo_end(RunityEditor *editor);

/* The entity under a point, or -1. Tested against bounding boxes: a
 * triangle-exact pick is better and much slower, and the difference only
 * shows on thin diagonal geometry. */
int runity_editor_pick(RunityEditor *editor,
                       unsigned int x,
                       unsigned int y);

#ifdef __cplusplus
}
#endif

#endif /* RUNITY_H */
