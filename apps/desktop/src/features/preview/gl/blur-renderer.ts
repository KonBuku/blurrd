/**
 * The WebGL2 preview blur (plan C7, F25): the decoded video frame is
 * uploaded as a texture, every box of the frame is blurred with the same
 * steps as `CpuBlur` (block means with noise, bilinear upsample, separable
 * gaussian with a replicated border, hard blocks or a solid fill, mixed in
 * through the rounded-rectangle mask, boxes composited in order) and the
 * result is drawn to the canvas as Before, After or Split.
 *
 * The browser hands out RGB frames while the export blurs YUV 4:2:0, so
 * luma is blurred at full resolution and chroma on its own half-resolution
 * grid, mixed with the max-pooled chroma mask. The remaining difference
 * (chroma upsampling, RGB rounding) is measured against frames blurred by
 * `CpuBlur` (docs/development.md, "Preview blur parity").
 */
import type { RectI } from "@/bindings/RectI";
import {
  prepareBoxes,
  type ColourModel,
  type Fill,
  type PixelParams,
  type PreparedBox,
} from "@/lib/blur-geometry";

import { BoxGeometryCache, type GridSize, type PackedBox } from "./box-geometry";
import { GlKit, WebGlUnavailableError, type Program, type Target } from "./gl-kit";
import {
  CHROMA_ROWS,
  COMPOSITE,
  GEOMETRY_ROWS,
  MEANS,
  PATCH,
  PRESENT,
  RESAMPLE,
  SMOOTH_H,
  SMOOTH_V,
} from "./shaders";

/** How the canvas shows the frame. */
export type PreviewMode = "before" | "after" | "split";

/** A frame source: the video element or, in tests, decoded pixels. */
export type FrameSource = HTMLVideoElement | ImageBitmap | ImageData | HTMLCanvasElement;

/** One decoded backend patch (Exact) and where it goes, in source pixels. */
export interface Patch {
  readonly image: ImageBitmap | ImageData;
  readonly rect: RectI;
}

/** Size of the blank frame `warmUp` draws. */
const WARM_UP_SIZE = [256, 128] as const;

const FILL_CODE: Readonly<Record<Fill["kind"], number>> = {
  smooth: 0,
  blocks: 1,
  boxMean: 2,
  colour: 3,
};

const MODE_CODE: Readonly<Record<PreviewMode, number>> = { before: 0, after: 1, split: 2 };

/** The source and output frames at source size and their MRT framebuffer. */
interface Frames {
  readonly source: Target;
  readonly output: Target;
  readonly both: WebGLFramebuffer;
}

type ProgramName = "resample" | "means" | "smoothH" | "smoothV" | "composite" | "patch" | "present";

/** Scratch targets: block means, horizontal passes, chroma result, output under the box. */
type ScratchName = "means" | "chromaMeans" | "rows" | "chromaRows" | "chroma" | "previous";

/**
 * Renders the preview of one video into a canvas whose size is the
 * source size (zoom and pan are CSS transforms of the canvas).
 */
export class BlurRenderer {
  private readonly gl: WebGL2RenderingContext;
  private readonly kit: GlKit;
  private readonly programs: Readonly<Record<ProgramName, Program>>;
  private readonly cache = new BoxGeometryCache();
  private readonly video: WebGLTexture;
  private readonly patchTexture: WebGLTexture;
  private readonly geometry: WebGLTexture;
  private geometrySize: readonly [number, number] = [0, 0];
  private frames: Frames | null = null;
  private readonly scratch: Readonly<Record<ScratchName, Target>>;
  private width = 0;
  private height = 0;

  constructor(readonly canvas: HTMLCanvasElement | OffscreenCanvas) {
    const gl = canvas.getContext("webgl2", {
      alpha: false,
      antialias: false,
      depth: false,
      premultipliedAlpha: false,
      // The end-to-end tests read the canvas back (`drawImage`), which needs
      // the drawing buffer to survive compositing; the app does not.
      preserveDrawingBuffer: import.meta.env.MODE === "mock",
      powerPreference: "high-performance",
    }) as WebGL2RenderingContext | null;
    if (!gl) throw new WebGlUnavailableError("This system does not provide WebGL 2.");
    this.gl = gl;
    const kit = new GlKit(gl);
    this.kit = kit;
    // Float targets keep the means and the gaussian passes exact; without
    // the extension 8-bit targets are used (slightly less precise).
    const float = gl.getExtension("EXT_color_buffer_float") ? gl.RGBA32F : gl.RGBA8;
    this.programs = {
      resample: kit.program(RESAMPLE),
      means: kit.program(MEANS),
      smoothH: kit.program(SMOOTH_H),
      smoothV: kit.program(SMOOTH_V),
      composite: kit.program(COMPOSITE),
      patch: kit.program(PATCH),
      present: kit.program(PRESENT),
    };
    this.video = kit.texture(gl.LINEAR);
    this.patchTexture = kit.texture(gl.LINEAR);
    this.geometry = kit.texture(gl.NEAREST);
    this.scratch = {
      means: kit.target(float, 64, 16),
      chromaMeans: kit.target(float, 64, 16),
      rows: kit.target(float, 256, 64),
      chromaRows: kit.target(float, 128, 32),
      chroma: kit.target(float, 128, 32),
      previous: kit.target(gl.RGBA8, 256, 64),
    };
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  }

  /** Source size the canvas currently renders at. */
  get size(): readonly [number, number] {
    return [this.width, this.height];
  }

  /**
   * Uploads a decoded frame, scaled to `width` × `height` source pixels
   * (a proxy stream is smaller than the source).
   */
  upload(frame: FrameSource, width: number, height: number): void {
    const { gl, kit } = this;
    const frames = this.resize(width, height);
    gl.bindTexture(gl.TEXTURE_2D, this.video);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, frame);
    const program = this.programs.resample;
    gl.bindFramebuffer(gl.FRAMEBUFFER, frames.both);
    gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1]);
    kit.pass(frames.both, program, [0, 0, width, height], () => {
      kit.bind(program, "u_video", 0, this.video);
      gl.uniform2f(program.uniform("u_size"), width, height);
    });
  }

  /**
   * Blurs `boxes` into the output frame, starting again from the uploaded
   * source, then draws the backend `patches` (Exact) over them.
   */
  blur(
    boxes: readonly PreparedBox[],
    params: PixelParams,
    colour: ColourModel,
    patches: readonly Patch[] = [],
  ): void {
    const frames = this.frames;
    if (!frames) return;
    const whole = [0, 0, this.width, this.height] as const;
    this.kit.copy(frames.source.framebuffer, frames.output.framebuffer, whole);
    for (const b of boxes) this.blurBox(b, params, colour, frames);
    for (const patch of patches) this.drawPatch(patch, frames);
  }

  /** Draws the frame to the canvas in `mode`; `split` is in source pixels. */
  present(mode: PreviewMode, split: number): void {
    const { gl, kit } = this;
    const frames = this.frames;
    if (!frames) return;
    const program = this.programs.present;
    kit.pass(null, program, [0, 0, this.width, this.height], () => {
      kit.bind(program, "u_source", 0, frames.source.texture);
      kit.bind(program, "u_output", 1, frames.output.texture);
      gl.uniform1i(program.uniform("u_mode"), MODE_CODE[mode]);
      gl.uniform1f(program.uniform("u_split"), split);
      gl.uniform1i(program.uniform("u_height"), this.height);
    });
  }

  /**
   * Runs every program once on a small blank frame with `params` and
   * clears the canvas to black again (its look before the first frame).
   * A WebGL implementation may build a program's GPU executable at its
   * first draw (SwiftShader does: the first 1080p frame took 119-125 ms
   * without this and 48-50 ms with it); this moves that work to mount.
   */
  warmUp(params: PixelParams, colour: ColourModel): void {
    const [width, height] = WARM_UP_SIZE;
    const blank = new ImageData(width, height);
    const boxes = prepareBoxes(
      [{ rect: [32, 32, 224, 96], seed: 1, gridOrigin: [32, 32], blockScale: 1 }],
      width,
      height,
      params,
    );
    this.upload(blank, width, height);
    this.blur(boxes, params, colour, [{ image: blank, rect: [0, 0, 16, 16] }]);
    this.present("split", width / 2);
    const { gl } = this;
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
  }

  /** Reads the output frame (top row first, RGBA), for tests and checks. */
  readOutput(): Uint8Array {
    const { gl } = this;
    const pixels = new Uint8Array(this.width * this.height * 4);
    if (!this.frames) return pixels;
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.frames.output.framebuffer);
    gl.readPixels(0, 0, this.width, this.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    return pixels;
  }

  /** Frees the GPU resources (the canvas can get a new renderer later). */
  dispose(): void {
    this.kit.dispose();
    this.frames = null;
  }

  private blurBox(b: PreparedBox, params: PixelParams, colour: ColourModel, frames: Frames): void {
    const { gl, kit, scratch } = this;
    const fill = params.fill;
    const packed = this.cache.get(b, fill);
    this.uploadGeometry(packed);
    const [x0, y0, x1, y1] = b.padded;
    const box = [x0, y0, x1 - x0, y1 - y0] as const;
    if (fill.kind !== "colour") {
      const noise = params.noise ? colour.noise : 0;
      this.means(scratch.means, packed.luma, 0, noise, b, frames, colour);
    }
    if (fill.kind === "smooth" || fill.kind === "blocks") {
      this.means(scratch.chromaMeans, packed.chroma, CHROMA_ROWS, null, b, frames, colour);
    }
    if (fill.kind === "smooth") {
      this.smooth(scratch.means, scratch.rows, packed.luma, 0);
      this.smooth(scratch.chromaMeans, scratch.chromaRows, packed.chroma, CHROMA_ROWS);
      const { width, height, radius } = packed.chroma;
      kit.ensure(scratch.chroma, width, height);
      const program = this.programs.smoothV;
      kit.pass(scratch.chroma.framebuffer, program, [0, 0, width, height], () => {
        kit.bind(program, "u_rows", 0, scratch.chromaRows.texture);
        kit.bind(program, "u_geometry", 1, this.geometry);
        gl.uniform1i(program.uniform("u_radius"), radius);
        gl.uniform2i(program.uniform("u_box"), width, height);
      });
    }
    // The output under the box so far: earlier boxes stay mixed in.
    kit.ensure(scratch.previous, box[2], box[3]);
    kit.copy(frames.output.framebuffer, scratch.previous.framebuffer, box, 0, 0);
    const program = this.programs.composite;
    const whole = [0, 0, this.width, this.height] as const;
    const setup = () => {
      kit.bind(program, "u_rows", 0, scratch.rows.texture);
      kit.bind(program, "u_means", 1, scratch.means.texture);
      kit.bind(program, "u_chroma", 2, scratch.chroma.texture);
      kit.bind(program, "u_chroma_means", 3, scratch.chromaMeans.texture);
      kit.bind(program, "u_prev", 4, scratch.previous.texture);
      kit.bind(program, "u_geometry", 5, this.geometry);
      gl.uniform2f(program.uniform("u_luma"), ...colour.luma);
      gl.uniform1i(program.uniform("u_radius"), packed.luma.radius);
      gl.uniform2i(program.uniform("u_box"), packed.luma.width, packed.luma.height);
      gl.uniform1i(program.uniform("u_fill"), FILL_CODE[fill.kind]);
      const rgb = fill.kind === "colour" ? fill.rgb : [0, 0, 0];
      gl.uniform3f(program.uniform("u_colour"), rgb[0] / 255, rgb[1] / 255, rgb[2] / 255);
      gl.uniform4i(program.uniform("u_padded"), x0, y0, x1, y1);
      gl.uniform4i(program.uniform("u_core"), ...b.core);
      gl.uniform4f(program.uniform("u_shape"), ...b.shape);
      gl.uniform1f(program.uniform("u_corner"), b.radius);
      gl.uniform1f(program.uniform("u_feather"), b.feather);
    };
    kit.pass(frames.output.framebuffer, program, whole, setup, box);
  }

  /**
   * Block means of one resolution's grid (geometry rows from `set`) into
   * `target`: luma RGB with `noise` added, or chroma when `noise` is `null`.
   */
  private means(
    target: Target,
    grid: GridSize,
    set: number,
    noise: number | null,
    b: PreparedBox,
    frames: Frames,
    colour: ColourModel,
  ): void {
    const { gl, kit } = this;
    kit.ensure(target, grid.cols, grid.rows);
    const program = this.programs.means;
    kit.pass(target.framebuffer, program, [0, 0, grid.cols, grid.rows], () => {
      kit.bind(program, "u_source", 0, frames.source.texture);
      kit.bind(program, "u_geometry", 1, this.geometry);
      gl.uniform2i(program.uniform("u_origin"), b.padded[0], b.padded[1]);
      gl.uniform1i(program.uniform("u_bounds"), set + GEOMETRY_ROWS.colBounds);
      gl.uniform1i(program.uniform("u_chroma"), noise === null ? 1 : 0);
      gl.uniform2f(program.uniform("u_luma"), ...colour.luma);
      gl.uniform1f(program.uniform("u_noise"), noise ?? 0);
    });
  }

  /** Upsample and horizontal gaussian of one resolution's means. */
  private smooth(means: Target, rows: Target, grid: GridSize, set: number): void {
    const { gl, kit } = this;
    kit.ensure(rows, grid.width, grid.height);
    const program = this.programs.smoothH;
    kit.pass(rows.framebuffer, program, [0, 0, grid.width, grid.height], () => {
      kit.bind(program, "u_means", 0, means.texture);
      kit.bind(program, "u_geometry", 1, this.geometry);
      gl.uniform1i(program.uniform("u_set"), set);
      gl.uniform1i(program.uniform("u_radius"), grid.radius);
      gl.uniform2i(program.uniform("u_box"), grid.width, grid.height);
    });
  }

  private drawPatch(patch: Patch, frames: Frames): void {
    const { gl, kit } = this;
    const [x0, y0, x1, y1] = patch.rect;
    gl.bindTexture(gl.TEXTURE_2D, this.patchTexture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, patch.image);
    const program = this.programs.patch;
    const box = [x0, y0, x1 - x0, y1 - y0] as const;
    const whole = [0, 0, this.width, this.height] as const;
    const setup = () => {
      kit.bind(program, "u_patch", 0, this.patchTexture);
      gl.uniform4f(program.uniform("u_rect"), ...box);
    };
    kit.pass(frames.output.framebuffer, program, whole, setup, box);
  }

  private uploadGeometry(packed: PackedBox): void {
    const { gl } = this;
    gl.bindTexture(gl.TEXTURE_2D, this.geometry);
    const [w, h] = this.geometrySize;
    if (packed.texWidth > w || packed.texHeight > h) {
      this.geometrySize = [Math.max(packed.texWidth, w), Math.max(packed.texHeight, h)];
      const [width, height] = this.geometrySize;
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32F, width, height, 0, gl.RGBA, gl.FLOAT, null);
    }
    const { texWidth, texHeight, data } = packed;
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, texWidth, texHeight, gl.RGBA, gl.FLOAT, data);
  }

  /** The frame targets at `width` × `height`, recreated when the size changes. */
  private resize(width: number, height: number): Frames {
    if (this.frames && width === this.width && height === this.height) return this.frames;
    const { gl, kit } = this;
    this.width = width;
    this.height = height;
    this.canvas.width = width;
    this.canvas.height = height;
    if (this.frames) {
      kit.free(this.frames.source);
      kit.free(this.frames.output);
      kit.freeFramebuffer(this.frames.both);
    }
    const source = kit.target(gl.RGBA8, width, height);
    const output = kit.target(gl.RGBA8, width, height);
    const both = kit.framebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, both);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, source.texture, 0);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT1, gl.TEXTURE_2D, output.texture, 0);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    this.frames = { source, output, both };
    return this.frames;
  }
}
