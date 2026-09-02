import { sparql } from './api'

const NOW = 'https://repolex.ai/git-lex/NamedGraph/now'
const GL_FILE_ID = 'https://repolex.ai/ontology/git-lex/fileId'

export interface Triple {
  s: string
  p: string
  o: string
  /** Whether the object is a node in the graph or a plain value. */
  oIsUri: boolean
}

/**
 * Every triple that mentions this document, on BOTH planes.
 *
 * A document is a File and, usually, a Thing that speaks through it. Its
 * facts are split across the two: the frontmatter that bound to a class sits
 * on the Thing, the frontmatter that did not sits on the File, and the body
 * links sit on the File. Asking about only one plane is how a document looks
 * half-empty when it is not — which is the same mistake that made every soul
 * render as a hex string for eight months.
 *
 * Returned unfiltered and unsorted-by-opinion: this panel exists so the raw
 * rows can be checked, so it must not quietly decide which rows matter.
 */
export async function triplesFor(genesis: string, id: string): Promise<{
  subjects: string[]
  asSubject: Triple[]
  asObject: Triple[]
  /** Thing URI -> the File URI it speaks through.
   *
   *  Needed because the layout folds every document onto its FILE, so the
   *  stage is indexed by file URIs — while a declared reference like
   *  `gl:relatedToId` points at the THING. Without this map those rows render
   *  as dead text, and the declared references are exactly the links someone
   *  opened this panel to follow. Resolved per selection rather than shipped
   *  for every document, which would add roughly 440KB to a large soul's
   *  metadata to answer a question asked one node at a time. */
  twinOf: Map<string, string>
}> {
  const esc = (u: string) => `<${u}>`

  // The twin, in whichever direction it exists: this id may be the File (the
  // usual case, since the layout folds onto files) or the Thing.
  const twins = await sparql<{ other: string }>(
    genesis,
    `SELECT DISTINCT ?other WHERE {
       GRAPH <${NOW}> {
         { ?other <${GL_FILE_ID}> ${esc(id)} }
         UNION
         { ${esc(id)} <${GL_FILE_ID}> ?other }
       }
     }`,
  )
  const subjects = [id, ...twins.map((t) => t.other)]
  const values = subjects.map(esc).join(' ')

  const [out, inc] = await Promise.all([
    sparql<{ s: string; p: string; o: string; kind: string }>(
      genesis,
      `SELECT ?s ?p ?o (DATATYPE(?o) AS ?dt) WHERE {
         GRAPH <${NOW}> { VALUES ?s { ${values} } ?s ?p ?o }
       } ORDER BY ?p`,
    ),
    sparql<{ s: string; p: string; o: string }>(
      genesis,
      `SELECT ?s ?p ?o WHERE {
         GRAPH <${NOW}> { VALUES ?o { ${values} } ?s ?p ?o }
       } ORDER BY ?p`,
    ),
  ])

  const isUri = (v: string) => /^https?:\/\//.test(v)
  const asSubject = out.map((r) => ({ s: r.s, p: r.p, o: r.o, oIsUri: isUri(r.o) }))
  const asObject = inc.map((r) => ({ s: r.s, p: r.p, o: r.o, oIsUri: true }))

  // Any URI mentioned by these rows that might be a Thing: ask which file it
  // speaks through, in one query.
  const mentioned = new Set<string>()
  for (const t of asSubject) if (t.oIsUri) mentioned.add(t.o)
  for (const t of asObject) mentioned.add(t.s)
  const twinOf = new Map<string, string>()
  if (mentioned.size) {
    const vals = [...mentioned].map(esc).join(' ')
    try {
      const pairs = await sparql<{ thing: string; file: string }>(
        genesis,
        `SELECT ?thing ?file WHERE {
           GRAPH <${NOW}> { VALUES ?thing { ${vals} } ?thing <${GL_FILE_ID}> ?file }
         }`,
      )
      for (const p of pairs) twinOf.set(p.thing, p.file)
    } catch {
      /* a failed resolution costs a clickable link, not the panel */
    }
  }

  return { subjects, asSubject, asObject, twinOf }
}

export function shortUri(u: string): string {
  if (!/^https?:\/\//.test(u)) return u
  const tail = u.replace(/[/#]$/, '').split(/[/#]/).pop() ?? u
  return tail || u
}

/** A compact prefix for a predicate, so a column of them is scannable:
 *  `soul:journalId`, `gl:md/linksTo`, `rdf:type`. */
export function curie(u: string): string {
  const map: [string, string][] = [
    ['https://repolex.ai/ontology/git-lex/git2/', 'git2:'],
    ['https://repolex.ai/ontology/git-lex/md/', 'md:'],
    ['https://repolex.ai/ontology/git-lex/fm/', 'fm:'],
    ['https://repolex.ai/ontology/git-lex/', 'gl:'],
    ['https://repolex.ai/ontology/soul/', 'soul:'],
    ['https://repolex.ai/ontology/copia/', 'copia:'],
    ['https://repolex.ai/ontology/pool/', 'pool:'],
    ['https://repolex.ai/ontology/ravel/', 'ravel:'],
    ['http://www.w3.org/1999/02/22-rdf-syntax-ns#', 'rdf:'],
    ['http://www.w3.org/2000/01/rdf-schema#', 'rdfs:'],
    ['http://www.w3.org/2001/XMLSchema#', 'xsd:'],
  ]
  for (const [pre, short] of map) {
    if (u.startsWith(pre)) return short + u.slice(pre.length)
  }
  return u
}
