<script lang="ts">
  import { marked } from 'marked'
  import type { FileText } from './types'

  interface Props {
    title: string
    file: FileText | null
    loading: boolean
    onclose: () => void
  }
  let { title, file, loading, onclose }: Props = $props()

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

<aside class="panel">
  <header>
    <h3 title={title}>{title}</h3>
    {#if file?.text}<span class="size">{(file.bytes / 1024).toFixed(1)}k</span>{/if}
    <button class="close" onclick={onclose} aria-label="Close">×</button>
  </header>
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
</aside>

<style>
  /* Floats over the stage rather than living in a rail, matching the old
     viewer's markdown panel — the graph stays the whole surface and the
     document is something you open on top of it and dismiss. Left-anchored
     so it never covers the inspector on the right. */
  .panel {
    position: absolute;
    left: 0.75rem;
    top: 2.6rem;
    bottom: 0.75rem;
    width: min(30rem, 42%);
    display: flex;
    flex-direction: column;
    background: var(--paper);
    border: 1px solid var(--ink);
    box-shadow: 3px 3px 0 rgba(0, 0, 0, 0.09);
    z-index: 5;
  }

  header {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    padding: 0.4rem 0.5rem;
    border-bottom: 1px solid var(--rule);
    background: var(--paper-tint);
  }
  h3 {
    margin: 0;
    font-family: var(--display);
    font-size: 13px;
    font-weight: normal;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size {
    font-size: 10px;
    color: var(--ink-faint);
    font-variant-numeric: tabular-nums;
  }
  .close {
    border: none;
    background: none;
    font-size: 16px;
    line-height: 1;
    padding: 0 0.15rem;
    cursor: pointer;
    color: var(--ink-faint);
  }
  .close:hover {
    color: var(--ink);
  }

  .body {
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
