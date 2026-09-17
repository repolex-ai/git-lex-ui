/**
 * WebGL2 renderer for the graph stage.
 *
 * Hand-written, no charting library. The layout arrives already computed as
 * typed arrays, so the browser's job is to put them in buffers and draw.
 *
 * Two things are dynamic and everything else is static. **Positions** are
 * dynamic because the stage swaps between layouts — the spiral, where
 * position means time, and a neighbourhood, where position means distance
 * from one node. **State** is dynamic because filtering and selection change
 * constantly and must not cost a re-upload of the whole graph.
 *
 * Pan and zoom are uniforms, never a re-upload: dragging a 7,300-node soul
 * costs exactly what dragging a 147-node one costs.
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
  /** u16 per edge: an index into the layout's predicate table. */
  edge_predicates: number
  edge_predicates_bytes: number
  /** Commit ordinal each link was first asserted in, parallel to `edges`.
   *  `0xFFFFFFFF` means the store records no birthday for that link. */
  edge_born: number
  edge_born_bytes: number
  total: number
}

export interface View {
  scale: number
  x: number
  y: number
}

/** Per-node draw state. Ordered so that a larger number draws louder. */
export const NodeState = {
  /** Filtered out entirely — not drawn, and not pickable. */
  Hidden: 0,
  /** Present but pushed back: context, not subject. */
  Dim: 1,
  Normal: 2,
  /** A search or neighbourhood match. */
  Marked: 3,
  Selected: 4,
} as const

const POINT_VS = `#version 300 es
in vec2 a_pos;
in vec3 a_color;
in float a_size;
in float a_state;
uniform vec2 u_offset;
uniform float u_scale;
uniform vec2 u_viewport;
uniform float u_dpr;
out vec3 v_color;
out float v_alpha;
out float v_ring;
void main() {
  if (a_state < 0.5) {
    // Hidden: collapse to a degenerate point off-screen rather than
    // discarding in the fragment stage, so it costs nothing downstream.
    gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
    gl_PointSize = 0.0;
    return;
  }
  vec2 p = (a_pos + u_offset) * u_scale;
  p.x /= u_viewport.x / u_viewport.y;
  gl_Position = vec4(p, 0.0, 1.0);

  float grow = a_state > 3.5 ? 2.1 : (a_state > 2.5 ? 1.5 : 1.0);
  gl_PointSize = a_size * u_dpr * (0.6 + 0.4 * sqrt(u_scale)) * grow;

  // Dimmed nodes keep their position and lose their voice. Washing them
  // toward the page rather than hiding them is what keeps a filtered view
  // honest: you can still see how much you are not looking at.
  float dim = a_state < 1.5 ? 0.13 : 1.0;
  v_color = mix(vec3(1.0), a_color, dim);
  v_alpha = a_state < 1.5 ? 0.5 : 1.0;
  v_ring = a_state > 3.5 ? 1.0 : 0.0;
}`

const POINT_FS = `#version 300 es
precision mediump float;
in vec3 v_color;
in float v_alpha;
in float v_ring;
out vec4 outColor;
void main() {
  vec2 d = gl_PointCoord - vec2(0.5);
  float r = length(d);
  float a = smoothstep(0.5, 0.42, r) * v_alpha;
  if (a <= 0.0) discard;
  // The selected node wears a ring rather than just being bigger, so it is
  // findable in a field of thousands of dots that are also big.
  if (v_ring > 0.5 && r > 0.34 && r < 0.46) {
    outColor = vec4(0.05, 0.05, 0.05, a);
    return;
  }
  outColor = vec4(v_color, a);
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

export class GraphRenderer {
  private gl: WebGL2RenderingContext
  private pointProg: WebGLProgram
  private lineProg: WebGLProgram
  private vaoPoints: WebGLVertexArrayObject
  private vaoLines: WebGLVertexArrayObject
  private vaoTrack: WebGLVertexArrayObject
  private posBuf: WebGLBuffer
  private stateBuf: WebGLBuffer
  private edgeBuf: WebGLBuffer
  private trackBuf: WebGLBuffer
  private trackCount = 0
  /** Edge indices currently uploaded — a subset when a neighbourhood is shown. */
  private edgeCount = 0

  readonly n: number
  positions: Float32Array
  /** When each link was first asserted, one per edge. */
  edgeBorn: Uint32Array
  /** The spiral track, retained for `fitView`. Empty until `setTrack`. */
  track: Float32Array = new Float32Array(0)
  colors: Uint8Array
  sizes: Float32Array
  edges: Uint32Array
  states: Uint8Array

  private grid = new Map<string, number[]>()
  private cell = 0.05

  constructor(
    private canvas: HTMLCanvasElement,
    buffer: ArrayBuffer,
    offsets: LayoutOffsets,
    nodeCount: number,
  ) {
    const gl = canvas.getContext('webgl2', { antialias: true, alpha: false })
    if (!gl) throw new Error('WebGL2 is not available in this browser')
    this.gl = gl
    this.n = nodeCount

    this.positions = new Float32Array(buffer.slice(offsets.positions, offsets.positions + offsets.positions_bytes))
    this.edgeBorn = new Uint32Array(
      buffer.slice(offsets.edge_born, offsets.edge_born + offsets.edge_born_bytes),
    )
    this.colors = new Uint8Array(buffer.slice(offsets.colors, offsets.colors + offsets.colors_bytes))
    this.sizes = new Float32Array(buffer.slice(offsets.sizes, offsets.sizes + offsets.sizes_bytes))
    this.edges = new Uint32Array(buffer.slice(offsets.edges, offsets.edges + offsets.edges_bytes))
    this.states = new Uint8Array(nodeCount).fill(NodeState.Normal)

    this.pointProg = program(gl, POINT_VS, POINT_FS)
    this.lineProg = program(gl, LINE_VS, LINE_FS)

    this.posBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, this.posBuf)
    gl.bufferData(gl.ARRAY_BUFFER, this.positions, gl.DYNAMIC_DRAW)

    const colBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, colBuf)
    gl.bufferData(gl.ARRAY_BUFFER, this.colors, gl.STATIC_DRAW)

    const sizBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, sizBuf)
    gl.bufferData(gl.ARRAY_BUFFER, this.sizes, gl.STATIC_DRAW)

    this.stateBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, this.stateBuf)
    gl.bufferData(gl.ARRAY_BUFFER, this.states, gl.DYNAMIC_DRAW)

    const bind = (
      prog: WebGLProgram, name: string, buf: WebGLBuffer,
      size: number, type: number, norm: boolean,
    ) => {
      const loc = gl.getAttribLocation(prog, name)
      if (loc < 0) return
      gl.bindBuffer(gl.ARRAY_BUFFER, buf)
      gl.enableVertexAttribArray(loc)
      gl.vertexAttribPointer(loc, size, type, norm, 0, 0)
    }

    this.vaoPoints = gl.createVertexArray()!
    gl.bindVertexArray(this.vaoPoints)
    bind(this.pointProg, 'a_pos', this.posBuf, 2, gl.FLOAT, false)
    bind(this.pointProg, 'a_color', colBuf, 3, gl.UNSIGNED_BYTE, true)
    bind(this.pointProg, 'a_size', sizBuf, 1, gl.FLOAT, false)
    bind(this.pointProg, 'a_state', this.stateBuf, 1, gl.UNSIGNED_BYTE, false)

    this.vaoLines = gl.createVertexArray()!
    gl.bindVertexArray(this.vaoLines)
    bind(this.lineProg, 'a_pos', this.posBuf, 2, gl.FLOAT, false)
    this.edgeBuf = gl.createBuffer()!
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.edgeBuf)
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, this.edges, gl.DYNAMIC_DRAW)
    this.edgeCount = this.edges.length

    this.trackBuf = gl.createBuffer()!
    this.vaoTrack = gl.createVertexArray()!
    gl.bindVertexArray(this.vaoTrack)
    bind(this.lineProg, 'a_pos', this.trackBuf, 2, gl.FLOAT, false)

    gl.bindVertexArray(null)
    this.rebuildGrid()
  }

  /** The spiral track, drawn under the dots. Without it an Archimedean
   *  spiral at two turns is indistinguishable from a blob, and the viewer has
   *  no way to know that angle means time. Empty in layouts where it would be
   *  a lie. */
  setTrack(points: Float32Array) {
    // Kept so the fit can include it. The track is drawn, so it counts as
    // content — fitting only the nodes framed the dots correctly and let the
    // spiral run off three edges, which reads as a broken view rather than a
    // deliberate crop.
    this.track = points
    const gl = this.gl
    gl.bindBuffer(gl.ARRAY_BUFFER, this.trackBuf)
    gl.bufferData(gl.ARRAY_BUFFER, points, gl.STATIC_DRAW)
    this.trackCount = points.length / 2
  }

  setPositions(p: Float32Array) {
    this.positions = p
    const gl = this.gl
    gl.bindBuffer(gl.ARRAY_BUFFER, this.posBuf)
    gl.bufferData(gl.ARRAY_BUFFER, p, gl.DYNAMIC_DRAW)
    this.rebuildGrid()
  }

  setStates(s: Uint8Array) {
    this.states = s
    const gl = this.gl
    gl.bindBuffer(gl.ARRAY_BUFFER, this.stateBuf)
    gl.bufferData(gl.ARRAY_BUFFER, s, gl.DYNAMIC_DRAW)
  }

  /** Draw only these edges. Used by the neighbourhood view, which shows one
   *  node's links and would otherwise be buried under every other edge. */
  setEdgeSubset(indices: Uint32Array) {
    const gl = this.gl
    gl.bindVertexArray(this.vaoLines)
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.edgeBuf)
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.DYNAMIC_DRAW)
    this.edgeCount = indices.length
    gl.bindVertexArray(null)
  }

  private rebuildGrid() {
    this.grid.clear()
    for (let i = 0; i < this.n; i++) {
      const k = `${Math.floor(this.positions[i * 2] / this.cell)},${Math.floor(this.positions[i * 2 + 1] / this.cell)}`
      const b = this.grid.get(k)
      if (b) b.push(i)
      else this.grid.set(k, [i])
    }
  }

  /** Nearest visible node, searching the nine cells around the query rather
   *  than all nodes, so hover cost does not grow with the size of the soul.
   *  Hidden nodes are never picked — a node you cannot see must not be a node
   *  you can accidentally select. */
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
          if (this.states[i] === NodeState.Hidden) continue
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

    const vw = gl.drawingBufferWidth
    const vh = gl.drawingBufferHeight

    const setLine = (rgba: [number, number, number, number]) => {
      gl.useProgram(this.lineProg)
      gl.uniform2f(gl.getUniformLocation(this.lineProg, 'u_offset'), view.x, view.y)
      gl.uniform1f(gl.getUniformLocation(this.lineProg, 'u_scale'), view.scale)
      gl.uniform2f(gl.getUniformLocation(this.lineProg, 'u_viewport'), vw, vh)
      gl.uniform4f(gl.getUniformLocation(this.lineProg, 'u_color'), ...rgba)
    }

    if (this.trackCount) {
      setLine([0, 0, 0, 0.26])
      gl.bindVertexArray(this.vaoTrack)
      gl.drawArrays(gl.LINE_STRIP, 0, this.trackCount)
    }

    if (showEdges && this.edgeCount) {
      setLine([0.1, 0.1, 0.15, 0.13])
      gl.bindVertexArray(this.vaoLines)
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.edgeBuf)
      gl.drawElements(gl.LINES, this.edgeCount, gl.UNSIGNED_INT, 0)
    }

    gl.useProgram(this.pointProg)
    gl.uniform2f(gl.getUniformLocation(this.pointProg, 'u_offset'), view.x, view.y)
    gl.uniform1f(gl.getUniformLocation(this.pointProg, 'u_scale'), view.scale)
    gl.uniform2f(gl.getUniformLocation(this.pointProg, 'u_viewport'), vw, vh)
    gl.uniform1f(gl.getUniformLocation(this.pointProg, 'u_dpr'), dpr)
    gl.bindVertexArray(this.vaoPoints)
    gl.drawArrays(gl.POINTS, 0, this.n)
    gl.bindVertexArray(null)
  }

  toLayout(px: number, py: number, view: View): [number, number] {
    const rect = this.canvas.getBoundingClientRect()
    const aspect = rect.width / rect.height
    const ndcX = ((px - rect.left) / rect.width) * 2 - 1
    const ndcY = 1 - ((py - rect.top) / rect.height) * 2
    return [(ndcX * aspect) / view.scale - view.x, ndcY / view.scale - view.y]
  }

  /** A view that frames the drawn content, whatever its extent.
   *
   *  The camera used to open at a fixed scale for every soul. That is one
   *  zoom level shared by a 51-document soul and a 7,651-document one — the
   *  small one opens as a dot in the middle of an empty stage, the large one
   *  overflows, and in both cases the first thing you do is fight the view
   *  before you can read anything. Called out by @goodlux as the panning and
   *  zooming not feeling right; the panning was fine, it was starting in the
   *  wrong place.
   *
   *  Fits the bounding box of the actual positions, with a margin, so the
   *  first frame is already the whole picture. `subset` lets a filtered view
   *  frame only what it draws. */
  fitView(aspect: number, subset?: Uint32Array | number[]): View {
    const n = this.n
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity
    const visit = (i: number) => {
      const x = this.positions[i * 2]
      const y = this.positions[i * 2 + 1]
      if (!Number.isFinite(x) || !Number.isFinite(y)) return
      if (x < minX) minX = x
      if (x > maxX) maxX = x
      if (y < minY) minY = y
      if (y > maxY) maxY = y
    }
    if (subset && subset.length) {
      for (const i of subset) if (i < n) visit(i)
    } else {
      for (let i = 0; i < n; i++) visit(i)
      // Include the track. It is real drawn content and it reaches further
      // than the nodes do: documents occupy only part of the commit range,
      // but the spiral is drawn for all of it, so the outermost thing on
      // screen is usually track rather than a node.
      for (let i = 0; i < this.track.length; i += 2) {
        const x = this.track[i]
        const y = this.track[i + 1]
        if (!Number.isFinite(x) || !Number.isFinite(y)) continue
        if (x < minX) minX = x
        if (x > maxX) maxX = x
        if (y < minY) minY = y
        if (y > maxY) maxY = y
      }
    }
    // Nothing finite to frame — a soul with no drawable nodes. Return the
    // old fixed view rather than a NaN camera, which renders as a blank
    // stage indistinguishable from an empty graph.
    if (!Number.isFinite(minX) || maxX < minX) return { scale: 0.92, x: 0, y: 0 }

    const cx = (minX + maxX) / 2
    const cy = (minY + maxY) / 2
    // A single node, or a perfectly flat run, has zero extent in one axis.
    // Floor the span so the scale stays finite.
    const spanX = Math.max(maxX - minX, 1e-3)
    const spanY = Math.max(maxY - minY, 1e-3)
    // Clip space runs -aspect..aspect horizontally and -1..1 vertically, so
    // the usable half-extents are `aspect` and 1. 0.88 leaves a margin, and
    // stops nodes on the rim being clipped by their own point size.
    const scale = Math.min((2 * aspect * 0.88) / spanX, (2 * 0.88) / spanY)
    return { scale: Math.min(80, Math.max(0.05, scale)), x: -cx, y: -cy }
  }

  /** Centre the view on a node without changing zoom. */
  centreOn(i: number, view: View): View {
    return { ...view, x: -this.positions[i * 2], y: -this.positions[i * 2 + 1] }
  }
}
