#version 300 es

precision highp float;

in vec4 v_color;
in vec2 v_texcoord;

uniform vec2 size;
uniform vec2 position;
uniform float radius_top_left;
uniform float radius_top_right;
uniform float radius_bottom_left;
uniform float radius_bottom_right;
uniform float fade_inset;

uniform vec2 clip_size;
uniform vec2 clip_position;
uniform float clip_radius_top_left;
uniform float clip_radius_top_right;
uniform float clip_radius_bottom_left;
uniform float clip_radius_bottom_right;

out vec4 fragColor;

float corner_alpha(vec2 size, vec2 position, float round_tl, float round_tr, float round_bl, float round_br);
float corner_dist(vec2 size, vec2 position, float round_tl, float round_tr, float round_bl, float round_br);

void main() {
    float quad_corner_alpha = corner_alpha(
        size - 1.0,
        position + 0.5,
        radius_top_left,
        radius_top_right,
        radius_bottom_left,
        radius_bottom_right
    );

    // Clipping
    float clip_corner_alpha = corner_alpha(
        clip_size - 1.0,
        clip_position + 0.5,
        clip_radius_top_left,
        clip_radius_top_right,
        clip_radius_bottom_left,
        clip_radius_bottom_right
    );

    vec4 final_color = v_color;
    if (fade_inset > 0.0) {
        float d = corner_dist(
            size - 1.0,
            position + 0.5,
            radius_top_left,
            radius_top_right,
            radius_bottom_left,
            radius_bottom_right
        );
        float inside_dist = -d;
        float fade_factor = clamp(inside_dist / fade_inset, 0.0, 1.0);
        final_color *= fade_factor;
    }

    fragColor = mix(final_color, vec4(0.0), quad_corner_alpha) * clip_corner_alpha;
}
