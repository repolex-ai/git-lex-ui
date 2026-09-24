<script lang="ts">
  import { marked } from 'marked'
  import type { FileText } from './types'

  interface Props {
    file: FileText | null
    loading: boolean
  }
  let { file, loading }: Props = $props()

  /** Strip the YAML frontmatter block.
   *
   *  The frontmatter is a document's *facts*, and the panel beside this one
   *  already shows every one of them as triples, resolved and clickable.
   *  Repeating it here as raw YAML pushed the actual prose below the fold —
   *  which is what the first version did, and why it read as a data dump
   *  rather than as a document. */
  function body(src: string): string {
    const m = src.match(/^---\r?\n[\s\S]*?\r?\n---\r?\n?/)
    return m ? src.slice(m[0].length) : src
  }

  /** Render markdown, then remove anything that could execute.
   *
   *  These files are local and written by us, so this is not a hostile input
   *  in practice. It is still rendered HTML built from a file on disk, and
   *  the cost of being careful is one pass over a detached tree. Scripts,
   *  frames, and inline event handlers come out; everything a note actually
   *  uses stays. */
  function render(src: string): string {
    const host = document.createElement('div')
    host.innerHTML = marked.parse(body(src), { async: false }) as string
    for (const el of Array.from(host.querySelectorAll('script, iframe, object, embed, style'))) {
      el.remove()
    }
    for (const el of Array.from(host.querySelectorAll('*'))) {
      for (const a of Array.from(el.attributes)) {
        const n = a.name.toLowerCase()
        if (n.startsWith('on') || (n === 'href' && a.value.trim().toLowerCase().startsWith('javascript:'))) {
          el.removeAttribute(a.name)
        }
      }
    }
    return host.innerHTML
  }

  const html = $derived(file?.text ? render(file.text) : null)
</script>

<div class="panel">
  {#if file?.text}
    <div class="where"><span class="path" title={file.path}>{file.path}</span><span class="size">{(file.bytes / 1024).toFixed(1)}k</span></div>
  {/if}
  <div class="body">
    {#if loading}
      <p class="note">reading…</p>
    {:else if html !== null}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      <div class="md">{@html html}</div>
    {:else if file?.error}
      <!-- Not an error state. A document recorded in the graph and absent
           from disk is history doing its job. -->
      <p class="note">{file.error}</p>
    {:else}
      <p class="note">no document for this node</p>
    {/if}
  </div>
</div>

<style>
  /* Lives in a tab of the right rail. It used to float over the stage and
     covered the graph it was opened from (goodlux, 2026-09-24: the document
     and the graph should both stay in view), so the rail now holds it and
     the stage keeps its whole surface. */
  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .where {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
    padding: 0.35rem 0.7rem;
    border-bottom: 1px solid var(--paper-tint);
    font-size: 10px;
    color: var(--ink-faint);
  }
  .path { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .size { font-variant-numeric: tabular-nums; }

  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0.7rem 0.9rem 1.2rem;
  }
  .note {
    color: var(--ink-faint);
    font-size: 11px;
    margin: 0;
  }

  /* Reads like a document, not like a data view: the display face for
     headings, generous measure, and code kept monospace. */
  .md {
    font-size: 12.5px;
    line-height: 1.62;
  }
  .md :global(h1),
  .md :global(h2),
  .md :global(h3),
  .md :global(h4) {
    font-family: var(--display);
    font-weight: normal;
    line-height: 1.25;
    margin: 1.1em 0 0.4em;
  }
  .md :global(h1) { font-size: 17px; }
  .md :global(h2) { font-size: 15px; }
  .md :global(h3) { font-size: 13.5px; }
  .md :global(h4) { font-size: 12.5px; }
  .md :global(p), .md :global(ul), .md :global(ol), .md :global(blockquote) {
    margin: 0 0 0.75em;
  }
  .md :global(ul), .md :global(ol) { padding-left: 1.2em; }
  .md :global(li) { margin-bottom: 0.2em; }
  .md :global(code) {
    font-family: var(--mono);
    font-size: 11px;
    background: var(--paper-tint);
    padding: 0.05em 0.3em;
  }
  .md :global(pre) {
    background: var(--paper-tint);
    border: 1px solid var(--rule);
    padding: 0.5rem 0.6rem;
    overflow-x: auto;
  }
  .md :global(pre code) { background: none; padding: 0; }
  .md :global(blockquote) {
    border-left: 2px solid var(--rule);
    padding-left: 0.7em;
    color: var(--ink-faint);
    margin-left: 0;
  }
  .md :global(a) { color: inherit; }
  .md :global(hr) { border: none; border-top: 1px solid var(--rule); margin: 1.2em 0; }
  .md :global(table) { border-collapse: collapse; font-size: 11px; }
  .md :global(th), .md :global(td) {
    border: 1px solid var(--rule);
    padding: 0.2rem 0.4rem;
    text-align: left;
  }
  .md :global(img) { max-width: 100%; }
</style>
