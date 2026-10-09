/**
 * Small WebGL2 helpers of the preview renderer: programs with cached
 * uniform locations, textures, render targets that grow on demand and
 * one-triangle passes.
 */
import { VERTEX } from "./shaders";

/** Why the preview cannot use WebGL. */
export class WebGlUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "WebGlUnavailableError";
  }
}

/** A texture with its framebuffer. */
export interface Target {
  format: number;
  texture: WebGLTexture;
  framebuffer: WebGLFramebuffer;
  width: number;
  height: number;
}

/** A linked program with cached uniform locations. */
export interface Program {
  readonly program: WebGLProgram;
  readonly uniform: (name: string) => WebGLUniformLocation | null;
}

/** `[x, y, width, height]` in pixels of the bound framebuffer. */
export type Box = readonly [number, number, number, number];

/** The helpers, bound to one context. */
export class GlKit {
  /** Everything created through the kit, deleted by {@link GlKit.dispose}. */
  private readonly owned = {
    programs: new Set<WebGLProgram>(),
    textures: new Set<WebGLTexture>(),
    framebuffers: new Set<WebGLFramebuffer>(),
  };

  constructor(readonly gl: WebGL2RenderingContext) {}

  /** A framebuffer owned by the kit. */
  framebuffer(): WebGLFramebuffer {
    const framebuffer = this.gl.createFramebuffer();
    this.owned.framebuffers.add(framebuffer);
    return framebuffer;
  }

  /**
   * Deletes every program, texture and framebuffer of the kit. The context
   * stays usable, so a remounted component can render into the same canvas.
   */
  dispose(): void {
    const { gl, owned } = this;
    for (const program of owned.programs) gl.deleteProgram(program);
    for (const texture of owned.textures) gl.deleteTexture(texture);
    for (const framebuffer of owned.framebuffers) gl.deleteFramebuffer(framebuffer);
    owned.programs.clear();
    owned.textures.clear();
    owned.framebuffers.clear();
  }

  /** Compiles `fragment` with the shared vertex shader. */
  program(fragment: string): Program {
    const { gl } = this;
    const program = gl.createProgram();
    this.owned.programs.add(program);
    for (const [type, source] of [
      [gl.VERTEX_SHADER, VERTEX],
      [gl.FRAGMENT_SHADER, fragment],
    ] as const) {
      const shader = gl.createShader(type);
      if (!shader) throw new WebGlUnavailableError("A shader could not be created.");
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        const log = gl.getShaderInfoLog(shader) ?? "";
        throw new WebGlUnavailableError(`A shader did not compile: ${log}`);
      }
      gl.attachShader(program, shader);
      gl.deleteShader(shader);
    }
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      const log = gl.getProgramInfoLog(program) ?? "";
      throw new WebGlUnavailableError(`A shader program did not link: ${log}`);
    }
    const locations = new Map<string, WebGLUniformLocation | null>();
    return {
      program,
      uniform: (name) => {
        if (!locations.has(name)) locations.set(name, gl.getUniformLocation(program, name));
        return locations.get(name) ?? null;
      },
    };
  }

  /** A texture with clamped edges and the given filter. */
  texture(filter: number): WebGLTexture {
    const { gl } = this;
    const texture = gl.createTexture();
    this.owned.textures.add(texture);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return texture;
  }

  /** A render target of `format` (sized internal format). */
  target(format: number, width: number, height: number): Target {
    const { gl } = this;
    const texture = this.texture(gl.NEAREST);
    gl.texStorage2D(gl.TEXTURE_2D, 1, format, width, height);
    const framebuffer = this.framebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    return { format, texture, framebuffer, width, height };
  }

  /** Grows `target` (same format) so that it holds `width` × `height`. */
  ensure(target: Target, width: number, height: number): void {
    if (width <= target.width && height <= target.height) return;
    const grown = this.target(
      target.format,
      Math.max(width, target.width),
      Math.max(height, target.height),
    );
    this.free(target);
    Object.assign(target, grown);
  }

  /** Deletes a target's texture and framebuffer. */
  free(target: Target): void {
    this.gl.deleteTexture(target.texture);
    this.gl.deleteFramebuffer(target.framebuffer);
    this.owned.textures.delete(target.texture);
    this.owned.framebuffers.delete(target.framebuffer);
  }

  /** Deletes a framebuffer created with {@link GlKit.framebuffer}. */
  freeFramebuffer(framebuffer: WebGLFramebuffer): void {
    this.gl.deleteFramebuffer(framebuffer);
    this.owned.framebuffers.delete(framebuffer);
  }

  /** Binds `texture` to unit `unit` and points sampler `name` at it. */
  bind(program: Program, name: string, unit: number, texture: WebGLTexture): void {
    const { gl } = this;
    gl.activeTexture(gl.TEXTURE0 + unit);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.uniform1i(program.uniform(name), unit);
  }

  /**
   * Draws one triangle over `viewport` of `framebuffer` (`null` = canvas)
   * with `program`, after `setup` set its uniforms; `scissor` limits the
   * pixels written.
   */
  pass(
    framebuffer: WebGLFramebuffer | null,
    program: Program,
    viewport: Box,
    setup: () => void,
    scissor?: Box,
  ): void {
    const { gl } = this;
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.useProgram(program.program);
    gl.viewport(...viewport);
    if (scissor) {
      gl.enable(gl.SCISSOR_TEST);
      gl.scissor(...scissor);
    }
    setup();
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    if (scissor) gl.disable(gl.SCISSOR_TEST);
  }

  /** Copies `box` of one framebuffer to `(x, y)` of another. */
  copy(from: WebGLFramebuffer, to: WebGLFramebuffer, box: Box, x = box[0], y = box[1]): void {
    const { gl } = this;
    const [x0, y0, w, h] = box;
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, from);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, to);
    gl.blitFramebuffer(x0, y0, x0 + w, y0 + h, x, y, x + w, y + h, gl.COLOR_BUFFER_BIT, gl.NEAREST);
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
  }
}
