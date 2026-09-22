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

/* --- the view ------------------------------------------------------- */

bool runity_editor_set_camera(RunityEditor *editor,
                              const float *eye_xyz,
                              const float *target_xyz);

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

/* Which arm is under a point: 0 X, 1 Y, 2 Z, -1 none. `hover` only looks;
 * `begin` grabs. */
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
