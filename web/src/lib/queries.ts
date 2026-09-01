/** SPARQL the overview runs. Kept here, named, and commented, so the
 *  reasoning travels with the query rather than living in a design doc
 *  nobody opens. */

export const NOW = 'https://repolex.ai/git-lex/NamedGraph/now'
export const GL = 'https://repolex.ai/ontology/git-lex/'
export const FM_TITLE = 'https://repolex.ai/ontology/git-lex/fm/title'
export const GL_NAME = `${GL}name`
export const GL_FILE_ID = `${GL}fileId`
export const MD_LINKS_TO = `${GL}md/linksTo`

export const classCounts = `
  SELECT ?t (COUNT(?s) AS ?n) WHERE {
    GRAPH <${NOW}> { ?s a ?t }
  } GROUP BY ?t ORDER BY DESC(?n)`

/**
 * Titles, resolved across BOTH planes.
 *
 * This is commit 2ebd895 carried forward. An unqualified frontmatter key
 * (`title:` rather than `soul.Note.title:`) cannot bind to a class property,
 * so git-lex emits it under the `fm/` fallback namespace attached to the
 * FILE subject — never to the Thing. A query asking `?thing fm:title ?label`
 * therefore matches nothing, every time, for every kit class, while the
 * answer sits one hop away on the file the Thing already points at.
 *
 * Measured on W3BL0RD: 33 fm:title against 4 gl:title, and gl:name on
 * exactly one subject in the whole store — roughly 89% of the titles that
 * exist were unreachable from the plane the query ran on. That is why souls
 * have been rendering as identifiers for eight months.
 *
 * The third UNION branch is the fix, and it is a branch rather than a
 * replacement so a real Thing-plane title still wins where one exists.
 */
export function titlesFor(classUri: string, limit = 6): string {
  return `
  SELECT DISTINCT ?s ?label WHERE {
    GRAPH <${NOW}> {
      ?s a <${classUri}> .
      {
        ?s <${GL_NAME}> ?label
      } UNION {
        ?s <${GL}title> ?label
      } UNION {
        ?s <${GL_FILE_ID}> ?f .
        ?f <${FM_TITLE}> ?label
      }
    }
  } LIMIT ${limit}`
}

/** Body links between files — what people actually wrote. */
export const fileLinkCount = `
  SELECT (COUNT(*) AS ?n) WHERE {
    GRAPH <${NOW}> { ?a <${MD_LINKS_TO}> ?b }
  }`

/** Body links whose target is a file that exists in the store. The rest are
 *  dangling — history, not errors: a link to a deleted document is often the
 *  only surviving evidence the target ever existed. Counted, never hidden. */
export const fileLinkResolved = `
  SELECT (COUNT(*) AS ?n) WHERE {
    GRAPH <${NOW}> {
      ?a <${MD_LINKS_TO}> ?b .
      ?b a <${GL}File> .
    }
  }`

/** The bridge. Every Thing that expresses itself through a file. */
export const bridged = `
  SELECT (COUNT(*) AS ?n) WHERE {
    GRAPH <${NOW}> { ?t <${GL_FILE_ID}> ?f }
  }`

export const thingCount = `
  SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE {
    GRAPH <${NOW}> { ?s a ?t . FILTER(?t != <${GL}File>) }
  }`

/** Declared references that are genuinely edges — the object is a node, not
 *  a string, and it is a *different* node.
 *
 *  Two distinctions, and both were paid for.
 *
 *  The object must be a node: `relatedTo "LSPy"` names something without
 *  linking to it, and counting it as connectivity is how a soul reads as
 *  connected when nothing in it actually joins up.
 *
 *  The object must not be the subject. Written without `?a != ?b`, this
 *  query returned **87** on W3BL0RD — and 79 of those were `gl:id`, which
 *  every Thing carries pointing at itself. A self-reference is not a
 *  connection, and a loose join counts it as one: the caption would have
 *  read "87 declared references" against a true figure of 8, an eleven-fold
 *  overstatement with a plausible-looking number vouching for it. Excluded
 *  by shape rather than by blacklisting `gl:id`, so the next
 *  identity-shaped predicate does not reintroduce the same lie. */
export const thingEdges = `
  SELECT (COUNT(*) AS ?n) WHERE {
    GRAPH <${NOW}> {
      ?a ?p ?b .
      ?a a ?ta . ?b a ?tb .
      FILTER(?a != ?b)
      FILTER(?p != <http://www.w3.org/1999/02/22-rdf-syntax-ns#type>)
      FILTER(?p != <${GL_FILE_ID}>)
      FILTER(?ta != <${GL}File>)
      FILTER(?tb != <${GL}File>)
    }
  }`

/** Self-referential triples between Things — a node naming itself. Surfaced
 *  rather than merely excluded, because "79 of this soul's 87 apparent
 *  connections are nodes pointing at themselves" is worth knowing, and a
 *  number that is only ever filtered out silently is a number nobody can
 *  check. */
export const thingSelfRefs = `
  SELECT (COUNT(*) AS ?n) WHERE {
    GRAPH <${NOW}> {
      ?a ?p ?a .
      ?a a ?ta .
      FILTER(?p != <http://www.w3.org/1999/02/22-rdf-syntax-ns#type>)
      FILTER(?ta != <${GL}File>)
    }
  }`

/** Declared references whose object is a literal — named, but not linked. */
export const thingLiteralRefs = `
  SELECT ?p (COUNT(*) AS ?n) WHERE {
    GRAPH <${NOW}> {
      ?a a ?ta . ?a ?p ?o .
      FILTER(STRSTARTS(STR(?p), "${GL}"))
      FILTER(CONTAINS(LCASE(STR(?p)), "related"))
      FILTER(isLiteral(?o))
    }
  } GROUP BY ?p ORDER BY DESC(?n)`
