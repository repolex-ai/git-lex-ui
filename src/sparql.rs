//! Talking to a repo's `git-lex-serve sparql` endpoint.
//!
//! This is the data feed. The other server — `git-lex-serve viz` — is the old
//! viewer's own backend: it serves that viewer's HTML and script, and it opens
//! a browser tab pointing at them every time it starts. Feeding a new
//! interface from it meant every soul opened here also opened the old page,
//! and it took a sandbox profile to stop something that should never have been
//! in the path at all.
//!
//! `sparql` is the endpoint with no interface attached. It binds the port it
//! is given or exits, it has a real `/health`, an `/info` that names the repo
//! it is serving, and it answers the W3C SPARQL protocol. Nothing about it
//! wants to be looked at.
//!
//! Results come back in the standard SPARQL JSON shape, which is a term
//! object per binding. Everything here flattens that to a plain string map so
//! the row structs stay simple — the one thing worth keeping from the term is
//! whether it was an IRI, and the callers that care recover that from the
//! value itself.

use serde::de::DeserializeOwned;

pub struct SparqlClient {
    http: reqwest::Client,
    base: String,
}

impl SparqlClient {
    pub fn new(http: reqwest::Client, port: u16) -> Self {
        Self { http, base: format!("http://127.0.0.1:{port}") }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Liveness only. Says the process is up and its store opened; says
    /// nothing about WHICH repo, which is why it is never used alone.
    pub async fn health(&self) -> Result<bool, String> {
        let r = self
            .http
            .get(format!("{}/health", self.base))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
        Ok(v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false))
    }

    /// The repo this endpoint is serving, as it reports itself.
    pub async fn info(&self) -> Result<EndpointInfo, String> {
        let r = self
            .http
            .get(format!("{}/info", self.base))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        r.json().await.map_err(|e| e.to_string())
    }

    /// Run a query and flatten the bindings into plain rows.
    pub async fn query<T: DeserializeOwned>(&self, q: &str) -> Result<Vec<T>, String> {
        let r = self
            .http
            .post(format!("{}/sparql", self.base))
            .header("content-type", "application/sparql-query")
            .header("accept", "application/sparql-results+json")
            .body(q.to_string())
            .send()
            .await
            .map_err(|e| format!("query failed: {e}"))?;
        if !r.status().is_success() {
            let code = r.status();
            let body = r.text().await.unwrap_or_default();
            return Err(format!("query rejected ({code}): {}", body.trim()));
        }
        let v: serde_json::Value = r.json().await.map_err(|e| format!("bad results: {e}"))?;
        let rows = flatten(&v)?;
        serde_json::from_value(rows).map_err(|e| format!("unexpected result shape: {e}"))
    }

}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct EndpointInfo {
    pub root: String,
    #[serde(default)]
    pub kit: Option<String>,
    #[serde(default)]
    pub optional_kits: Vec<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// `{"results":{"bindings":[{"n":{"type":"literal","value":"W3BL0RD"}}]}}`
/// becomes `[{"n":"W3BL0RD"}]`.
///
/// An unbound variable is simply absent from its binding, which is why every
/// row struct treats its optional columns as `Option`.
/// Flatten a W3C SPARQL results envelope into plain `{var: string}` rows.
///
/// Returns `Err` when the body is not a results envelope at all, and that
/// distinction is the whole point of the function's signature.
///
/// It used to return an empty array for anything it did not recognise. That
/// makes a 404 page, an error body, and a genuinely empty result set into
/// **the same value** — and @w3bl0rd-web nearly published a finding of mine
/// as confirmed on exactly that: they queried `/query` instead of `/sparql`,
/// got a 404, and read the empty body as "the store does not have this",
/// which happened to be the answer they expected. A wrong path and a true
/// negative are byte-identical downstream unless something refuses to
/// conflate them here.
///
/// An empty `bindings` array is still `Ok(vec![])`. "No rows matched" is a
/// real answer and must not be an error.
fn flatten(v: &serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(bindings) = v
        .get("results")
        .and_then(|r| r.get("bindings"))
        .and_then(|b| b.as_array())
    else {
        let head = v.to_string();
        let head = if head.len() > 200 { format!("{}\u{2026}", &head[..200]) } else { head };
        return Err(format!(
            "not a SPARQL results envelope (no results.bindings) — got: {head}"
        ));
    };
    Ok(serde_json::Value::Array(
        bindings
            .iter()
            .map(|b| {
                let mut row = serde_json::Map::new();
                if let Some(obj) = b.as_object() {
                    for (k, term) in obj {
                        if let Some(val) = term.get("value").and_then(|x| x.as_str()) {
                            row.insert(k.clone(), serde_json::Value::String(val.to_string()));
                        }
                    }
                }
                serde_json::Value::Object(row)
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_standard_results() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"head":{"vars":["n","o"]},"results":{"bindings":[
                 {"n":{"type":"literal","value":"W3BL0RD"},
                  "o":{"type":"uri","value":"https://example/x"}},
                 {"n":{"type":"literal","value":"only n"}}
               ]}}"#,
        )
        .unwrap();
        let rows = flatten(&v).expect("a well-formed envelope must flatten");
        assert_eq!(rows[0]["n"], "W3BL0RD");
        assert_eq!(rows[0]["o"], "https://example/x");
        // An unbound variable is absent, not null — callers must treat it as
        // optional rather than expecting a placeholder.
        assert!(rows[1].get("o").is_none());
    }

    #[test]
    fn empty_results_are_an_empty_list_not_an_error() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"head":{"vars":["n"]},"results":{"bindings":[]}}"#).unwrap();
        assert_eq!(
            flatten(&v).expect("an empty result set is a real answer, not an error"),
            serde_json::Value::Array(vec![])
        );
    }

    /// The trap this signature exists to close.
    ///
    /// @w3bl0rd-web hit `/query` instead of `/sparql` while independently
    /// reproducing a finding of mine, got a 404, and read the empty body as
    /// "the store does not contain this" — which was the answer they were
    /// expecting, so it looked like a confirmation. A wrong path and a true
    /// negative are the same value downstream unless something refuses to
    /// conflate them, and this is that something.
    ///
    /// Every case below returned `[]` before the change. Only the last one
    /// is allowed to now.
    #[test]
    fn a_body_that_is_not_a_results_envelope_is_an_error_not_an_empty_answer() {
        let not_envelopes = [
            r#"{"error":"not found"}"#,                       // an error body
            r#"{"head":{"vars":["n"]}}"#,                     // head, no results
            r#"{"results":{}}"#,                              // results, no bindings
            r#"{"results":{"bindings":{"n":"oops"}}}"#,       // bindings not a list
            r#"{}"#,                                          // empty object
            r#"[]"#,                                          // a bare array
        ];
        for body in not_envelopes {
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            assert!(
                flatten(&v).is_err(),
                "{body} must not read as an empty result set"
            );
        }

        // ...and the one shape that genuinely means "nothing matched".
        let real_empty: serde_json::Value =
            serde_json::from_str(r#"{"results":{"bindings":[]}}"#).unwrap();
        assert!(flatten(&real_empty).is_ok());
    }
}
