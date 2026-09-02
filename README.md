# git-lex-ui

A front door to every git-lex repo on the machine.

Before this, looking at a soul meant knowing its path, changing directory to
it, running `git lex serve viz --port N`, and remembering which port you gave
which soul. This starts once on a fixed port, reads the machine registry,
drops the entries that have rotted, shows what is left, and starts the right
server when you pick one.

## Running it

```
cargo run -- --port 8888          # the front door
cd web && npm install && npm run build
```

`--no-prune` reads the registry without rewriting it. `--no-open` skips the
browser. `--web <dir>` points at a different frontend build.

In development, `cd web && npm run dev` serves the frontend on 5173 and
forwards `/api` and `/r` to the Rust process on 8888, so the URLs are the
same in development as in a build.

## The interface

Three fixed zones. The rails keep their home and their width; the stage takes
what is left. Nothing opens over the graph or moves when the state changes.

- **Left** — every soul on the machine, the view switcher, a search box, and
  the class list with per-class counts. Classes toggle; a class switched off
  is genuinely removed from the stage, not merely faded, because a filter that
  leaves things pickable is a filter you cannot trust.
- **Centre** — the graph. *Whole soul* lays documents out on a spiral where
  angle is time. *Neighbourhood* puts one document at the centre and rings its
  links by hop distance — deliberately not a force simulation, because "what
  is this connected to, and how far away" has an exact answer that should not
  wobble or settle differently on a second look.
- **Right** — the selected document: its class, birth commit, change count,
  its links in and out, and **every triple that mentions it, on both planes**.
  Object URIs are clickable, including declared references that point at a
  Thing rather than at its file.

A soul is addressed by its genesis sha and a document by its URI, so
`/?doc=<uri>#<genesis>` is a durable link to one row in one soul.

## How it is put together

The browser only ever talks to this process. Per-repo `git-lex-serve`
instances are children behind a proxy at `/r/<genesis-sha>/api/…`.

Repos are addressed by **genesis sha** — the hash of their first commit,
which is also what `.lex/repo.yml` and the running server both declare as the
repo's identity. Not by path, and not by port. A path is where a repo is
sitting today; a port is where a server happens to be listening this minute.
Neither is a name. This is why a link to a soul survives the repo being moved.

## Things that are true about this machine, and shaped the build

Measured 2026-09-01 against the live registry, not assumed:

- The registry is **JSON**, though the original brief described YAML. The
  format is sniffed from the file contents, not the extension.
- **28 of 50 entries point at directories that no longer exist.** Rot is the
  normal case, so pruning is not tidy-up.
- **46 of 50 have no `last_used`.** Sorting by it alone would tie 92% of rows
  while looking deliberate, so rows fall back to the repo's own last commit —
  and every row says which clock it used.
- **Six registry entries are scratch directories**, not two. Job directories
  and agent scratchpads both carry real `.lex/` state.
- The registry is **live**: it gained an entry mid-session while other agents
  were saving. The prune re-reads at write time and removes only what it
  condemned, so a repo registered in the window is not silently dropped.
- `git lex serve viz` is a **three-process chain**; only the innermost
  `git-lex-serve` binds the port. Killing the wrapper leaves the server
  running. This spawns `git-lex-serve` directly.
- That server **picks its own port**, walking up to 20 from the one it is
  given. So the port is read back off its stdout, and then the identity of
  whatever answered is verified against the expected genesis sha.

## What it does not do

It reads and serves. It does not write to any repo, edit documents, or sync.
