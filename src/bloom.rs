//! A real post-process bloom: the whole scene is rendered into a texture, the
//! bright parts (sun, moon, stars, specular glints, sky, the water's reflections)
//! are extracted, blurred wide with a separable Gaussian, and added back over the
//! scene. Every light in the frame blooms, not just discs drawn around the sun.
//!
//! Render-target orientation note: render-target textures are stored bottom-up, so
//! the scene ends up flipped in its texture. Every texture→texture pass (bright,
//! blur) draws with `flip_y: true`, which keeps the whole chain in one consistent
//! orientation, and the final composite to the screen also draws with `flip_y: true`
//! — flipping the scene and its bloom together so the world is upright and aligned.

use macroquad::prelude::*;

const VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
varying mediump vec2 uv;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    uv = texcoord;
}"#;

// Bright-pass: keep only what's brighter than a soft threshold, squared so the
// glow falls off gracefully toward the cutoff rather than banding.
const BRIGHT_FRAG: &str = r#"#version 100
precision mediump float;
varying mediump vec2 uv;
uniform sampler2D Texture;
uniform float Threshold;
uniform float Knee;
void main() {
    vec3 c = texture2D(Texture, uv).rgb;
    float l = max(c.r, max(c.g, c.b));
    float w = clamp((l - Threshold) / max(Knee, 0.0001), 0.0, 1.0);
    gl_FragColor = vec4(c * w * w, 1.0);
}"#;

// Separable 9-tap Gaussian; `Direction` is the per-tap texel step on one axis.
const BLUR_FRAG: &str = r#"#version 100
precision mediump float;
varying mediump vec2 uv;
uniform sampler2D Texture;
uniform vec2 Direction;
void main() {
    vec3 s = vec3(0.0);
    s += texture2D(Texture, uv + Direction * -4.0).rgb * 0.0162;
    s += texture2D(Texture, uv + Direction * -3.0).rgb * 0.0540;
    s += texture2D(Texture, uv + Direction * -2.0).rgb * 0.1216;
    s += texture2D(Texture, uv + Direction * -1.0).rgb * 0.1945;
    s += texture2D(Texture, uv).rgb                     * 0.2270;
    s += texture2D(Texture, uv + Direction *  1.0).rgb * 0.1945;
    s += texture2D(Texture, uv + Direction *  2.0).rgb * 0.1216;
    s += texture2D(Texture, uv + Direction *  3.0).rgb * 0.0540;
    s += texture2D(Texture, uv + Direction *  4.0).rgb * 0.0162;
    gl_FragColor = vec4(s, 1.0);
}"#;

// Composite: scene + blurred bloom, with a gentle filmic shoulder so the brightest
// blooms roll off toward white instead of hard-clipping into flat blobs.
const COMPOSITE_FRAG: &str = r#"#version 100
precision mediump float;
varying mediump vec2 uv;
uniform sampler2D Texture;
uniform sampler2D BloomTex;
uniform float Intensity;
void main() {
    vec3 scene = texture2D(Texture, uv).rgb;
    vec3 bloom = texture2D(BloomTex, uv).rgb;
    vec3 c = scene + bloom * Intensity;
    // Highlight-only shoulder: values up to 0.8 pass through untouched (the base
    // scene is unchanged), and only the over-bright sum is compressed toward white,
    // scaling all channels together so the bloom keeps its colour instead of clipping.
    float m = max(c.r, max(c.g, c.b));
    float over = max(m - 0.8, 0.0);
    c *= 1.0 / (1.0 + over);
    gl_FragColor = vec4(c, 1.0);
}"#;

struct Targets {
    w: u32,
    h: u32,
    /// Whether the scene target is multisampled (the MSAA setting). Tracked so the
    /// targets are rebuilt when the setting is toggled mid-voyage.
    msaa: bool,
    bw: u32,
    bh: u32,
    scene: RenderTarget,
    bright: RenderTarget,
    blur_a: RenderTarget,
    blur_b: RenderTarget,
}

pub struct Bloom {
    bright_mat: Material,
    blur_mat: Material,
    composite_mat: Material,
    targets: Option<Targets>,
    /// Brightness above which a pixel starts to bloom, and the soft ramp width.
    pub threshold: f32,
    pub knee: f32,
    /// How strongly the blurred bloom is added back.
    pub intensity: f32,
    /// Separable blur passes — more = wider, softer glow.
    pub iterations: usize,
}

fn make_target(w: u32, h: u32) -> RenderTarget {
    let rt = render_target(w.max(1), h.max(1));
    rt.texture.set_filter(FilterMode::Linear);
    rt
}

/// The scene target, optionally multisampled (the MSAA 4× setting). The bright/blur
/// chain stays single-sampled — they're texture-to-texture passes with nothing to
/// alias — so only the scene where the world is rasterized carries the samples.
fn make_scene_target(w: u32, h: u32, msaa: bool) -> RenderTarget {
    let rt = if msaa {
        render_target_msaa(w.max(1), h.max(1))
    } else {
        render_target(w.max(1), h.max(1))
    };
    rt.texture.set_filter(FilterMode::Linear);
    rt
}

/// Draw `src` across a `dw`×`dh` target through `mat`, flipping Y to keep the
/// texture→texture chain in the scene's orientation (see the module note).
fn blit(dest: &RenderTarget, dw: f32, dh: f32, src: &Texture2D, mat: &Material) {
    let mut cam = Camera2D::from_display_rect(Rect::new(0.0, 0.0, dw, dh));
    cam.render_target = Some(dest.clone());
    set_camera(&cam);
    clear_background(BLANK);
    gl_use_material(mat);
    draw_texture_ex(
        src,
        0.0,
        0.0,
        WHITE,
        DrawTextureParams {
            dest_size: Some(vec2(dw, dh)),
            flip_y: true,
            ..Default::default()
        },
    );
    gl_use_default_material();
}

impl Bloom {
    pub fn new() -> Bloom {
        let load = |frag: &str, uniforms: Vec<UniformDesc>, textures: Vec<String>| {
            load_material(
                ShaderSource::Glsl {
                    vertex: VERTEX,
                    fragment: frag,
                },
                MaterialParams {
                    uniforms,
                    textures,
                    ..Default::default()
                },
            )
            .expect("bloom shader failed to compile")
        };
        let bright_mat = load(
            BRIGHT_FRAG,
            vec![
                UniformDesc::new("Threshold", UniformType::Float1),
                UniformDesc::new("Knee", UniformType::Float1),
            ],
            vec![],
        );
        let blur_mat = load(
            BLUR_FRAG,
            vec![UniformDesc::new("Direction", UniformType::Float2)],
            vec![],
        );
        let composite_mat = load(
            COMPOSITE_FRAG,
            vec![UniformDesc::new("Intensity", UniformType::Float1)],
            vec!["BloomTex".to_string()],
        );
        Bloom {
            bright_mat,
            blur_mat,
            composite_mat,
            targets: None,
            threshold: 0.74,
            knee: 0.26,
            intensity: 1.2,
            iterations: 4,
        }
    }

    /// Ensure the render targets match the window (and the MSAA setting), then hand
    /// back the scene target for the caller to render the world into.
    pub fn scene_target(&mut self, w: f32, h: f32, msaa: bool) -> RenderTarget {
        let (fw, fh) = (w.max(1.0) as u32, h.max(1.0) as u32);
        let (bw, bh) = ((fw / 2).max(1), (fh / 2).max(1));
        let stale = match &self.targets {
            Some(t) => t.w != fw || t.h != fh || t.msaa != msaa,
            None => true,
        };
        if stale {
            self.targets = Some(Targets {
                w: fw,
                h: fh,
                msaa,
                bw,
                bh,
                scene: make_scene_target(fw, fh, msaa),
                bright: make_target(bw, bh),
                blur_a: make_target(bw, bh),
                blur_b: make_target(bw, bh),
            });
        }
        self.targets.as_ref().unwrap().scene.clone()
    }

    /// Draw the scene target straight to the screen with no bloom — the path used
    /// when bloom is off but the world was still rendered offscreen (so the MSAA
    /// setting is honoured). Flips Y like the bloom composite, since render-target
    /// textures are stored bottom-up.
    pub fn blit_scene_to_screen(&self, w: f32, h: f32) {
        let t = match &self.targets {
            Some(t) => t,
            None => return,
        };
        set_default_camera();
        draw_texture_ex(
            &t.scene.texture,
            0.0,
            0.0,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(w, h)),
                flip_y: true,
                ..Default::default()
            },
        );
    }

    /// Extract → blur → composite the scene target onto the screen. Call after the
    /// world has been drawn into the scene target and the default camera restored
    /// is *not* required (this restores it).
    pub fn render_to_screen(&self, w: f32, h: f32) {
        let t = match &self.targets {
            Some(t) => t,
            None => return,
        };
        let (bw, bh) = (t.bw as f32, t.bh as f32);

        // Bright-pass the scene into the half-res bright target.
        self.bright_mat.set_uniform("Threshold", self.threshold);
        self.bright_mat.set_uniform("Knee", self.knee);
        blit(&t.bright, bw, bh, &t.scene.texture, &self.bright_mat);

        // Ping-pong separable blur: H into blur_a, V into blur_b, repeat to widen.
        let mut src = &t.bright;
        for _ in 0..self.iterations.max(1) {
            self.blur_mat.set_uniform("Direction", vec2(1.0 / bw, 0.0));
            blit(&t.blur_a, bw, bh, &src.texture, &self.blur_mat);
            self.blur_mat.set_uniform("Direction", vec2(0.0, 1.0 / bh));
            blit(&t.blur_b, bw, bh, &t.blur_a.texture, &self.blur_mat);
            src = &t.blur_b;
        }

        // Composite scene + bloom straight to the screen.
        set_default_camera();
        self.composite_mat
            .set_texture("BloomTex", t.blur_b.texture.weak_clone());
        self.composite_mat.set_uniform("Intensity", self.intensity);
        gl_use_material(&self.composite_mat);
        draw_texture_ex(
            &t.scene.texture,
            0.0,
            0.0,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(w, h)),
                // Render-target textures are stored bottom-up, so flip when drawing to
                // the screen. The bloom (`BloomTex`) shares the scene's orientation and
                // is sampled at the same uv, so this flips both together — still aligned.
                flip_y: true,
                ..Default::default()
            },
        );
        gl_use_default_material();
    }
}
