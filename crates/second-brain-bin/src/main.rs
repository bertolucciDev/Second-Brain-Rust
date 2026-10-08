//! `memory` — CLI do Second Brain (P7): wiring de `bin → Application → infra`.
//!
//! Único lugar onde core e infra se encontram (arquitetura congelada). Toda a
//! lógica fica em `run*` testáveis; `main` só traduz argv/exit code.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use second_brain_core::app::contract::SearchQuery;
use second_brain_core::app::error::{AppError, Result};
use second_brain_core::app::ports::{ConfigStore, EmbedPort, StorePort, VaultEvent};
use second_brain_core::app::{Application, Config, CreateNoteRequest, SearchStrategy};
use second_brain_infra::{
    default_vault_path, resolve_default_db_path, FsConfigStore, FtsSearch, MarkdownParser,
    NvidiaEmbed, Platform, ProcessRunner, SqliteStore, Vault,
};

mod mcp;

/// Embedder "offline": sem `NVIDIA_API_KEY`, `reindex` falha explicitamente
/// (degradação honesta — nunca embeddings sintéticos em produção) e a busca
/// degrada para keyword via `FtsSearch` (que recebe embedder=None).
struct NullEmbed;

impl EmbedPort for NullEmbed {
    fn embed_for(&mut self, _texts: &[String], _as_query: bool) -> Result<Vec<Vec<f32>>> {
        Err(AppError::Embed(
            "embedder indisponível (defina NVIDIA_API_KEY)".into(),
        ))
    }

    fn is_available(&mut self) -> bool {
        false
    }

    fn vector_size(&mut self) -> usize {
        0
    }
}

fn try_load_dotenv(cwd: &Path) {
    // Conveniência para 'subir e pronto': linhas KEY=VALUE do `.env` local
    // (gitignored) são exportadas APENAS se ausentes — env real sempre vence.
    let Ok(raw) = std::fs::read_to_string(cwd.join(".env")) else {
        return;
    };
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, value);
            }
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    try_load_dotenv(&cwd);
    match dispatch(&args, &cwd) {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "memory — Second Brain (Rust)

uso: memory <comando> [args]

comandos:
  init                                   cria estrutura do vault + Home.md
  create <path> [--title T] [--content C] [--tags a,b] [--links x,y]
  read <id>                              lê nota (com ou sem .md)
  search <query> [--strategy keyword|hybrid|semantic]
  sync                                   reindex completo a partir do vault
  watch                                  watcher contínuo (debounce 250ms)
  stats                                  estatísticas do índice
  doctor                                 checagens (db fora do vault, etc.)
  reindex [--force]                      regenera embeddings (requer NVIDIA_API_KEY)
";

/// Ponto de entrada testável: todos os comandos retornam o texto de saída.
/// `watch` fica fora (loop contínuo — ver `watch_command`).
fn run(args: &[String], cwd: &Path) -> Result<String> {
    let Some(cmd) = args.first() else {
        return Ok(USAGE.to_string());
    };
    match cmd.as_str() {
        "help" | "-h" | "--help" => Ok(USAGE.to_string()),
        "init" => {
            let mut app = build_app(cwd)?;
            let r = app.init()?;
            Ok(format!(
                "vault inicializado em {}\níndice: {}",
                r.vault_path, r.db_path
            ))
        }
        "create" => {
            let p = parse_args(args);
            let path = p
                .pos
                .first()
                .ok_or_else(|| AppError::InvalidInput("create requer <path>".into()))?
                .clone();
            let title = p.values.get("title").cloned();
            let content = p.values.get("content").cloned().unwrap_or_default();
            let tags = split_list(p.values.get("tags"));
            let links = split_list(p.values.get("links"));
            let mut app = build_app(cwd)?;
            let note = app.create_note(CreateNoteRequest {
                path: path.clone(),
                title,
                content,
                tags,
                links,
            })?;
            Ok(format!(
                "criada: {} (id {})",
                note.path(),
                note.id().value()
            ))
        }
        "read" => {
            let id = parse_args(args)
                .pos
                .first()
                .ok_or_else(|| AppError::InvalidInput("read requer <id>".into()))?
                .clone();
            let mut app = build_app(cwd)?;
            let note = app.read_note(&id)?;
            Ok(note.to_markdown())
        }
        "search" => {
            let p = parse_args(args);
            let query = p
                .pos
                .first()
                .ok_or_else(|| AppError::InvalidInput("search requer <query>".into()))?
                .clone();
            let strategy = p
                .values
                .get("strategy")
                .map(|s| parse_strategy(s))
                .transpose()?;
            let mut app = build_app(cwd)?;
            let results = app.search(&SearchQuery {
                query,
                strategy,
                ..Default::default()
            })?;
            if results.is_empty() {
                return Ok("(sem resultados)".to_string());
            }
            let mut out = String::new();
            for r in &results {
                let snippet = r.snippet.replace('\n', " ");
                out.push_str(&format!(
                    "{:.3}\t{}\t{}\n",
                    r.score,
                    r.note_id,
                    snippet.trim()
                ));
            }
            if results.iter().any(|r| r.degraded) {
                out.push_str("(degradado: fallback keyword)\n");
            }
            Ok(out.trim_end().to_string())
        }
        "sync" => {
            let mut app = build_app(cwd)?;
            let r = app.sync()?;
            let mut out = format!(
                "indexadas {}/{} notas em {}ms ({} removidas)",
                r.indexed, r.scanned, r.duration_ms, r.removed
            );
            for e in &r.errors {
                out.push_str(&format!("\naviso: {e}"));
            }
            Ok(out)
        }
        "stats" => {
            let mut app = build_app(cwd)?;
            let s = app.stats()?;
            let size = s
                .index_size_kb
                .map(|kb| format!("{kb:.1} KB"))
                .unwrap_or_else(|| "-".to_string());
            Ok(format!(
                "notas: {}\nvinculadas a projetos: {}\ntamanho do índice: {}",
                s.total_notes, s.linked_to_project, size
            ))
        }
        "doctor" => {
            let mut app = build_app(cwd)?;
            let findings = app.doctor()?;
            if findings.is_empty() {
                return Ok("nenhum achado".to_string());
            }
            Ok(findings
                .iter()
                .map(|f| format!("{:?}: {}", f.level, f.message))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "reindex" => {
            let force = parse_args(args).flags.contains("force");
            let mut app = build_app(cwd)?;
            let r = app.reindex(force)?;
            Ok(format!(
                "reindex: {} regeneradas, {} puladas",
                r.regenerated, r.skipped
            ))
        }
        "watch" => Err(AppError::InvalidInput(
            "watch é um processo contínuo; use o binário diretamente".into(),
        )),
        other => Err(AppError::InvalidInput(format!(
            "comando desconhecido: {other}\n{USAGE}"
        ))),
    }
}

/// `watch` contínuo: sync de startup + delta por evento. `max_events` existe
/// para testes (None em produção = loop infinito até sinal do SO).
fn watch_command(cwd: &Path, sink: &mut dyn FnMut(&str), max_events: Option<usize>) -> Result<()> {
    let cfg = load_config(cwd)?;
    if !cfg.watch {
        return Err(AppError::InvalidInput(
            "watch desabilitado pela config".into(),
        ));
    }
    let vault_path = cfg.vault_path.clone();
    let index_on_startup = cfg.index_on_startup;

    use second_brain_core::app::ports::VaultPort;

    // O Application é single-writer: possui um `Vault` próprio. O watcher usa
    // uma segunda instância do adapter no mesmo root (sem regra de negócio —
    // apenas fs/notify → VaultEvent).
    let (tx, rx) = mpsc::channel::<VaultEvent>();
    let mut watcher_vault = Vault::new(&vault_path);
    let mut handle = watcher_vault.watch(Box::new(move |ev| {
        let _ = tx.send(ev);
    }))?;

    let mut app = compose(cfg, cwd)?;
    if index_on_startup {
        let r = app.sync()?;
        sink(&format!(
            "sync inicial: {}/{} notas ({} removidas)",
            r.indexed, r.scanned, r.removed
        ));
    }
    sink(&format!("observando {vault_path} (debounce 250ms)"));

    let mut seen = 0usize;
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(ev) => {
                match app.apply_vault_event(&ev) {
                    Ok(()) => sink(&format!("{:?} {} ok", ev.event_type, ev.path)),
                    Err(e) => sink(&format!("{:?} {} erro: {e}", ev.event_type, ev.path)),
                }
                seen += 1;
                if let Some(m) = max_events {
                    if seen >= m {
                        handle.stop();
                        return Ok(());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                handle.stop();
                return Ok(());
            }
        }
    }
}

fn dispatch(args: &[String], cwd: &Path) -> Result<String> {
    if args.first().map(String::as_str) == Some("watch") {
        let mut sink = |line: &str| println!("{line}");
        watch_command(cwd, &mut sink, None)?;
        return Ok(String::new());
    }
    if args.first().map(String::as_str) == Some("mcp") {
        // stdout é o canal JSON-RPC do MCP; erros vão para stderr (sem print aqui).
        return mcp::serve(cwd).map(|()| String::new()).map_err(|e| {
            eprintln!("mcp: {e}");
            e
        });
    }
    run(args, cwd)
}

// ---------------------------------------------------------------------------
// composição (bin): config + adapters reais
// ---------------------------------------------------------------------------

fn load_config(cwd: &Path) -> Result<Config> {
    let mut store = FsConfigStore::new(cwd.join("memory.config.json"));
    let raw = store.load()?;
    let platform = if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Unix
    };
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let local_appdata = std::env::var("LOCALAPPDATA").ok();
    let defaults = Config {
        vault_path: default_vault_path(cwd.to_string_lossy().as_ref()),
        db_path: resolve_default_db_path(platform, xdg.as_deref(), local_appdata.as_deref()),
        ..Config::default()
    };
    let cfg = match raw.as_object() {
        Some(map) if !map.is_empty() => {
            Config::from_json(&raw.to_string())?.merge_defaults(&defaults)
        }
        _ => defaults,
    };
    cfg.validate()?;
    Ok(cfg)
}

/// Composição completa (mesmo desenho dos testes de integração da infra):
/// `SqliteStore` (com FileLock exclusivo) → conexão compartilhada → `FtsSearch`.
fn compose(cfg: Config, cwd: &Path) -> Result<Application> {
    let vault_path = cfg.vault_path.clone();
    let mut store = SqliteStore::new();
    store
        .open(&cfg.db_path)
        .map_err(|e| AppError::Store(format!("{e} (db: {})", cfg.db_path)))?;
    let conn = store
        .shared_connection()
        .ok_or_else(|| AppError::Store("conexão sqlite indisponível".into()))?;
    let search_embedder = NvidiaEmbed::from_env().map(|e| Box::new(e) as Box<dyn EmbedPort>);
    let search = FtsSearch::new(conn, search_embedder, cfg.search_strategy);
    let app_embed: Box<dyn EmbedPort> = match NvidiaEmbed::from_env() {
        Some(e) => Box::new(e),
        None => Box::new(NullEmbed),
    };
    Ok(Application::new(
        cfg,
        Box::new(Vault::new(&vault_path)),
        Box::new(store),
        Box::new(search),
        app_embed,
        Box::new(MarkdownParser),
        Box::new(FsConfigStore::new(cwd.join("memory.config.json"))),
        Box::new(ProcessRunner),
    ))
}

fn build_app(cwd: &Path) -> Result<Application> {
    let cfg = load_config(cwd)?;
    compose(cfg, cwd)
}

// ---------------------------------------------------------------------------
// parsing de argv (mínimo, sem deps)
// ---------------------------------------------------------------------------

struct Parsed {
    pos: Vec<String>,
    values: BTreeMap<String, String>,
    flags: BTreeSet<String>,
}

fn parse_args(args: &[String]) -> Parsed {
    let mut out = Parsed {
        pos: Vec::new(),
        values: BTreeMap::new(),
        flags: BTreeSet::new(),
    };
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.strip_prefix("--") {
            Some(key) => {
                if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                    out.values.insert(key.to_string(), args[i + 1].clone());
                    i += 2;
                } else {
                    out.flags.insert(key.to_string());
                    i += 1;
                }
            }
            None => {
                out.pos.push(arg.clone());
                i += 1;
            }
        }
    }
    out
}

fn split_list(raw: Option<&String>) -> Vec<String> {
    raw.map(|v| {
        v.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn parse_strategy(raw: &str) -> Result<SearchStrategy> {
    match raw {
        "keyword" => Ok(SearchStrategy::Keyword),
        "hybrid" => Ok(SearchStrategy::Hybrid),
        "semantic" => Ok(SearchStrategy::Semantic),
        other => Err(AppError::InvalidInput(format!(
            "strategy desconhecida: {other} (keyword|hybrid|semantic)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Cria um diretório de trabalho com `memory.config.json` apontando para
    /// vault+db em tempdir — nunca toca o vault real.
    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let db = dir.path().join(".memoryos").join("index.db");
        let cfg = serde_json::json!({
            "vaultPath": vault.to_string_lossy(),
            "dbPath": db.to_string_lossy(),
        });
        fs::write(dir.path().join("memory.config.json"), cfg.to_string()).unwrap();
        dir
    }

    #[test]
    fn init_create_read_search_stats_roundtrip() {
        let dir = setup();
        let cwd = dir.path();

        let out = run(&["init".into()], cwd).unwrap();
        assert!(out.contains("vault inicializado"));
        assert!(cwd.join("vault").join("Home.md").is_file());

        let out = run(
            &[
                "create".into(),
                "Knowledge/orange.md".into(),
                "--title".into(),
                "Orange".into(),
                "--content".into(),
                "alpha orange fruit".into(),
                "--tags".into(),
                "a,b".into(),
            ],
            cwd,
        )
        .unwrap();
        assert!(out.contains("Knowledge/orange.md"));
        let file = fs::read_to_string(cwd.join("vault/Knowledge/orange.md")).unwrap();
        assert!(file.starts_with("---\n"));
        assert!(file.contains("alpha orange fruit"));

        let out = run(&["read".into(), "Knowledge/orange".into()], cwd).unwrap();
        assert!(out.contains("alpha orange fruit"));

        let out = run(&["search".into(), "orange".into()], cwd).unwrap();
        // id canônico = path sem `.md` (IDENTITY): o FTS indexa pelo id.
        assert!(out.contains("Knowledge/orange"), "search: {out}");

        let out = run(&["stats".into()], cwd).unwrap();
        assert!(out.contains("notas: 1"));

        // sync: sem embedder (offline), embedded=false → FTS re-indexa (idempotente)
        // mas NUNCA chama embedding; file_state mantém a flag honesta.
        let out = run(&["sync".into()], cwd).unwrap();
        assert!(out.contains("indexadas"), "sync: {out}");
        assert!(!out.contains("aviso:"), "sync com erros: {out}");

        // doctor roda (db fora do vault → sem achado crítico).
        run(&["doctor".into()], cwd).unwrap();

        // reindex sem chave → erro honesto (NullEmbed), nunca embeddings falsos.
        let out = run(&["reindex".into()], cwd);
        assert!(out.is_err());
    }

    #[test]
    fn read_unknown_note_fails_clearly() {
        let dir = setup();
        let err = run(&["read".into(), "Knowledge/missing.md".into()], dir.path()).unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn watch_indexes_new_file_event() {
        let dir = setup();
        let cwd = dir.path();
        run(&["init".into()], cwd).unwrap();

        let vault_dir = cwd.join("vault");
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            fs::create_dir_all(vault_dir.join("Knowledge")).unwrap();
            fs::write(
                vault_dir.join("Knowledge").join("live.md"),
                "conteudo ao vivo",
            )
            .unwrap();
        });

        let mut lines: Vec<String> = Vec::new();
        watch_command(cwd, &mut |l| lines.push(l.to_string()), Some(1)).unwrap();
        writer.join().unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Created") && l.contains("live.md")),
            "linhas: {lines:?}"
        );
        // O delta do watch indexou no store: `read` resolve sem novo sync.
        let out = run(&["read".into(), "Knowledge/live.md".into()], cwd).unwrap();
        assert!(out.contains("conteudo ao vivo"));
    }

    #[test]
    fn dotenv_loads_new_vars_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "SEGUNDOM_TESTE_DOTENV=de_arquivo\n# comentario\nSEGUNDOM_EXISTENTE=do_env\n",
        )
        .unwrap();
        std::env::set_var("SEGUNDOM_EXISTENTE", "pre");
        try_load_dotenv(dir.path());
        assert_eq!(
            std::env::var("SEGUNDOM_TESTE_DOTENV").as_deref(),
            Ok("de_arquivo")
        );
        assert_eq!(
            std::env::var("SEGUNDOM_EXISTENTE").as_deref(),
            Ok("pre"),
            "env real deve vencer o .env"
        );
    }
}
