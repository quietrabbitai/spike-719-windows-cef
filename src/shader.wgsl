// Fullscreen triangle that samples one CEF texture. Used for the view and, in a smaller viewport, for popup widgets.
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: VOut;
    let xy = p[i];
    o.pos = vec4<f32>(xy, 0.0, 1.0);
    o.uv = vec2<f32>((xy.x + 1.0) * 0.5, 1.0 - (xy.y + 1.0) * 0.5);
    return o;
}

@group(0) @binding(0) var tex0: texture_2d<f32>;
@group(0) @binding(1) var samp0: sampler;

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    return textureSample(tex0, samp0, in.uv);
}
