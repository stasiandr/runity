// outline_shader_01: from Assets/Content/Art/Materials/Outline_Shader_01.shadergraph.
// Not an outline despite the name: a shell drawn over everything (ZTest
// Always), unlit and see-through, that shows only from afar — faded in by
// the camera's distance between Cutout Start (8 m) and Cutout End (12 m) —
// so a thing stays findable through walls when far away.
//
// The importer marks its materials `on_top: true` from the graph's ZTest.
// Left out: the shell's growth and its spin about an axis over time
// (M_Outline_01's _Scale 0.5 and _Speed_Rotation 20): a surface function
// cannot move vertices.
//
// scrap:params _Cutout_Start _Cutout_End

fn outline_fade(distance: f32, start: f32, end: f32) -> f32 {
    // A material that sets neither: the graph's 8 and 12.
    let a = select(start, 8.0, start == 0.0 && end == 0.0);
    let b = select(end, 12.0, start == 0.0 && end == 0.0);
    return smoothstep(a, b, distance);
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let distance = length(frame.camera_position.xyz - in.world_position);
    o.alpha = out.alpha * outline_fade(distance, in.params[0].x, in.params[0].y);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    return o;
}
