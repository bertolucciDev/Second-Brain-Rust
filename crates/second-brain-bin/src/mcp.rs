//! `memory mcp` — servidor MCP por stdio (JSON-RPC sobre linhas), composição
//! do Application Real (mesma do CLI). Substitui o MCP legado TS — contrato de
//! tools/list preservado a partir da fixture capturada na P0.
//!
//! Implementado: as **16 tools** do contrato congelado (P7 completo).
//! `second_brain_exec` segue a política FREEZE 3 (F24): desabilitada por default
//! (`exec.enabled=false`), allowlist obrigatória, sem shell e sem `env` do chamador.

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
                Ok(payload) => {
                    let content = payload.into_content();
                    Ok(Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": content, "isError": false }
                    })))
                }
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

/// Payload de retorno de uma tool, mapeado para os blocos de conteúdo MCP.
/// Mantém paridade com o legado: `exec` = texto cru; `context` = 2 blocos de
/// texto; demais = um bloco JSON.
#[derive(Debug)]
enum ToolPayload {
    Json(Value),
    Text(String),
    TextBlocks(Vec<String>),
}

impl ToolPayload {
    fn into_content(self) -> Value {
        let texts = match self {
            ToolPayload::Json(v) => vec![v.to_string()],
            ToolPayload::Text(t) => vec![t],
            ToolPayload::TextBlocks(ts) => ts,
        };
        Value::Array(
            texts
                .into_iter()
                .map(|t| json!({"type": "text", "text": t}))
                .collect(),
        )
    }

    /// Representação única para os testes (mesma semântica do payload antigo).
    #[cfg(test)]
    fn into_value(self) -> Value {
        match self {
            ToolPayload::Json(v) => v,
            ToolPayload::Text(t) => Value::String(t),
            ToolPayload::TextBlocks(ts) => {
                Value::Array(ts.into_iter().map(Value::String).collect())
            }
        }
    }
}

/// Mapeia o arg `strategy` para o enum (None = default da config).
fn strategy_arg(args: &Value) -> Option<SearchStrategy> {
    match opt_s(args, "strategy").as_deref() {
        Some("keyword") => Some(SearchStrategy::Keyword),
        Some("semantic") => Some(SearchStrategy::Semantic),
        Some("hybrid") => Some(SearchStrategy::Hybrid),
        _ => None,
    }
}

/// Campos de nota exigidos pelo contrato do MCP legado (path/title/tags/links).
fn note_fields(app: &mut Application, id: &str) -> (String, String, Vec<String>, Vec<String>) {
    match app.read_note(id) {
        Ok(n) => (
            n.path().to_string(),
            n.title().to_string(),
            n.tags().iter().map(|t| t.value().to_string()).collect(),
            n.wiki_links()
                .iter()
                .map(|l| l.target().to_string())
                .collect(),
        ),
        Err(_) => (String::new(), String::new(), Vec::new(), Vec::new()),
    }
}

fn notes_ref(items: &[second_brain_core::app::contract::NoteRef]) -> Vec<Value> {
    items
        .iter()
        .map(|n| json!({"path": n.path, "title": n.title}))
        .collect()
}

fn call_tool(name: &str, args: &Value, app: &mut Application) -> Result<ToolPayload> {
    match name {
        "second_brain_search" => {
            let limit = opt_usize(args, "limit").unwrap_or(12) as u32;
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as u32;
            let page = offset / limit.max(1) + 1;
            let q = SearchQuery {
                query: s(args, "query"),
                tags: vec_str(args, "tags"),
                links: vec_str(args, "links"),
                project: opt_s(args, "project"),
                strategy: strategy_arg(args),
                page,
                page_size: limit.max(1),
            };
            let results = app.search(&q)?;
            let mut items = Vec::with_capacity(results.len());
            for r in &results {
                let (path, title, tags, links) = note_fields(app, &r.note_id);
                items.push(json!({
                    "id": r.note_id,
                    "path": path,
                    "title": title,
                    "score": r.score,
                    "matchedFields": r.matched_fields,
                    "snippet": r.snippet,
                    "tags": tags,
                    "links": links,
                }));
            }
            Ok(ToolPayload::Json(json!({
                "total": items.len(),
                "offset": offset,
                "limit": limit,
                "items": items,
            })))
        }
        "second_brain_read" => {
            let id = s_opt_most(args, &["id", "path"]);
            let note = app.read_note(&id)?;
            let backlinks = app
                .backlinks(note.path())
                .map(|b| {
                    b.backlinks
                        .iter()
                        .map(|n| {
                            json!({
                                "id": n.path.trim_end_matches(".md"),
                                "title": n.title,
                                "path": n.path,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(ToolPayload::Json(json!({
                "id": note.id().value(),
                "path": note.path(),
                "title": note.title(),
                "content": note.content(),
                "tags": note.tags().iter().map(|t| t.value()).collect::<Vec<_>>(),
                "links": note.wiki_links().iter().map(|l| l.target()).collect::<Vec<_>>(),
                "project": note.project_id().map(|p| p.value()),
                "backlinks": backlinks,
            })))
        }
        "second_brain_create" => {
            let note = app.create_note(second_brain_core::app::CreateNoteRequest {
                path: s(args, "path"),
                title: opt_s(args, "title"),
                content: opt_s(args, "content").unwrap_or_default(),
                tags: vec_str(args, "tags"),
                links: vec_str(args, "links"),
                project: opt_s(args, "project"),
            })?;
            Ok(ToolPayload::Json(json!({
                "created": true,
                "id": note.id().value(),
                "path": note.path(),
                "title": note.title(),
            })))
        }
        "second_brain_stats" => {
            let s = app.stats()?;
            Ok(ToolPayload::Json(json!({
                "totalNotes": s.total_notes,
                "uniqueTags": s.unique_tags,
                "notesWithProject": s.linked_to_project,
            })))
        }
        "second_brain_graph" => {
            let out = app.graph(opt_s(args, "path").as_deref())?;
            Ok(ToolPayload::Json(
                serde_json::to_value(&out).map_err(ser_err)?,
            ))
        }
        "second_brain_info" => Ok(ToolPayload::Json(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "vaultPath": app.config.vault_path,
            "dbPath": app.config.db_path,
            "searchStrategy": app.config.search_strategy,
            "maxContextDocuments": app.config.max_context_documents,
            "capabilities": [
                "search", "read", "create", "context", "similar", "backlinks",
                "stats", "graph", "info", "adr_create", "adr_list",
                "project_create", "project_list", "project_show", "project_link", "exec"
            ],
            "embeddingModel": second_brain_infra::embed::DEFAULT_NVIDIA_EMBED_MODEL,
            "embeddingDim": second_brain_infra::embed::DEFAULT_NVIDIA_EMBED_DIM,
            "engine": "rust",
        }))),
        "second_brain_adr_create" => {
            let adr = app.create_adr(AdrCreateRequest {
                title: s(args, "title"),
                context: s(args, "context"),
                problem: s(args, "problem"),
                solution: s(args, "solution"),
                alternatives: vec_str(args, "alternatives"),
                consequences: vec_str(args, "consequences"),
            })?;
            Ok(ToolPayload::Json(json!({
                "created": true,
                "id": adr.id().value(),
                "title": adr.title(),
                "status": "proposed",
            })))
        }
        "second_brain_similar" => {
            let limit = opt_usize(args, "limit").unwrap_or(10);
            let results = app.similar(&s(args, "query"), limit)?;
            let mut items = Vec::with_capacity(results.len());
            for r in &results {
                let (path, title, tags, _) = note_fields(app, &r.note_id);
                items.push(json!({
                    "id": r.note_id,
                    "path": path,
                    "title": title,
                    "score": r.score,
                    "snippet": r.snippet,
                    "tags": tags,
                }));
            }
            Ok(ToolPayload::Json(
                json!({"total": items.len(), "items": items}),
            ))
        }
        "second_brain_context" => {
            let query = s(args, "query");
            let max_docs = opt_usize(args, "maxDocuments")
                .unwrap_or(app.config.max_context_documents as usize);
            let include_content = args
                .get("includeContent")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let docs = app.context(&query, max_docs, include_content, strategy_arg(args))?;
            let context_text = if docs.is_empty() {
                "No relevant documents found.".to_string()
            } else {
                docs.iter()
                    .map(|d| {
                        let tags = if d.tags.is_empty() {
                            "none".to_string()
                        } else {
                            d.tags.join(", ")
                        };
                        format!(
                            "SOURCE: {}\nTITLE: {}\nTAGS: {}\nRELEVANCE: {}\n\n{}\n\n---",
                            d.source, d.title, tags, d.relevance, d.content
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            let meta = json!({
                "count": docs.len(),
                "sources": docs.iter().map(|d| d.source.clone()).collect::<Vec<_>>(),
            });
            Ok(ToolPayload::TextBlocks(vec![
                context_text,
                meta.to_string(),
            ]))
        }
        "second_brain_backlinks" => {
            let out = app.backlinks(&s(args, "path"))?;
            Ok(ToolPayload::Json(json!({
                "note": {"path": out.note.path, "title": out.note.title},
                "backlinks": notes_ref(&out.backlinks),
                "outgoingLinks": notes_ref(&out.outgoing),
                "total": out.total,
            })))
        }
        "second_brain_adr_list" => {
            let items = app.adr_list()?;
            Ok(ToolPayload::Json(
                json!({ "total": items.len(), "items": items }),
            ))
        }
        "second_brain_project_create" => {
            let project =
                app.create_project(&s(args, "name"), &s(args, "description"), None, None, None)?;
            Ok(ToolPayload::Json(json!({
                "created": true,
                "id": project.id().value(),
                "name": s(args, "name"),
            })))
        }
        "second_brain_project_list" => {
            let items = app.project_list()?;
            Ok(ToolPayload::Json(
                json!({ "total": items.len(), "items": items }),
            ))
        }
        "second_brain_project_show" => {
            let out = app.project_show(&s(args, "id"))?;
            Ok(ToolPayload::Json(
                serde_json::to_value(&out).map_err(ser_err)?,
            ))
        }
        "second_brain_project_link" => {
            let note = app.set_project(&s(args, "notePath"), &s(args, "projectId"))?;
            Ok(ToolPayload::Json(json!({
                "linked": true,
                "note": note.path(),
                "project": s(args, "projectId"),
            })))
        }
        "second_brain_exec" => {
            // Política FREEZE 3 (F24): sem `env` injetável pelo chamador.
            if args.get("env").map(|v| !v.is_null()).unwrap_or(false) {
                return Err(AppError::InvalidInput(
                    "exec: env do chamador não é permitido (política de segurança)".into(),
                ));
            }
            let out = app.exec(
                &s(args, "command"),
                args.get("timeout").and_then(Value::as_u64),
            )?;
            let stdout = out.stdout.trim();
            let stderr = out.stderr.trim();
            let mut parts = Vec::new();
            if !stdout.is_empty() {
                parts.push(format!("stdout:\n{stdout}"));
            }
            if !stderr.is_empty() {
                parts.push(format!("stderr:\n{stderr}"));
            }
            parts.push(format!("exit code: {}", out.exit_code));
            Ok(ToolPayload::Text(parts.join("\n\n")))
        }
        other => Err(AppError::InvalidInput(format!(
            "tool '{other}' ainda não implementada na falha de migração Rust (P7) — \
             disponível no legado TS enquanto durar a transição"
        ))),
    }
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
            project: None,
        })
        .unwrap();
        app.create_note(CreateNoteRequest {
            path: "Knowledge/c.md".into(),
            title: Some("Gamma".into()),
            content: "launch gamma protocol someday".into(),
            tags: vec!["adr".into(), "status-accepted".into()],
            links: vec![],
            project: None,
        })
        .unwrap();
        app.create_note(CreateNoteRequest {
            path: "Knowledge/a.md".into(),
            title: Some("Alpha engine".into()),
            content: "the quick brown fox leaps over the dog".into(),
            tags: vec!["design".into(), "adr".into(), "status-proposed".into()],
            links: vec!["b".into(), "c".into()],
            project: None,
        })
        .unwrap();
        app
    }

    fn call(name: &str, args: &Value, app: &mut Application) -> Value {
        call_tool(name, args, app).unwrap().into_value()
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
    fn context_returns_two_text_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let payload = call_tool(
            "second_brain_context",
            &json!({"query": "brown", "maxDocuments": 3, "includeContent": true}),
            &mut app,
        )
        .unwrap();
        let blocks = match payload {
            ToolPayload::TextBlocks(b) => b,
            other => panic!("context deve devolver 2 blocos de texto, got {other:?}"),
        };
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("SOURCE: Knowledge/a.md"));
        assert!(blocks[0].contains("Alpha engine"));
        let meta: Value = serde_json::from_str(&blocks[1]).unwrap();
        assert!(meta["count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn search_wraps_results_like_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let out = call(
            "second_brain_search",
            &json!({"query": "brown", "limit": 5}),
            &mut app,
        );
        assert_eq!(out["offset"], 0);
        assert_eq!(out["limit"], 5);
        assert!(out["total"].is_number());
        let items = out["items"].as_array().unwrap();
        assert!(!items.is_empty());
        let first = &items[0];
        assert!(first["id"].is_string());
        assert!(first["path"].is_string());
        assert!(first["title"].is_string());
        assert!(first["score"].is_number());
        assert!(first["matchedFields"].is_array());
        assert!(first["tags"].is_array());
        assert!(first["links"].is_array());
    }

    #[test]
    fn info_exposes_contract_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let out = call("second_brain_info", &json!({}), &mut app);
        for key in [
            "version",
            "vaultPath",
            "dbPath",
            "searchStrategy",
            "maxContextDocuments",
            "capabilities",
            "embeddingModel",
            "embeddingDim",
        ] {
            assert!(!out[key].is_null(), "info.{key} ausente: {out}");
        }
        assert_eq!(out["embeddingDim"], 2048);
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
        assert!(out["outgoingLinks"].as_array().unwrap().is_empty());
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

    #[test]
    fn create_with_project_arg_links_note() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let proj = call(
            "second_brain_project_create",
            &json!({"name": "Proj X", "description": "d"}),
            &mut app,
        );
        let pid = proj["id"].as_str().unwrap().to_string();

        // `project` no create deve vincular a nota (contrato do MCP legado).
        let out = call(
            "second_brain_create",
            &json!({"path": "Knowledge/w.md", "content": "c", "project": pid}),
            &mut app,
        );
        assert_eq!(out["path"], "Knowledge/w.md");
        let show = call("second_brain_project_show", &json!({"id": pid}), &mut app);
        assert!(show["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["path"] == "Knowledge/w.md"));
    }

    #[test]
    fn exec_disabled_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        let err = call_tool(
            "second_brain_exec",
            &json!({"command": "git --version"}),
            &mut app,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("exec desabilitado"), "got: {err}");
    }

    #[test]
    fn exec_rejects_program_outside_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        app.config.exec.enabled = true;
        app.config.exec.allowed = vec!["git".into()];
        let err = call_tool(
            "second_brain_exec",
            &json!({"command": "__definitely_not_allowed__ arg"}),
            &mut app,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("allowlist"), "got: {err}");
    }

    #[test]
    fn exec_rejects_caller_env() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        app.config.exec.enabled = true;
        app.config.exec.allowed = vec!["git".into()];
        let err = call_tool(
            "second_brain_exec",
            &json!({"command": "git --version", "env": {"SECRET": "x"}}),
            &mut app,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("env"), "got: {err}");
    }

    #[test]
    fn exec_runs_allowlisted_command_in_vault() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = setup_app(&dir);
        app.config.exec.enabled = true;
        app.config.exec.allowed = vec!["git".into()];
        let payload = call(
            "second_brain_exec",
            &json!({"command": "git --version"}),
            &mut app,
        );
        // Payload textual (formato legado), não JSON.
        let text = payload.as_str().expect("exec retorna texto cru");
        assert!(text.contains("git version"), "got: {text}");
        assert!(text.contains("exit code: 0"), "got: {text}");
    }
}
