//! `memory mcp` — servidor MCP por stdio (JSON-RPC sobre linhas), composição
//! do Application Real (mesma do CLI). Substitui o MCP legado TS — contrato de
//! tools/list preservado a partir da fixture capturada na P0.
//!
//! Implementado nesta fase: search/read/create/stats/graph/info/adr_create;
//! o restante do contrato (16 tools) responde "not implemented" com erro
//! JSON-RPC claro até as próximas fases.

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
