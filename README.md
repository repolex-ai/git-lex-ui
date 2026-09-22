# git-lex-ui

A front door to every git-lex repository on the machine.

It starts on a fixed port (`8888`), reads the local repository registry, drops stale entries, and renders an interactive, deterministic picture of each repository's graph. Built with a Rust backend ([Axum](https://github.com/tokio-rs/axum)) and a [Svelte 5](https://svelte.dev) frontend powered by a custom WebGL2 renderer.

---

## What It Is

`git-lex-ui` provides a single visual entry point for navigating and inspecting git-lex repositories across your workstation.

Instead of hunting for local repository paths or memorizing transient network ports, you run one process. It displays a segmented list of every repository, visualizes how documents connect across history, and provides an inspector pane exposing triples on both the File plane and the Thing plane.

* **Left Rail:** The machine repository index (segmented by family: soul, kitted, or plain markdown), recency clocks, search filters, and an interactive class toggle that cleanly removes unselected classes from the stage.
* **Stage:** A hardware-accelerated WebGL2 canvas displaying the document graph in either a chronological spiral layout or a link neighbourhood view.
* **Right Inspector:** Deep metadata for the selected document—its declared class, birth commit, change frequency, inbound/outbound links, and every RDF triple that references it.

Every repository is addressed by its **genesis SHA** and every document by its id, making `/?doc=<id>#<genesis>` a shareable, permanent deep link. `?reading=base|typed` and `?links=<names>` ride along when they differ from the defaults, so a link carries the view you were actually looking at.

---

## Running It

### Production Build

Build the Svelte 5 frontend assets, then start the server:

```bash
cd web && npm install && npm run build
cargo run --release -- --port 8888
```

The server binds to `127.0.0.1:8888` and automatically opens your default browser.

### Flags

| Flag | Default | Description |
|---|---|---|
| `--port <n>` | `8888` | Port for the front door server. |
| `--no-open` | `false` | Skips launching the browser on startup. |
| `--no-prune` | `false` | Reads the machine registry without rewriting it (read-only mode). |
| `--web <dir>` | `./web/dist` | Points to a custom frontend build directory (read on every request). |
| `--daemon-port <n>` | `7880` | Port where `gitlexd` is listening (useful for testing). |

### Development Mode

Run the frontend via Vite to enable hot module replacement:

```bash
# Terminal 1: Run the backend
cargo run -- --port 8888

# Terminal 2: Run the frontend dev server
cd web && npm run dev
```

Vite serves the UI on `http://localhost:5173` and automatically proxies `/api` and `/r` requests to `127.0.0.1:8888`, ensuring route behavior is identical between development and production builds.

---

## The Two Readings of a Repo

`git-lex-ui` offers two distinct readings of any repository, switchable via `?view=base|typed`:

### 1. Base (`?view=base`)
* **What it draws:** Every markdown file in the repository, positioned at the commit git records it first appearing in, sized by its commit frequency, and colored by directory tree.
* **Requirements:** None. Works on **any git repository** holding markdown files—no kit installation, no YAML frontmatter, and no RDF ontologies required.
* **Why it matters:** git-lex's foundational substrate is plain markdown in git. This reading visualizes the physical document landscape as it actually evolved.

### 2. Typed (`?view=typed`)
* **What it draws:** High-level entities from the store's current `now` view, colored by their ontology class (`soul:Journal`, `soul:Note`, `copia:Texture`, etc.).
* **Requirements:** Requires a kit to have typed documents and an executed sync.
* **Fallback behavior:** Plain repositories open in base view automatically. Kitted repositories default to typed view, falling back to base view with an explanatory note if the store holds no typed entities.

---

## The Graph Layout

The stage renders graphs deterministically using server-side ordinal calculations, avoiding chaotic runtime physics:

### The Whole-Repo Spiral
Documents are laid out along an Archimedean spiral:
* **Angle & Radius:** The center represents the genesis commit; the outer rim represents `HEAD`. Angle is calculated strictly from the commit's position in history (`g2:ordinalDerived`).
* **Lateral Spreading:** Documents introduced in bulk (such as import commits or batch migrations) are spread laterally across the spiral track's *normal*, never along the angle. Spreading cohort commits along the angle would distort an afternoon's work into looking like three weeks of history.
* **Node Size:** Proportional to the number of commits touching that document.
* **Edge Routing:** Two link kinds, drawn from the commit that first asserted each one, so a link appears when it was written rather than all at once.
  * `md:linksTo` — a markdown link in a document's body. Note the namespace: it is `https://repolex.ai/ontology/git-lex/md/linksTo`, **not** `gl:linksTo`, which does not exist.
  * `gl:relatedToId` — a reference a document declares in its frontmatter. On lUX this is 14,529 of 18,131 drawn links; a view that quietly omits it loses most of the graph.

### The Neighbourhood Hop Ring
When a specific document is selected, the view can switch to its immediate neighborhood:
* The selected document is pinned at the center.
* Connected documents are arrayed on concentric rings determined by exact link distance (1 hop, 2 hops, etc.).
* **No force-directed simulation:** "What is this connected to, and how far away is it?" has an exact, unambiguous mathematical answer. It should never wobble, drift, or settle differently on a second glance.

---

## Data Architecture: `gitlexd`

All semantic state is served by **`gitlexd`**, a single machine-level daemon listening on `127.0.0.1:7880`.

* **Single Process:** `gitlexd` holds the embedded stores for every registered repository on the workstation and addresses each by its genesis SHA.
* **Automatic Supervision:** When `git-lex-ui` boots, it probes `http://127.0.0.1:7880/health`. If nothing responds, it spawns `gitlexd` in the background.
* **Port-as-Lock:** Port 7880 acts as the mutual exclusion lock. If a second daemon process is spawned concurrently, it detects the bound port and exits immediately without race conditions.
* **Direct Addressing:** The browser posts SPARQL to this server at `/r/<genesis-sha>/sparql`, which relays it to `gitlexd` at `/soul/<genesis-sha>/sparql`. The name travels the whole way down, so there are no per-repo port allocations and no process supervision. An earlier version of this tool ran one `git-lex-serve sparql` child per repository and spent 553 lines translating names into ports; that command no longer exists in git-lex, and neither does the translation.

---

## Three Architectural Decisions

### 1. Address Repos by Genesis SHA, Never by Path or Port
A filesystem path is merely where a repository sits today; a port is merely where a socket happens to be open this minute. Symlinks routinely create situations where two distinct paths point to the exact same repository (causing duplicate entries in naive registries). Genesis SHA—the hash of the repository's root commit (`git rev-list --max-parents=0 HEAD`)—is permanent, immutable, and universally unique across renames and disk migrations.

### 2. Query Freshness from the Writer, Not Disk Artifacts
Earlier iterations attempted to detect whether a graph was stale by comparing file modification timestamps on disk. However, storage engines like RocksDB rewrite their bookkeeping files, and can compact `.sst` data files, merely by being *opened*. Inferring that "the graph moved" because a file timestamp changed caused healthy repositories to falsely display stale-graph warnings. `git-lex-ui` queries `gitlexd` directly via `from_daemon()`, asking the active writer process what commit it has actually synced to.

### 3. Strict Rejection of Non-Envelope SPARQL Payloads
When flattening W3C SPARQL JSON results, the parser explicitly validates the presence of the `results.bindings` envelope. If the payload is an error object, HTML error page, or a 404 response, the parser returns an `Err` rather than falling back to an empty list `[]`. Downstream consumers must never confuse a broken route or failed request with a legitimate query that returned zero rows.

---

## Machine State & Verification

Measured against the active workstation registry:
* **29 repositories** registered on this machine, after folding the duplicate paths a symlink had created.
* **25** draw in the `base` reading — every repository that has ever been synced.
* **22** of those also draw in `typed`. The other three (`git-lex`, `git-lex-kit-base`, `repolex-development-docs`) are plain markdown with nothing typed in them, which is not a failure: it is the case the base reading exists for.
* **4** draw in neither. They are demo repositories that have never been synced, so there is no store to read a file tree from. This is the only state on the machine that cannot be drawn.
* **Zero** undated files in any reading.
* **23 tests** passing (`cargo test`).
