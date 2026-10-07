//! Adapter `VaultPort` (P6) — camada de arquivos do vault com a disciplina do
//! legado (`docs/rust-architecture.md` §VAULT / §INDEX):
//!
//! - **escrita atômica**: temp no mesmo dir → `sync_all` → `rename` (reduce
//!   replace-no-POSIX, MOVEFILE_REPLACE_EXISTING no Windows — std::fs::rename);
//! - **no-op if unchanged**: conteúdo idêntico não toca mtime (protege o
//!   embed-guard / index diff);
//! - **list** recursivo ignorando `.memoryos`, `.trash`, `Templates`;
//! - **path-safety**: caminhos relativos internos, sem `..`, sem absoluto;
//! - **stat** = (mtime_ms, size) para o fingerprint de re-embed.
//!
//! O watcher (`watch`, notify + debounce 250ms) é implementado no T5.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use second_brain_core::app::error::{AppError, Result};
use second_brain_core::app::ports::{VaultEvent, VaultPort, VaultStat, WatchHandle};

/// Diretórios obrigatórios do legado (`VaultAdapter.ts`).
const REQUIRED_DIRS: &[&str] = &[
    "Projects",
    "Sessions",
    "ADR",
    "Knowledge",
    "Research",
    "Studies",
    "Documentations",
    "Templates",
    "Archive",
    "Ideas",
    "Prompts",
    "Skills",
];

/// Diretórios ignorados na listagem (não são notas editáveis).
const IGNORED_DIRS: &[&str] = &[".memoryos", ".trash", "Templates"];

pub struct Vault {
    root: PathBuf,
}

impl Vault {
    pub fn new(root: &str) -> Self {
        Vault {
            root: PathBuf::from(root),
        }
    }

    fn resolve(&self, rel_path: &str) -> Result<PathBuf> {
        if rel_path.is_empty() {
            return Err(AppError::Vault("vault path vazio".into()));
        }
        let p = Path::new(rel_path);
        if p.is_absolute() {
            return Err(AppError::Vault(format!("vault path absoluto: {rel_path}")));
        }
        for comp in p.components() {
            match comp {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(AppError::Vault(format!("vault path inseguro: {rel_path}")))
                }
                Component::CurDir | Component::Normal(_) => {}
            }
        }
        Ok(self.root.join(p))
    }

    fn ensure_parent(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::Vault(format!(
                    "falha ao criar diretório {}: {e}",
                    parent.display()
                ))
            })?;
        }
        Ok(())
    }
}

impl VaultPort for Vault {
    fn path(&self) -> &str {
        self.root.to_str().unwrap_or_default()
    }

    fn ensure_structure(&mut self) -> Result<()> {
        fs::create_dir_all(&self.root).map_err(|e| {
            AppError::Vault(format!("falha ao criar vault {}: {e}", self.root.display()))
        })?;
        for dir in REQUIRED_DIRS {
            fs::create_dir_all(self.root.join(dir))
                .map_err(|e| AppError::Vault(format!("falha ao criar diretório {dir}: {e}")))?;
        }
        Ok(())
    }

    fn exists(&mut self, rel_path: &str) -> Result<bool> {
        Ok(self.resolve(rel_path)?.exists())
    }

    fn read_note(&mut self, rel_path: &str) -> Result<String> {
        let path = self.resolve(rel_path)?;
        fs::read_to_string(&path)
            .map_err(|e| AppError::Vault(format!("falha ao ler {}: {e}", path.display())))
    }

    fn write_note(&mut self, rel_path: &str, content: &str) -> Result<()> {
        let path = self.resolve(rel_path)?;
        // No-op if unchanged: mesmo conteúdo não toca mtime (embed-guard).
        if path.is_file() {
            match fs::read_to_string(&path) {
                Ok(existing) if existing == content => return Ok(()),
                Ok(_) => {}
                Err(_) => {
                    return Err(AppError::Vault(format!(
                        "vault corrompido (não é utf-8): {}",
                        path.display()
                    )))
                }
            }
        }
        self.ensure_parent(&path)?;
        let tmp = self.root.join(format!(
            ".{}.{}.tmp",
            rel_path.replace('/', "__"),
            std::process::id()
        ));
        {
            // Escrita + fsync no MESMO handle de escrita (W-1): um handle aberto
            // só para leitura (`File::open`) falha no `FlushFileBuffers` do
            // Windows (exige GENERIC_WRITE) — diferença crítica de plataforma
            // eliminada aqui, sem depender de API Unix.
            use std::io::Write;
            let mut file = fs::File::create(&tmp).map_err(|e| {
                AppError::Vault(format!("falha ao gravar temp {}: {e}", tmp.display()))
            })?;
            file.write_all(content.as_bytes()).map_err(|e| {
                AppError::Vault(format!("falha ao gravar temp {}: {e}", tmp.display()))
            })?;
            file.sync_all().map_err(|e| {
                AppError::Vault(format!("falha ao fsync temp {}: {e}", tmp.display()))
            })?;
            // drop fecha o handle antes do rename
        }
        fs::rename(&tmp, &path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            AppError::Vault(format!("falha ao substituir {}: {e}", path.display()))
        })?;
        // A-04 (G1): fsync do diretório após o rename torna durável a ENTRADA
        // do arquivo (não só o conteúdo) em crash de FS. Linux/POSIX: abrir o
        // diretório com `File::open` e chamar `sync_all` funciona e é válido.
        // Windows: a std não expõe `FILE_FLAG_BACKUP_SEMANTICS` → a abertura
        // falha e fazemos skip (rename em NTFS é journalizado). Diferença de
        // plataforma documentada — **não validado em execução Windows**.
        if let Some(parent) = path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                dir.sync_all().map_err(|e| {
                    AppError::Vault(format!(
                        "falha ao fsync diretório {}: {e}",
                        parent.display()
                    ))
                })?;
            }
        }
        Ok(())
    }

    fn stat(&mut self, rel_path: &str) -> Result<Option<VaultStat>> {
        let path = self.resolve(rel_path)?;
        let Ok(meta) = fs::metadata(&path) else {
            return Ok(None);
        };
        if !meta.is_file() {
            return Ok(None);
        }
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Ok(Some(VaultStat {
            mtime_ms,
            size: meta.len(),
        }))
    }

    fn delete_note(&mut self, rel_path: &str) -> Result<()> {
        let path = self.resolve(rel_path)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), // idempotente
            Err(e) => Err(AppError::Vault(format!(
                "falha ao apagar {}: {e}",
                path.display()
            ))),
        }
    }

    fn list_markdown_paths(&mut self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out)?;
        Ok(out)
    }

    fn watch(&mut self, callback: Box<dyn Fn(VaultEvent) + Send>) -> Result<Box<dyn WatchHandle>> {
        use notify::{RecommendedWatcher, RecursiveMode, Watcher};
        use std::time::Instant;

        let (notify_tx, notify_rx) = std::sync::mpsc::channel::<notify::Event>();
        let root = self.root.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                if let Ok(e) = res {
                    let _ = notify_tx.send(e);
                }
            },
            notify::Config::default(),
        )
        .map_err(|e| AppError::Vault(format!("falha ao criar watcher: {e}")))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| AppError::Vault(format!("falha ao observar {}: {e}", root.display())))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();

        std::thread::spawn(move || {
            // `watcher` movido para a closure: drop = para de observar. Vive
            // enquanto a thread viver (padrão notify de manter o watcher).
            let _watcher = watcher;
            let root = root;
            let mut pending: Vec<VaultEvent> = Vec::new();
            let mut due: Option<Instant> = None;
            // 250ms de janela de debounce (espelha `VaultWatcher.ts`).
            const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);
            loop {
                // 1) flushes primeiro: nunca perder um lote pendente no stop.
                if let Some(d) = due {
                    if d <= Instant::now() {
                        for ev in dedupe_events(std::mem::take(&mut pending)) {
                            callback(ev);
                        }
                        due = None;
                    }
                }
                if stop_flag.load(Ordering::SeqCst) {
                    // stop não engole o lote pendente.
                    for ev in dedupe_events(std::mem::take(&mut pending)) {
                        callback(ev);
                    }
                    break;
                }
                let timeout = match due {
                    Some(d) => d.saturating_duration_since(Instant::now()),
                    None => DEBOUNCE,
                };
                match notify_rx.recv_timeout(timeout) {
                    Ok(raw) => {
                        for ev in map_events(raw, &root) {
                            pending.push(ev);
                        }
                        if due.is_none() {
                            due = Some(Instant::now() + DEBOUNCE);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        Ok(Box::new(WatchHandleImpl { stop }))
    }
}

/// Handle concreto do watcher (T5): `stop()` cessa o loop em ≤ janela de poll.
struct WatchHandleImpl {
    stop: Arc<AtomicBool>,
}

impl WatchHandle for WatchHandleImpl {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Mapeia eventos `notify` para `VaultEvent` do core, filtrando:
/// - não-`.md` (temp `.NOME.PID.tmp` da escrita atômica, JSON/config, DBs);
/// - diretórios ignorados (`.memoryos`/`.trash`/`Templates` — o DB vive ali e
///   não pode triggar re-sync).
///
/// `Rename(from,to)` vira `Deleted(from)+Created(to)` — IDENTITY delete + insert.
fn map_events(raw: notify::Event, root: &std::path::Path) -> Vec<VaultEvent> {
    use notify::event::ModifyKind;
    use second_brain_core::app::ports::VaultEventType;
    let mut out = Vec::new();
    let mut push = |path: &std::path::Path, ty: VaultEventType| {
        if let Some(rel) = rel_md(path, root) {
            out.push(VaultEvent {
                event_type: ty,
                path: rel,
                old_path: None,
            });
        }
    };
    match &raw.kind {
        notify::EventKind::Create(_) => {
            for p in &raw.paths {
                push(p, VaultEventType::Created);
            }
        }
        notify::EventKind::Remove(_) => {
            for p in &raw.paths {
                push(p, VaultEventType::Deleted);
            }
        }
        notify::EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::From)) => {
            for p in &raw.paths {
                push(p, VaultEventType::Deleted);
            }
        }
        notify::EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::To)) => {
            for p in &raw.paths {
                push(p, VaultEventType::Created);
            }
        }
        // `RenameBoth` carrega [from, to] num único evento. O Linux entrega o
        // trio From/To/Both (os pares aqui são absorvidos como no-op pela
        // idempotência de Deleted/Created no Application); backends que só
        // entregam Both (ReadDirectoryChangesW do Windows) continuam gerando o
        // par semântico correto (Deleted + Created — IDENTITY).
        notify::EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Both)) => {
            if raw.paths.len() == 2 {
                push(&raw.paths[0], VaultEventType::Deleted);
                push(&raw.paths[1], VaultEventType::Created);
            } else {
                for p in &raw.paths {
                    push(p, VaultEventType::Modified);
                }
            }
        }
        notify::EventKind::Modify(kind)
            // Data/Any = edição de conteúdo (save/atômico lá fora). Duplicatas
            // (Create+Modify do próprio write atômico) são deduped na janela.
            if !matches!(kind, ModifyKind::Name(_)) => {
                for p in &raw.paths {
                    push(p, VaultEventType::Modified);
                }
            }
        _ => {}
    }
    out
}

/// `rel` (relativo ao vault root) se o caminho é um `.md` fora dos
/// diretórios ignorados. Caminho que escapa do root (não deveria ocorrer com
/// Recursive) é descartado por segurança.
fn rel_md(full: &std::path::Path, root: &std::path::Path) -> Option<String> {
    if full.extension().map(|e| e == "md").unwrap_or(false) {
        let rel = full.strip_prefix(root).ok()?;
        if IGNORED_DIRS.iter().any(|d| {
            rel.components()
                .any(|c| c.as_os_str().to_string_lossy() == *d)
        }) {
            return None;
        }
        Some(rel.to_string_lossy().replace('\\', "/"))
    } else {
        None
    }
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let entries = fs::read_dir(dir)
        .map_err(|e| AppError::Vault(format!("falha ao listar {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| AppError::Vault(format!("falha ao ler dir: {e}")))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if IGNORED_DIRS.iter().any(|d| *d == name) {
                continue;
            }
            walk(base, &path, out)?;
        } else if name.ends_with(".md") {
            let rel = path
                .strip_prefix(base)
                .map_err(|e| AppError::Vault(format!("erro de prefixo: {e}")))?
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

/// Tratamento de eventos agrupado por path em janela fixa (250ms) — T5.
/// Coalesce **repetições** (Create+Modify duplicados vêm do próprio write
/// atômico: temp/rename), mas preserva sequências distintas por path
/// (Create → Modify → Delete num único arquivo na mesma janela).
pub fn dedupe_events(events: Vec<VaultEvent>) -> Vec<VaultEvent> {
    let mut out: Vec<VaultEvent> = Vec::new();
    for e in events {
        let is_dup = out
            .last()
            .map(|prev| prev.path == e.path && prev.event_type == e.event_type)
            .unwrap_or(false);
        if !is_dup {
            out.push(e);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn ensure_structure_creates_required_dirs() {
        let tmp = vault_root();
        let root = tmp.path().join("vault");
        let mut v = Vault::new(root.to_str().unwrap());
        v.ensure_structure().unwrap();
        for dir in REQUIRED_DIRS {
            assert!(root.join(dir).is_dir(), "{dir} ausente");
        }
    }

    #[test]
    fn write_is_atomic_and_noop_on_same_content() {
        let tmp = vault_root();
        let mut v = Vault::new(tmp.path().to_str().unwrap());
        v.ensure_structure().unwrap();
        v.write_note("Projects/p.md", "# Oi\nmundo\n").unwrap();
        let before = v.stat("Projects/p.md").unwrap().unwrap();
        // Mesmo conteúdo: stat NÃO muda (mtime preservado).
        v.write_note("Projects/p.md", "# Oi\nmundo\n").unwrap();
        let after = v.stat("Projects/p.md").unwrap().unwrap();
        assert_eq!(before, after, "no-op deveria preservar stat");
        // Sem arquivos .tmp órfãos e conteúdo legível.
        assert_eq!(v.read_note("Projects/p.md").unwrap(), "# Oi\nmundo\n");
        assert!(fs::read_dir(tmp.path().join("Projects")).unwrap().count() == 1);
    }

    #[test]
    fn write_note_replaces_content_and_updates_stat() {
        let tmp = vault_root();
        let mut v = Vault::new(tmp.path().to_str().unwrap());
        v.write_note("a/b/c.md", "v1").unwrap();
        let s1 = v.stat("a/b/c.md").unwrap().unwrap();
        v.write_note("a/b/c.md", "v2 que muda").unwrap();
        let s2 = v.stat("a/b/c.md").unwrap().unwrap();
        assert!(s1 != s2);
        assert_eq!(v.read_note("a/b/c.md").unwrap(), "v2 que muda");
    }

    #[test]
    fn list_respects_ignores_and_walk() {
        let tmp = vault_root();
        let mut v = Vault::new(tmp.path().to_str().unwrap());
        v.write_note("Projects/a.md", "a").unwrap();
        v.write_note("Skills/sub/b.md", "b").unwrap();
        let _ = std::fs::create_dir_all(tmp.path().join("Templates").join("inner"));
        v.write_note("Templates/ptpl.md", "tpl").unwrap();
        let _ = std::fs::create_dir_all(tmp.path().join(".memoryos"));
        v.write_note(".memoryos/ignored.md", "x").unwrap();
        let _ = std::fs::create_dir_all(tmp.path().join(".trash"));
        v.write_note(".trash/del.md", "y").unwrap();
        let mut paths = v.list_markdown_paths().unwrap();
        paths.sort();
        // `Templates/inner` não existe → write criou? não; ignore no walk.
        assert_eq!(
            paths,
            vec!["Projects/a.md".to_string(), "Skills/sub/b.md".to_string()]
        );
    }

    #[test]
    fn delete_is_idempotent() {
        let tmp = vault_root();
        let mut v = Vault::new(tmp.path().to_str().unwrap());
        v.write_note("x.md", "c").unwrap();
        assert!(v.exists("x.md").unwrap());
        v.delete_note("x.md").unwrap();
        v.delete_note("x.md").unwrap(); // segunda vez OK
        assert!(!v.exists("x.md").unwrap());
        assert_eq!(v.stat("x.md").unwrap(), None);
    }

    #[test]
    fn path_safety_rejects_traversal_and_absolute() {
        let mut v = Vault::new("/tmp/nonexistent-vault");
        assert!(v.write_note("../escape.md", "x").is_err());
        assert!(v.write_note("/etc/passwd", "x").is_err());
        assert!(v.write_note("", "x").is_err());
        assert!(v.write_note("a/../../b.md", "x").is_err());
        assert!(v.read_note("../escape.md").is_err());
        assert!(v.delete_note("..").is_err());
    }

    #[test]
    fn subdir_write_creates_parents() {
        let tmp = vault_root();
        let mut v = Vault::new(tmp.path().to_str().unwrap());
        v.write_note("Research/sub/n.md", "c").unwrap();
        assert!(tmp.path().join("Research/sub/n.md").is_file());
        assert_eq!(v.read_note("Research/sub/n.md").unwrap(), "c");
    }

    #[test]
    fn dedupe_events_keeps_repeats_but_not_sequences() {
        let mk = |ty: second_brain_core::app::ports::VaultEventType, path: &str| VaultEvent {
            event_type: ty,
            path: path.into(),
            old_path: None,
        };
        let created = second_brain_core::app::ports::VaultEventType::Created;
        let modified = second_brain_core::app::ports::VaultEventType::Modified;
        let deleted = second_brain_core::app::ports::VaultEventType::Deleted;
        // repetições da mesma natureza → 1
        assert_eq!(
            dedupe_events(vec![
                mk(created.clone(), "a.md"),
                mk(created.clone(), "a.md"),
                mk(created.clone(), "a.md"),
            ])
            .len(),
            1
        );
        // sequência distinta no mesmo path → preserva os 3
        let seq = dedupe_events(vec![
            mk(created.clone(), "w.md"),
            mk(modified.clone(), "w.md"),
            mk(deleted.clone(), "w.md"),
        ]);
        assert_eq!(seq.len(), 3);
        assert_eq!(
            seq.iter().map(|e| e.event_type.clone()).collect::<Vec<_>>(),
            vec![created, modified, deleted]
        );
    }

    /// T5 — watch real (notify, tempdir): write atômico → Created(.md);
    /// edição em-place → Modified; delete → Deleted. Ignora non-`.md` (temp do
    /// write atômico) e `.memoryos` (onde vive o index.db). Tempo real de
    /// debounce (250ms) — pragmático, com margem generosa.
    #[test]
    fn watch_emits_md_events_debounced_and_ignores_db() {
        use second_brain_core::app::ports::VaultEventType;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;
        let tmp = vault_root();
        let mut v = Vault::new(tmp.path().to_str().unwrap());
        // Dirs criados ANTES do watch: race do inotify é com dir recém-criado
        // (recursive-add é assíncrono → evento para o filho pode se perder).
        std::fs::create_dir_all(tmp.path().join("Knowledge")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".memoryos")).unwrap();
        let got: Arc<Mutex<Vec<VaultEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let got2 = got.clone();
        let mut handle = v
            .watch(Box::new(move |e| got2.lock().unwrap().push(e)))
            .unwrap();

        // Barreira de registro: o backend inotify registra de forma assíncrona;
        // sonda até o primeiro evento atravessar (garante que os writes abaixo
        // não sejam perdidos numa janela de startup).
        v.write_note("Knowledge/p.md", "ready").unwrap();
        let ready = (0..40).any(|_| {
            if got
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.path == "Knowledge/p.md")
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
            false
        });
        assert!(
            ready,
            "watcher não registrou eventos em 2s; got={:?}",
            got.lock().unwrap().iter().collect::<Vec<_>>()
        );

        // Arquivo novo (write atômico = rename → Created).
        v.write_note("Knowledge/w.md", "v1").unwrap();
        // DB em `.memoryos` NÃO pode gerar evento (senão sync em loop).
        std::fs::write(
            tmp.path().join(".memoryos").join("index.db"),
            b"sqlite-bytes",
        )
        .unwrap();
        // Edição EM-PLACE (fora do nosso write atômico → Modify de conteúdo).
        std::fs::write(tmp.path().join("Knowledge").join("w.md"), "v2 externo").unwrap();
        // Delete.
        v.delete_note("Knowledge/w.md").unwrap();

        // deixa o debounce + latência inotify assentarem antes de parar
        std::thread::sleep(Duration::from_millis(1500));
        handle.stop();

        let events: Vec<VaultEvent> = got.lock().unwrap().iter().cloned().collect();
        let kinds: Vec<VaultEventType> = events.iter().map(|e| e.event_type.clone()).collect();
        assert!(
            kinds.contains(&VaultEventType::Created),
            "esperava Created, recebi {kinds:?}\n{events:?}"
        );
        assert!(
            kinds.contains(&VaultEventType::Modified),
            "esperava Modified, recebi {kinds:?}\n{events:?}"
        );
        assert!(
            kinds.contains(&VaultEventType::Deleted),
            "esperava Deleted, recebi {kinds:?}\n{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| e.path.contains("index.db") || e.path.contains(".memoryos")),
            "eventos do DB/perímetro não deveriam existir: {events:?}"
        );
        assert!(
            !events.iter().any(|e| e.path.ends_with(".tmp")),
            "temp da escrita atômica não é `.md` e deve ser filtrado"
        );
    }

    /// T5 — `starva`: reescrever o mesmo conteúdo (write atômico no-op) deve
    /// produzir **zero** eventos (`no-op if unchanged` ⇒ nada pra sincronizar).
    #[test]
    fn watch_silent_on_noop_write() {
        use std::sync::{Arc, Mutex};
        let tmp = vault_root();
        let mut v = Vault::new(tmp.path().to_str().unwrap());
        v.write_note("a.md", "imutável").unwrap();
        // deixa o primeiro lote assentar antes de observar o silêncio
        std::thread::sleep(std::time::Duration::from_millis(600));

        let got: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let got2 = got.clone();
        let mut handle = v
            .watch(Box::new(move |_e| {
                *got2.lock().unwrap() += 1;
            }))
            .unwrap();
        // mesmo conteúdo → write retorna sem tocar mtime (no-op)
        v.write_note("a.md", "imutável").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        handle.stop();
        assert_eq!(*got.lock().unwrap(), 0, "no-op não pode gerar eventos");
    }

    /// W-2 (mitigação): backends que entregam `RenameMode::Both` num evento só
    /// (Windows/ReadDirectoryChangesW) devem produzir o par Deleted+Created
    /// — mesmo sem o trio From/To/Both do inotify do Linux.
    #[test]
    fn rename_both_maps_to_deleted_plus_created() {
        use notify::event::{EventKind, ModifyKind, RenameMode};
        let ev = notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_some_path(Some(std::path::PathBuf::from("/vault/Knowledge/antigo.md")))
            .add_some_path(Some(std::path::PathBuf::from("/vault/Knowledge/novo.md")));
        let got = map_events(ev, std::path::Path::new("/vault"));
        assert_eq!(
            got.iter()
                .map(|e| (e.event_type.clone(), e.path.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (
                    second_brain_core::app::ports::VaultEventType::Deleted,
                    "Knowledge/antigo.md"
                ),
                (
                    second_brain_core::app::ports::VaultEventType::Created,
                    "Knowledge/novo.md"
                ),
            ]
        );
    }

    /// A-04: escrever via `write_note` executa o fsync do diretório após o
    /// rename (caminho exercitado sem erro em Linux); conteúdo integral.
    #[test]
    fn write_note_and_dir_fsync_probe() {
        let tmp = vault_root();
        let mut v = Vault::new(tmp.path().to_str().unwrap());
        v.write_note("dir/sub/a.md", "duravel").unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("dir/sub/a.md")).unwrap(),
            "duravel"
        );
    }
}
