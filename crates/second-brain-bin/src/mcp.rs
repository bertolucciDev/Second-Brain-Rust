//! `memory mcp` — servidor MCP por stdio (JSON-RPC sobre linhas), composição
//! do Application Real (mesma do CLI). Substitui o MCP legado TS — contrato de
//! tools/list preservado a partir da fixture capturada na P0.
//!
//! Implementado nesta fase: search/read/create/stats/graph/info/adr_create/
//! similar/context/backlinks/adr_list/project_create/project_list/project_show/
//! project_link; falta apenas `second_brain_exec` (decisão explícita do usuário —
//! executa comando arbitrário) para fechar o contrato de 16 tools.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use second_brain_core::app::contract::SearchQuery;
use second_brain_core::app::error::{AppError, Result};
use second_brain_core::app::{AdrCreateRequest, Application, SearchStrategy};
use serde_json::{json, Value};

use crate::load_config;

// ---------------------------------------------------------------------------
// stdio loop
// ---------------------------------------------------------------------------

/// Sobe o servidor MCP: abre a aplicação (lock exclusivo do DB), roda o sync
/// de startup como o legado (indexOnStartup) e atende stdio line-delimited.
pub fn serve(cwd: &Path) -> Result<()> {
    let cfg = load_config(cwd)?;
    let index_on_startup = cfg.index_on_startup;
    let mut app = super::compose(cfg, cwd)?;
    if index_on_startup {
        // sync no startup para o índice ficar pronto antes do 1º call.
        let _ = app.sync();
    }

    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in BufReader::new(stdin.lock()).lines() {
        let line = line.map_err(|e| AppError::Other(format!("stdin: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let resp = handle_line(&line, &mut app)?;
        if let Some(frame) = resp {
            let text = frame.to_string();
            out.write_all(text.as_bytes())
                .and_then(|()| out.write_all(b"\n"))
                .and_then(|()| out.flush())
                .map_err(|e| AppError::Other(format!("stdout: {e}")))?;
        }
    }
    Ok(())
}

/// Um frame por linha; erros nunca detêm o servidor (JSON-RPC error frame).
pub(crate) fn handle_line(line: &str, app: &mut Application) -> Result<Option<Value>> {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32700, "message": format!("parse error: {e}")}
            })));
        }
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(json!({}));

    let is_notification = !req
        .as_object()
        .map(|o| o.contains_key("id"))
        .unwrap_or(false);

    match method {
        "initialize" => Ok(Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "second-brain", "version": env!("CARGO_PKG_VERSION")}
            }
        }))),
        "notifications/initialized" | "initialized" | "notifications/cancelled" => Ok(None),
        "ping" => Ok(Some(json!({"jsonrpc": "2.0", "id": id, "result": {}}))),
        "tools/list" => Ok(Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": tools_list_fixture()
        }))),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(name, &args, app) {
                Ok(payload) => Ok(Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": payload.to_string()}],
                        "isError": false
                    }
                }))),
                Err(e) => Ok(Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32602, "message": format!("{name}: {e}")}
                }))),
            }
        }
        other => {
            if is_notification {
                Ok(None)
            } else {
                Ok(Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("method not found: {other}")}
                })))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// tools/list (contrato congelado — fixture P0)
// ---------------------------------------------------------------------------

fn tools_list_fixture() -> Value {
    static RAW: &str = include_str!("../../../tests/fixtures/contracts/mcp-tools.json");
    let fixture: Value = serde_json::from_str(RAW).expect("fixture de tools inválida");
    fixture["result"].clone()
}

// ---------------------------------------------------------------------------
// tools/call
// ---------------------------------------------------------------------------

fn call_tool(name: &str, args: &Value, app: &mut Application) -> Result<Value> {
    match name {
        "second_brain_search" => {
            let q = SearchQuery {
                query: s(args, "query"),
                tags: vec_str(args, "tags"),
                links: vec_str(args, "links"),
                project: opt_s(args, "project"),
                strategy: match opt_s(args, "strategy").as_deref() {
                    Some("keyword") => Some(SearchStrategy::Keyword),
                    Some("semantic") => Some(SearchStrategy::Semantic),
                    Some("hybrid") => Some(SearchStrategy::Hybrid),
                    _ => None,
                },
                ..Default::default()
            };
            Ok(serde_json::to_value(app.search(&q)?).map_err(ser_err)?)
        }
        "second_brain_read" => {
            let id = s_opt_most(args, &["id", "path"]);
            let note = app.read_note(&id)?;
            Ok(note_to_json(&note))
        }
        "second_brain_create" => {
            let note = app.create_note(second_brain_core::app::CreateNoteRequest {
                path: s(args, "path"),
                title: opt_s(args, "title"),
                content: opt_s(args, "content").unwrap_or_default(),
                tags: vec_str(args, "tags"),
                links: vec_str(args, "links"),
            })?;
            Ok(note_to_json(&note))
        }
        "second_brain_stats" => Ok(serde_json::to_value(app.stats()?).map_err(ser_err)?),
        "second_brain_graph" => {
            let target = opt_s(args, "target");
            Ok(serde_json::to_value(app.graph(target.as_deref())?).map_err(ser_err)?)
        }
        "second_brain_info" => Ok(json!({
            "name": "second-brain",
            "version": env!("CARGO_PKG_VERSION"),
            "engine": "rust",
            "vaultPath": app.vault_path(),
        })),
        "second_brain_adr_create" => {
            let adr = app.create_adr(AdrCreateRequest {
                title: s(args, "title"),
                context: opt_s(args, "context").unwrap_or_default(),
                problem: opt_s(args, "problem").unwrap_or_default(),
                solution: opt_s(args, "solution").unwrap_or_default(),
                alternatives: vec_str(args, "alternatives"),
                consequences: vec_str(args, "consequences"),
            })?;
            Ok(json!({
                "id": adr.id().value(),
                "number": adr.number(),
                "title": adr.title(),
                "status": adr.status().as_str(),
            }))
        }
        "second_brain_similar" => {
            let limit = opt_usize(args, "limit").unwrap_or(10);
            let results = app.similar(&s(args, "query"), limit)?;
            let items = results
                .iter()
                .map(|r| {
                    let title = app
                        .read_note(&r.note_id)
                        .map(|n| n.title().to_string())
                        .unwrap_or_default();
                    json!({
                        "id": r.note_id,
                        "path": r.note_id,
                        "title": title,
                        "score": r.score,
                        "snippet": r.snippet,
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({ "total": items.len(), "items": items }))
        }
        "second_brain_context" => {
            let query = s(args, "query");
            let max_docs = opt_usize(args, "maxDocuments").unwrap_or(5);
            let include_content = args
                .get("includeContent")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let strategy = match opt_s(args, "strategy").as_deref() {
                Some("keyword") => Some(SearchStrategy::Keyword),
                Some("semantic") => Some(SearchStrategy::Semantic),
                Some("hybrid") => Some(SearchStrategy::Hybrid),
                _ => None,
            };
            let docs = app.context(&query, max_docs, include_content, strategy)?;
            let sources = docs.iter().map(|d| d.source.clone()).collect::<Vec<_>>();
            let context_text = docs
                .iter()
                .map(|d| {
                    format!(
                        "SOURCE: {}\nTITLE: {}\nTAGS: {}\nRELEVANCE: {}\n\n{}\n\n---",
                        d.source,
                        d.title,
                        d.tags.join(", "),
                        d.relevance,
                        d.content,
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            Ok(json!({
                "context": context_text,
                "count": docs.len(),
                "sources": sources,
            }))
        }
        "second_brain_backlinks" => {
            let target = s_opt_most(args, &["path", "id"]);
            let out = app.backlinks(&target)?;
            Ok(serde_json::to_value(&out).map_err(ser_err)?)
        }
        "second_brain_adr_list" => {
            let items = app.adr_list()?;
            Ok(json!({ "total": items.len(), "items": items }))
        }
        "second_brain_project_create" => {
            let project =
                app.create_project(&s(args, "name"), &s(args, "description"), None, None, None)?;
            Ok(json!({
                "created": true,
                "id": project.id().value(),
                "name": s(args, "name"),
            }))
        }
        "second_brain_project_list" => {
            let items = app.project_list()?;
            Ok(serde_json::to_value(&items)
                .map_err(ser_err)
                .map(|v| json!({ "total": items.len(), "items": v }))?)
        }
        "second_brain_project_show" => {
            let out = app.project_show(&s(args, "id"))?;
            Ok(serde_json::to_value(&out).map_err(ser_err)?)
        }
        "second_brain_project_link" => {
            let note = app.set_project(&s(args, "notePath"), &s(args, "projectId"))?;
            Ok(json!({
                "linked": true,
                "note": note.path(),
                "project": s(args, "projectId"),
            }))
        }
        other => Err(AppError::InvalidInput(format!(
            "tool '{other}' ainda não implementada na falha de migração Rust (P7) — \
             disponível no legado TS enquanto durar a transição"
        ))),
    }
}

fn note_to_json(note: &second_brain_core::domain::entities::Note) -> Value {
    json!({
        "id": note.id().value(),
        "path": note.path(),
        "title": note.title(),
        "content": note.content(),
        "tags": note.tags().iter().map(|t| t.value()).collect::<Vec<_>>(),
        "links": note.wiki_links().iter().map(|l| l.target()).collect::<Vec<_>>(),
    })
}

fn ser_err(e: serde_json::Error) -> AppError {
    AppError::Other(format!("serialize: {e}"))
}

fn s(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn opt_s(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn opt_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .map(|v| v as usize)
}

fn s_opt_most(args: &Value, keys: &[&str]) -> String {
    keys.iter().find_map(|k| opt_s(args, k)).unwrap_or_default()
}

fn vec_str(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use second_brain_core::app::CreateNoteRequest;

    /// Setup com infra real (SQLite real + parser real) em tempdir — nunca toca
    /// o vault do usuário. Sem chave NVIDIA → embedder Null (keyword fallback).
    fn setup_app(dir: &tempfile::TempDir) -> Application {
        let cfg_raw = json!({
            "vaultPath": dir.path().join("vault").to_string_lossy(),
            "dbPath": dir.path().join(".memoryos").join("index.db").to_string_lossy(),
            "indexOnStartup": false,
        });
        fs::write(dir.path().join("memory.config.json"), cfg_raw.to_string()).unwrap();
        let cfg = super::super::load_config(dir.path()).unwrap();
        let mut app = super::super::compose(cfg, dir.path()).unwrap();
        // seed físico (o sync de startup está desligado)
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join("Knowledge")).unwrap();
        // alvos ANTES de quem cita: resolve_target_id só resolve se a nota existe.
        app.create_note(CreateNoteRequest {
            path: "Knowledge/b.md".into(),
            title: Some("Beta".into()),
            content: "no match here, just filler words".into(),
            tags: vec!["backlog".into()],
            links: vec![],
        })
        .unwrap();
        app.create_note(CreateNoteRequest {
            path: "Knowledge/c.md".into(),
            title: Some("Gamma".into()),
            content: "launch gamma protocol someday".into(),
            tags: vec!["adr".into(), "status-accepted".into()],
            links: vec![],
        })
        .unwrap();
        app.create_note(CreateNoteRequest {
            path: "Knowledge/a.md".into(),
            title: Some("Alpha engine".into()),
            content: "the quick brown fox leaps over the dog".into(),
            tags: vec!["design".into(), "adr".into(), "status-proposed".into()],
            links: vec!["b".into(), "c".into()],
        })
        .unwrap();
        app
    }

    fn call(name: &str, args: &Value, app: &mut Application) -> Value {
        call_tool(name, args, app).unwrap()
    }

    #[test]
    fn similar_returns_ranked_items() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let out = call(
            "second_brain_similar",
            &json!({"query": "quick brown", "limit": 5}),
            &mut app,
        );
        let items = out["items"].as_array().unwrap();
        assert!(!items.is_empty(), "semântica/fallback deve retornar hits");
        assert!(items[0]["score"].is_number());
    }

    #[test]
    fn context_formats_docs_and_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let out = call(
            "second_brain_context",
            &json!({"query": "brown", "maxDocuments": 3, "includeContent": true}),
            &mut app,
        );
        assert!(out["count"].as_u64().unwrap() >= 1);
        let text = out["context"].as_str().unwrap();
        assert!(text.contains("SOURCE: Knowledge/a.md"));
        assert!(text.contains("Alpha engine"));
    }

    #[test]
    fn backlinks_distinguishes_in_and_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let out = call(
            "second_brain_backlinks",
            &json!({"path": "Knowledge/b"}),
            &mut app,
        );
        assert_eq!(out["note"]["title"], "Beta");
        let backlinks = out["backlinks"].as_array().unwrap();
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0]["path"], "Knowledge/a.md");
        assert!(out["outgoing"].as_array().unwrap().is_empty());
        assert_eq!(out["total"].as_u64().unwrap(), 1);
    }

    #[test]
    fn adr_list_extracts_status_from_tags() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let out = call("second_brain_adr_list", &json!({}), &mut app);
        assert_eq!(out["total"].as_u64().unwrap(), 2);
        let items = out["items"].as_array().unwrap();
        assert!(items
            .iter()
            .any(|i| i["status"] == "proposed" && i["title"] == "Alpha engine"));
        assert!(items
            .iter()
            .any(|i| i["status"] == "accepted" && i["title"] == "Gamma"));
    }

    #[test]
    fn project_create_list_show_and_link() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);

        let out = call(
            "second_brain_project_create",
            &json!({"name": "Alpha Proj", "description": "desc"}),
            &mut app,
        );
        assert_eq!(out["created"], true);
        let pid = out["id"].as_str().unwrap().to_string();

        let list = call("second_brain_project_list", &json!({}), &mut app);
        assert_eq!(list["total"].as_u64().unwrap(), 1);
        assert_eq!(list["items"][0]["name"], pid);
        assert_eq!(list["items"][0]["notes"].as_u64().unwrap(), 2);

        let show = call("second_brain_project_show", &json!({"id": pid}), &mut app);
        assert_eq!(show["projectId"], pid);
        assert_eq!(show["notes"].as_u64().unwrap(), 2);

        let link = call(
            "second_brain_project_link",
            &json!({"projectId": pid, "notePath": "Knowledge/b.md"}),
            &mut app,
        );
        assert_eq!(link["linked"], true);
        assert_eq!(link["project"], pid);
        let show2 = call("second_brain_project_show", &json!({"id": pid}), &mut app);
        assert_eq!(show2["notes"].as_u64().unwrap(), 3);
    }
}
