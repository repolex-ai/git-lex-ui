/**
 * WebGL2 renderer for the Whole Soul spiral.
 *
 * Hand-written, no charting library. Nothing off the shelf draws six
 * thousand nodes on a deterministic layout and holds a frame budget, and the
 * layout arrives already computed as typed arrays — so the browser's whole
 * job is to put them in buffers and draw. A library would be weight between
 * this and the GPU, buying nothing.
 *
 * Positions, colours and sizes come from the server as one packed buffer and
 * are uploaded once. Pan and zoom are a uniform, not a re-upload: no per-frame
 * CPU work proportional to the node count, which is what keeps a 6,400-node
 * soul as cheap to drag as a 147-node one.
 */

export interface LayoutOffsets {
  positions: number
  positions_bytes: number
  colors: number
  colors_bytes: number
  sizes: number
  sizes_bytes: number
  edges: number
  edges_bytes: number
  total: number
}

export interface View {
  scale: number
  x: number
  y: number
}

const POINT_VS = `#version 300 es
in vec2 a_pos;
in vec3 a_color;
in float a_size;
uniform vec2 u_offset;
uniform float u_scale;
uniform vec2 u_viewport;
uniform float u_dpr;
uniform float u_dim;
out vec3 v_color;
out float v_dim;
void main() {
  vec2 p = (a_pos + u_offset) * u_scale;
  // Aspect-correct: the spiral is round, and it must stay round in a
  // window that is not.
  float aspect = u_viewport.x / u_viewport.y;
  p.x /= aspect;
  gl_Position = vec4(p, 0.0, 1.0);
  // Size grows with zoom but sub-linearly, so a zoomed-in view shows more
  // detail rather than a screen of overlapping discs.
  gl_PointSize = a_size * u_dpr * (0.6 + 0.4 * sqrt(u_scale));
  v_color = a_color;
  v_dim = u_dim;
}`

const POINT_FS = `#version 300 es
precision mediump float;
in vec3 v_color;
in float v_dim;
out vec4 outColor;
void main() {
  // Round points with a soft edge. gl_PointCoord is the only cheap way to
  // get a disc out of a square point sprite.
  vec2 d = gl_PointCoord - vec2(0.5);
  float r = length(d);
  float a = smoothstep(0.5, 0.42, r);
  if (a <= 0.0) discard;
  outColor = vec4(mix(vec3(1.0), v_color, v_dim), a);
}`

const LINE_VS = `#version 300 es
in vec2 a_pos;
uniform vec2 u_offset;
uniform float u_scale;
uniform vec2 u_viewport;
void main() {
  vec2 p = (a_pos + u_offset) * u_scale;
  p.x /= u_viewport.x / u_viewport.y;
  gl_Position = vec4(p, 0.0, 1.0);
}`

const LINE_FS = `#version 300 es
precision mediump float;
uniform vec4 u_color;
out vec4 outColor;
void main() { outColor = u_color; }`

function compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
  const s = gl.createShader(type)!
  gl.shaderSource(s, src)
  gl.compileShader(s)
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    throw new Error(`shader: ${gl.getShaderInfoLog(s)}`)
  }
  return s
}

function program(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const p = gl.createProgram()!
  gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, vs))
  gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs))
  gl.linkProgram(p)
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    throw new Error(`link: ${gl.getProgramInfoLog(p)}`)
  }
  return p
}

export class SpiralRenderer {
  private gl: WebGL2RenderingContext
  private pointProg: WebGLProgram
  private lineProg: WebGLProgram
  private vaoPoints: WebGLVertexArrayObject
  private vaoLines: WebGLVertexArrayObject
  private vaoTrack: WebGLVertexArrayObject
  private vaoTicks: WebGLVertexArrayObject
  private edgeBuf: WebGLBuffer
  private trackCount = 0
  private tickCount = 0

  positions: Float32Array
  colors: Uint8Array
  sizes: Float32Array
  edges: Uint32Array

  /** A uniform grid over layout space, for hit-testing without walking every
   *  node on every mouse move. */
  private grid = new Map<string, number[]>()
  private cell = 0.05

  constructor(
    private canvas: HTMLCanvasElement,
    buffer: ArrayBuffer,
    offsets: LayoutOffsets,
    nodeCount: number,
    turns: number,
  ) {
    const gl = canvas.getContext('webgl2', {
      antialias: true,
      alpha: false,
      premultipliedAlpha: false,
    })
    if (!gl) throw new Error('WebGL2 is not available in this browser')
    this.gl = gl

    this.positions = new Float32Array(buffer, offsets.positions, nodeCount * 2)
    this.colors = new Uint8Array(buffer, offsets.colors, nodeCount * 3)
    this.sizes = new Float32Array(buffer, offsets.sizes, nodeCount)
    this.edges = new Uint32Array(buffer, offsets.edges, offsets.edges_bytes / 4)

    this.pointProg = program(gl, POINT_VS, POINT_FS)
    this.lineProg = program(gl, LINE_VS, LINE_FS)

    const posBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, posBuf)
    gl.bufferData(gl.ARRAY_BUFFER, this.positions, gl.STATIC_DRAW)

    const colBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, colBuf)
    gl.bufferData(gl.ARRAY_BUFFER, this.colors, gl.STATIC_DRAW)

    const sizBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, sizBuf)
    gl.bufferData(gl.ARRAY_BUFFER, this.sizes, gl.STATIC_DRAW)

    // Points
    this.vaoPoints = gl.createVertexArray()!
    gl.bindVertexArray(this.vaoPoints)
    const bind = (prog: WebGLProgram, name: string, buf: WebGLBuffer, size: number, type: number, norm: boolean) => {
      const loc = gl.getAttribLocation(prog, name)
      if (loc < 0) return
      gl.bindBuffer(gl.ARRAY_BUFFER, buf)
      gl.enableVertexAttribArray(loc)
      gl.vertexAttribPointer(loc, size, type, norm, 0, 0)
    }
    bind(this.pointProg, 'a_pos', posBuf, 2, gl.FLOAT, false)
    bind(this.pointProg, 'a_color', colBuf, 3, gl.UNSIGNED_BYTE, true)
    bind(this.pointProg, 'a_size', sizBuf, 1, gl.FLOAT, false)

    // Edges — drawn as indexed lines over the same position buffer, so the
    // edge data on the wire is two integers per link and nothing more.
    this.vaoLines = gl.createVertexArray()!
    gl.bindVertexArray(this.vaoLines)
    bind(this.lineProg, 'a_pos', posBuf, 2, gl.FLOAT, false)
    this.edgeBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.edgeBuf)
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, this.edges, gl.STATIC_DRAW)

    // The spiral track itself, as a hairline under the dots. Without it, at
    // two turns an Archimedean spiral is indistinguishable from a blob, and
    // the viewer has no way to know that angle means time.
    const track: number[] = []
    const steps = Math.max(400, turns * 400)
    for (let i = 0; i <= steps; i++) {
      const t = i / steps
      const th = 2 * Math.PI * turns * t
      const r = 0.12 + 0.86 * t
      track.push(Math.cos(th) * r, Math.sin(th) * r)
    }
    this.trackCount = track.length / 2
    const trackBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, trackBuf)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(track), gl.STATIC_DRAW)
    this.vaoTrack = gl.createVertexArray()!
    gl.bindVertexArray(this.vaoTrack)
    bind(this.lineProg, 'a_pos', trackBuf, 2, gl.FLOAT, false)

    // Turn boundaries, as short radial ticks. The track alone says the
    // layout is a spiral; the ticks say where one lap ends and the next
    // begins, which is what makes "one turn is one slice of its life"
    // something a reader can check rather than take on trust.
    const ticks: number[] = []
    for (let k = 0; k <= turns; k++) {
      const t = k / turns
      const th = 2 * Math.PI * turns * t
      const r = 0.12 + 0.86 * t
      // Short. Every lap of a spiral crosses angle zero, so these ticks are
      // collinear by construction — drawn long they merge into one stray
      // horizontal rule that reads as a rendering artifact rather than as a
      // scale.
      const g = (0.86 / turns) * 0.12
      ticks.push(Math.cos(th) * (r - g), Math.sin(th) * (r - g))
      ticks.push(Math.cos(th) * (r + g), Math.sin(th) * (r + g))
    }
    this.tickCount = ticks.length / 2
    const tickBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, tickBuf)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(ticks), gl.STATIC_DRAW)
    this.vaoTicks = gl.createVertexArray()!
    gl.bindVertexArray(this.vaoTicks)
    bind(this.lineProg, 'a_pos', tickBuf, 2, gl.FLOAT, false)

    gl.bindVertexArray(null)
    this.buildGrid(nodeCount)
  }

  private key(x: number, y: number): string {
    return `${Math.floor(x / this.cell)},${Math.floor(y / this.cell)}`
  }

  private buildGrid(n: number) {
    for (let i = 0; i < n; i++) {
      const k = this.key(this.positions[i * 2], this.positions[i * 2 + 1])
      const b = this.grid.get(k)
      if (b) b.push(i)
      else this.grid.set(k, [i])
    }
  }

  /** Nearest node to a point in layout space, within `maxDist`. Searches the
   *  nine cells around the query rather than all nodes, so hover cost does
   *  not grow with the size of the soul. */
  pick(x: number, y: number, maxDist: number): number | null {
    let best: number | null = null
    let bestD = maxDist * maxDist
    const cx = Math.floor(x / this.cell)
    const cy = Math.floor(y / this.cell)
    for (let gx = cx - 1; gx <= cx + 1; gx++) {
      for (let gy = cy - 1; gy <= cy + 1; gy++) {
        const b = this.grid.get(`${gx},${gy}`)
        if (!b) continue
        for (const i of b) {
          const dx = this.positions[i * 2] - x
          const dy = this.positions[i * 2 + 1] - y
          const d = dx * dx + dy * dy
          if (d < bestD) {
            bestD = d
            best = i
          }
        }
      }
    }
    return best
  }

  resize(dpr: number) {
    const w = Math.max(1, Math.floor(this.canvas.clientWidth * dpr))
    const h = Math.max(1, Math.floor(this.canvas.clientHeight * dpr))
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w
      this.canvas.height = h
    }
    this.gl.viewport(0, 0, w, h)
  }

  draw(view: View, dpr: number, showEdges: boolean) {
    const gl = this.gl
    gl.clearColor(1, 1, 1, 1)
    gl.clear(gl.COLOR_BUFFER_BIT)
    gl.enable(gl.BLEND)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)

    const vp: [number, number] = [gl.drawingBufferWidth, gl.drawingBufferHeight]

    const setLine = (rgba: [number, number, number, number]) => {
      gl.useProgram(this.lineProg)
      gl.uniform2f(gl.getUniformLocation(this.lineProg, 'u_offset'), view.x, view.y)
      gl.uniform1f(gl.getUniformLocation(this.lineProg, 'u_scale'), view.scale)
      gl.uniform2f(gl.getUniformLocation(this.lineProg, 'u_viewport'), vp[0], vp[1])
      gl.uniform4f(gl.getUniformLocation(this.lineProg, 'u_color'), ...rgba)
    }

    // Track first, under everything — but legibly. At 10% it read as an
    // artifact and the dots looked like they were floating free, which
    // silently withdraws the one claim the view is making: that angle is
    // time. A form that does not supply the structure a reader is reaching
    // for is undersupplied.
    setLine([0, 0, 0, 0.26])
    gl.bindVertexArray(this.vaoTrack)
    gl.drawArrays(gl.LINE_STRIP, 0, this.trackCount)

    setLine([0, 0, 0, 0.42])
    gl.bindVertexArray(this.vaoTicks)
    gl.drawArrays(gl.LINES, 0, this.tickCount)

    if (showEdges && this.edges.length) {
      setLine([0.1, 0.1, 0.15, 0.13])
      gl.bindVertexArray(this.vaoLines)
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.edgeBuf)
      gl.drawElements(gl.LINES, this.edges.length, gl.UNSIGNED_INT, 0)
    }

    gl.useProgram(this.pointProg)
    gl.uniform2f(gl.getUniformLocation(this.pointProg, 'u_offset'), view.x, view.y)
    gl.uniform1f(gl.getUniformLocation(this.pointProg, 'u_scale'), view.scale)
    gl.uniform2f(gl.getUniformLocation(this.pointProg, 'u_viewport'), vp[0], vp[1])
    gl.uniform1f(gl.getUniformLocation(this.pointProg, 'u_dpr'), dpr)
    gl.uniform1f(gl.getUniformLocation(this.pointProg, 'u_dim'), 1.0)
    gl.bindVertexArray(this.vaoPoints)
    gl.drawArrays(gl.POINTS, 0, this.sizes.length)
    gl.bindVertexArray(null)
  }

  /** Layout-space coordinates for a pixel, so hit-testing and the pointer
   *  agree at every zoom level. */
  toLayout(px: number, py: number, view: View): [number, number] {
    const rect = this.canvas.getBoundingClientRect()
    const aspect = rect.width / rect.height
    const ndcX = ((px - rect.left) / rect.width) * 2 - 1
    const ndcY = 1 - ((py - rect.top) / rect.height) * 2
    return [(ndcX * aspect) / view.scale - view.x, ndcY / view.scale - view.y]
  }
}
