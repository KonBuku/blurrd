/**
 * GLSL ES 3.00 sources of the preview blur (plan C7). Every pass draws one
 * triangle that covers its viewport and works on whole pixels with
 * `texelFetch`. Frame textures keep the image's top row in texture row 0;
 * only the present pass flips rows for the canvas.
 *
 * Like the 4:2:0 export, a box is blurred twice: luma at full resolution
 * and chroma (B−Y, R−Y) on its own half-resolution block grid. Per box a
 * geometry texture (RGBA32F, see `box-geometry.ts`) carries what the CPU
 * backend precomputes for both: bilinear taps `(c0, c1, t, cell)` per
 * column and per row, the column and row bounds of the block grid, the
 * gaussian kernel and the luma noise of every block.
 */

/** Rows of the geometry texture; the chroma set starts at `CHROMA_ROWS`. */
export const GEOMETRY_ROWS = {
  colTaps: 0,
  rowTaps: 1,
  colBounds: 2,
  rowBounds: 3,
  kernel: 4,
} as const;
/** First row of the chroma set (same order as the luma rows). */
export const CHROMA_ROWS = 5;
/** First row of the per-block luma noise. */
export const NOISE_ROW = 10;

const HEADER = `#version 300 es
precision highp float;
precision highp int;
precision highp sampler2D;`;

/** Covers the viewport with one triangle. */
export const VERTEX = `#version 300 es
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

/**
 * Resamples the decoded video (any size) to the source size and writes it
 * to the source and the output texture at once.
 */
export const RESAMPLE = `${HEADER}
uniform sampler2D u_video;
uniform vec2 u_size;
layout(location = 0) out vec4 o_source;
layout(location = 1) out vec4 o_output;
void main() {
  vec4 c = vec4(texture(u_video, gl_FragCoord.xy / u_size).rgb, 1.0);
  o_source = c;
  o_output = c;
}`;

/**
 * Area-average of every block: one output texel per cell. Cells cut by the
 * padded box average only their pixels inside it. Bounds are in luma
 * pixels of the padded box (chroma cells cover two per sample). Luma means
 * keep RGB and get the noise, a luma offset that adds the same amount to
 * R, G and B; chroma means hold (B−Y, R−Y).
 */
export const MEANS = `${HEADER}
uniform sampler2D u_source;
uniform sampler2D u_geometry;
uniform ivec2 u_origin;
uniform int u_bounds;
uniform bool u_chroma;
uniform vec2 u_luma;
uniform float u_noise;
out vec4 o_mean;
void main() {
  ivec2 cell = ivec2(gl_FragCoord.xy);
  ivec2 xs = ivec2(texelFetch(u_geometry, ivec2(cell.x, u_bounds), 0).xy);
  ivec2 ys = ivec2(texelFetch(u_geometry, ivec2(cell.y, u_bounds + 1), 0).xy);
  vec3 sum = vec3(0.0);
  for (int y = ys.x; y < ys.y; y++) {
    vec3 row = vec3(0.0);
    for (int x = xs.x; x < xs.y; x++) {
      row += texelFetch(u_source, u_origin + ivec2(x, y), 0).rgb;
    }
    sum += row;
  }
  vec3 mean = sum / float((xs.y - xs.x) * (ys.y - ys.x));
  if (u_chroma) {
    float luma = dot(mean, vec3(u_luma.x, 1.0 - u_luma.x - u_luma.y, u_luma.y));
    o_mean = vec4(mean.b - luma, mean.r - luma, 0.0, 1.0);
  } else {
    float n = texelFetch(u_geometry, ivec2(cell.x, ${NOISE_ROW} + cell.y), 0).x;
    o_mean = vec4(mean + n * u_noise, 1.0);
  }
}`;

/**
 * Bilinear upsample of the block means plus the horizontal gaussian with a
 * replicated border, at the resolution of the geometry set `u_set` (row of
 * its column taps), written at padded-box coordinates.
 */
export const SMOOTH_H = `${HEADER}
uniform sampler2D u_means;
uniform sampler2D u_geometry;
uniform int u_set;
uniform int u_radius;
uniform ivec2 u_box;
out vec4 o_row;
vec3 mean(int c, int r) { return texelFetch(u_means, ivec2(c, r), 0).rgb; }
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  vec4 rt = texelFetch(u_geometry, ivec2(p.y, u_set + 1), 0);
  int r0 = int(rt.x);
  int r1 = int(rt.y);
  vec3 sum = vec3(0.0);
  for (int t = 0; t <= 2 * u_radius; t++) {
    int x = clamp(p.x + t - u_radius, 0, u_box.x - 1);
    vec4 ct = texelFetch(u_geometry, ivec2(x, u_set), 0);
    int c0 = int(ct.x);
    int c1 = int(ct.y);
    vec3 top = mix(mean(c0, r0), mean(c1, r0), ct.z);
    vec3 bottom = mix(mean(c0, r1), mean(c1, r1), ct.z);
    float k = texelFetch(u_geometry, ivec2(t, u_set + 4), 0).x;
    sum += k * mix(top, bottom, rt.z);
  }
  o_row = vec4(sum, 1.0);
}`;

/** The vertical gaussian of the chroma set over its horizontal pass. */
export const SMOOTH_V = `${HEADER}
uniform sampler2D u_rows;
uniform sampler2D u_geometry;
uniform int u_radius;
uniform ivec2 u_box;
out vec4 o_value;
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  vec3 sum = vec3(0.0);
  for (int t = 0; t <= 2 * u_radius; t++) {
    int y = clamp(p.y + t - u_radius, 0, u_box.y - 1);
    float k = texelFetch(u_geometry, ivec2(t, ${CHROMA_ROWS + GEOMETRY_ROWS.kernel}), 0).x;
    sum += k * texelFetch(u_rows, ivec2(p.x, y), 0).rgb;
  }
  o_value = vec4(sum, 1.0);
}`;

/**
 * The fill of a box mixed into the output through its soft mask. Fill 0 =
 * smooth (luma: vertical gaussian over the horizontal pass; chroma: the
 * half-resolution result), 1 = hard blocks, 2 = box mean, 3 = fixed
 * colour. The mask is the CPU backend's rounded-rectangle SDF with the
 * unpadded box always at 1. Luma mixes with the pixel's mask and chroma
 * with the largest mask of the pixel's 2×2 chroma block, like the 4:2:0
 * export. `u_prev` holds the output under the box before this box (boxes
 * composite in order), at padded-box coordinates.
 */
export const COMPOSITE = `${HEADER}
uniform sampler2D u_rows;
uniform sampler2D u_means;
uniform sampler2D u_chroma;
uniform sampler2D u_chroma_means;
uniform sampler2D u_prev;
uniform sampler2D u_geometry;
uniform vec2 u_luma;
uniform int u_radius;
uniform ivec2 u_box;
uniform int u_fill;
uniform vec3 u_colour;
uniform ivec4 u_padded;
uniform ivec4 u_core;
uniform vec4 u_shape;
uniform float u_corner;
uniform float u_feather;
out vec4 o_colour;

float soft(float sdf) {
  if (u_feather <= 0.0) return sdf < 0.0 ? 1.0 : 0.0;
  float t = clamp(-sdf / u_feather, 0.0, 1.0);
  return t * t * (3.0 - 2.0 * t);
}

// x: mask along the axis, y: q, z: corner band, w: inside the core.
vec4 axisMask(int v, int p0, int p1, int c0, int c1, float s0, float s1) {
  if (v < p0 || v >= p1) return vec4(0.0, 1e30, 0.0, 0.0);
  float q = abs(float(v - p0) + 0.5 - 0.5 * (s0 + s1)) - ((s1 - s0) * 0.5 - u_corner);
  bool core = v >= c0 && v < c1;
  return vec4(core ? 1.0 : soft(q - u_corner), q, q > 0.0 ? 1.0 : 0.0, core ? 1.0 : 0.0);
}

float coverage(ivec2 p) {
  float px = float(u_padded.x);
  float py = float(u_padded.y);
  vec4 col = axisMask(p.x, u_padded.x, u_padded.z, u_core.x, u_core.z, u_shape.x - px, u_shape.z - px);
  vec4 row = axisMask(p.y, u_padded.y, u_padded.w, u_core.y, u_core.w, u_shape.y - py, u_shape.w - py);
  if (col.w > 0.5 && row.w > 0.5) return 1.0;
  if (col.z > 0.5 && row.z > 0.5) {
    float squared = col.y * col.y + row.y * row.y;
    float inner = u_corner - u_feather;
    if (squared >= u_corner * u_corner) return 0.0;
    if (inner >= 0.0 && squared <= inner * inner) return 1.0;
    return soft(sqrt(squared) - u_corner);
  }
  return min(col.x, row.x);
}

int cell(int v, int row) { return int(texelFetch(u_geometry, ivec2(v, row), 0).w); }

void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  ivec2 local = p - u_padded.xy;
  ivec2 half_local = local >> 1;
  vec3 kweights = vec3(u_luma.x, 1.0 - u_luma.x - u_luma.y, u_luma.y);
  vec3 value;
  vec2 chroma;
  if (u_fill == 0) {
    value = vec3(0.0);
    for (int t = 0; t <= 2 * u_radius; t++) {
      int y = clamp(local.y + t - u_radius, 0, u_box.y - 1);
      float k = texelFetch(u_geometry, ivec2(t, ${GEOMETRY_ROWS.kernel}), 0).x;
      value += k * texelFetch(u_rows, ivec2(local.x, y), 0).rgb;
    }
    chroma = texelFetch(u_chroma, half_local, 0).xy;
  } else if (u_fill == 1) {
    ivec2 c = ivec2(cell(local.x, ${GEOMETRY_ROWS.colTaps}), cell(local.y, ${GEOMETRY_ROWS.rowTaps}));
    value = texelFetch(u_means, c, 0).rgb;
    ivec2 cc = ivec2(
      cell(half_local.x, ${CHROMA_ROWS + GEOMETRY_ROWS.colTaps}),
      cell(half_local.y, ${CHROMA_ROWS + GEOMETRY_ROWS.rowTaps}));
    chroma = texelFetch(u_chroma_means, cc, 0).xy;
  } else {
    value = u_fill == 2 ? texelFetch(u_means, ivec2(0, 0), 0).rgb : u_colour;
    float y = dot(value, kweights);
    chroma = vec2(value.b - y, value.r - y);
  }
  ivec2 c = p & ~1;
  float chroma_mask = max(
    max(coverage(c), coverage(c + ivec2(1, 0))),
    max(coverage(c + ivec2(0, 1)), coverage(c + ivec2(1, 1))));
  vec3 previous = texelFetch(u_prev, local, 0).rgb;
  float y0 = dot(previous, kweights);
  vec2 c0 = vec2(previous.b - y0, previous.r - y0);
  float y = mix(y0, dot(value, kweights), coverage(p));
  vec2 cbcr = mix(c0, chroma, chroma_mask);
  float r = y + cbcr.y;
  float b = y + cbcr.x;
  float g = (y - u_luma.x * r - u_luma.y * b) / kweights.y;
  o_colour = vec4(r, g, b, 1.0);
}`;

/** Draws a backend patch (Exact) over its rectangle of the output. */
export const PATCH = `${HEADER}
uniform sampler2D u_patch;
uniform vec4 u_rect;
out vec4 o_colour;
void main() {
  o_colour = vec4(texture(u_patch, (gl_FragCoord.xy - u_rect.xy) / u_rect.zw).rgb, 1.0);
}`;

/**
 * Shows the frame on the canvas: 0 = source (Before), 1 = output (After),
 * 2 = split at `u_split` source pixels (Before on the left).
 */
export const PRESENT = `${HEADER}
uniform sampler2D u_source;
uniform sampler2D u_output;
uniform int u_mode;
uniform float u_split;
uniform int u_height;
out vec4 o_colour;
void main() {
  ivec2 p = ivec2(int(gl_FragCoord.x), u_height - 1 - int(gl_FragCoord.y));
  bool before = u_mode == 0 || (u_mode == 2 && gl_FragCoord.x < u_split);
  o_colour = before ? texelFetch(u_source, p, 0) : texelFetch(u_output, p, 0);
}`;
