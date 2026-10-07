# P6 — Vault fs (escrita) + integração sync

> **Agentes:** implementação nesta sessão (native); validação por gates (test/clippy/fmt) a cada task.
> Tasks usam checkbox `- [ ]`.

**Goal:** Gravar `.md` de forma atômica e preservando frontmatter (matriz), ligar create/update ao vault (hoje só tocam o banco), e dar embed-guard + watch ao sync — "fonte de verdade = vault" passa a valer de fato.

**Architecture:** Camada de escrita dividida em (a) editor **puro** `MarkdownEditor` (matriz FRONTMATTER: reuso raw, edição mínima linha-a-linha, recusa de FM inválido/avançado, sem criado FM em edição de conteúdo) e (b) adapter `Vault` na infra (escrita atômica temp→fsync→rename, no-op se inalterado, glob com ignores, watch 250ms via `notify`). O `Application` passa a escrever `.md` **antes** do commit SQLite (ordem G1/G2); `sync` indexa embeddings só quando conteúdo mudou (fingerprint `mtime+size` numa tabela `file_state`), evitando estourar a cota NVIDIA (~40 RPM).

**Tech Stack:** Rust, rusqlite (WAL, já), `fs4` (lock, já), `notify` (novo — watcher), std fs.

**Spec:** `docs/rust-architecture.md` §§VAULT, FRONTMATTER MATRIX, CONSISTENCY (G1/G2/G3), INDEX, IDENTITY; `docs/migration-log.md` P3/P4 "NÃO VALIDADO → P6" (replace atômico, watcher 250ms, `*.sync-conflict-*`).

## Global Constraints

- Uma única conexão SQLite; escrita no `.md` **antes** do commit dos derivados (consistência G1/G2).
- Nunca alterar o vault real em testes — fixtures/tempdir.
- `NoteId` canônico = path relativo sem `.md`; entrada com `.md` normaliza.
- FM: nunca "consertar" YAML inválido; edição de FM recusada em YAML inválido/avançado.
- Não reescrever arquivo se conteúdo não mudou.
- Gate = Linux (replace atômico via `rename`; watcher `notify`).

## Review Focus

- Path traversal: `rel_path` com `..`/absoluto → **Erro**, nunca escapa do vault.
- Escrita em edição de conteúdo de nota sem FM: **não cria** bloco FM (matriz "sem frontmatter").
- `updated` é auto-tocado (Note::update_*) — é edição de FM mínima (só essa linha muda), não reescrita total.
- Sync repetido (idempotência): 2× sync → mesmo conteúdo e **0 reembeds** quando nada mudou.
- Conflito externo: `.md` alterado fora do nosso fluxo desde o último DB → persistência aborta (não sobrescreve).

## Task 1: Editor (matriz FRONTMATTER) — puro

**Files:** Create `crates/second-brain-infra/src/markdown_editor.rs`; Modify `crates/second-brain-infra/src/lib.rs` (export).

**Interfaces**
- Consumes: `Frontmatter`/`FmValue` (core), `Note`.
- Produces:
  - `pub struct MarkdownEditor;`
  - `pub fn rewritten_body(original: &str, new_body: &str) -> String`
  - `pub fn fm_is_editable(yaml: &str) -> bool`
  - `pub fn apply_note(original: &str, note: &Note) -> Result<String>` (Err = recusa FM; conteúdo intacto nunca recusado)

^- [x] Step 1: escrever testes (casos da matriz, representativos das 16 linhas) — ver `markdown_editor.rs` final.
^- [x] Step 2: implementar; reuso raw + diff linha-a-linha de chaves (só changed/added/removed), estilo canônico na linha alterada.
^- [x] Step 3: gates + commit-friendly (sem commit nesta fase).
  > **Nota (divergência do plano):** o editor vive no **core** (`crates/second-brain-core/src/app/markdown_editor.rs`) — regra de produto usada pelo `Application`, e o core não pode importar infra (freeze arquitetural). Registrar no migration-log.

## Task 2: Adapter Vault (fs)

**Files:** Create `crates/second-brain-infra/src/vault.rs`; Modify `lib.rs`.

**Interfaces**
- Produces: `pub struct Vault { vault_path: PathBuf }` implementando `VaultPort`; `resolve(rel)->Result<PathBuf>` (safety); `write_atomic` interno.
- `ensure_structure`, `exists`, `read_note`, `write_note` (no-op se igual; temp→sync_all→rename; cleanup temp), `delete_note` (idempotente), `list_markdown_paths` (walk, ignores `.memoryos`/`.trash`/`Templates`), `stat` no port (fingerprint), `watch` (Task 5).

- [x] Step 1: testes (traversal, atomic, no-op, ignores, structure, list).
- [x] Step 2: implementar. Add `stat` e `VaultStat` ao `VaultPort` (core) + stub MemoryVault.

## Task 3: Application escreve `.md`

**Files:** Modify `crates/second-brain-core/src/app/application.rs`, `reindex`.

**Interfaces**
- `create_note`: `vault.write_note(path, &note.to_markdown())` antes do tx.
- `persist_note`: lê raw on-disk → conflito (parser(db)≠disk) aborta → `MarkdownEditor::apply_note` → `write_note` (no-op) → tx.
- `sync`/`reindex`: bora de `has_embedding` + fingerprint (Task 4).

- [x] Step 1: testes em core (MemoryVault) — create grava no vault; update preserva FM raw; sem FM não cria; conflito aborta.
- [x] Step 2: implementar.

## Task 4: Fingerprint + embed-guard no sync

**Files:** Modify `crates/second-brain-core/src/app/ports.rs` (`StorePort` + `SearchPort` + `FileState`/`VaultStat`/`FileState`), `stubs.rs`, `crates/second-brain-infra/src/store.rs` (tabela `file_state`), `search.rs` (`index_skip_embeddings`), `application.rs::sync`.

**Interfaces**
- `SearchPort::index_skip_embeddings(note, skip_embeddings) -> Result<()` (FTS sempre; embed só se `!skip`).
- `StorePort::{put_file_state,get_file_state,delete_file_state}`.
- `Application::sync`: parse + put_note sempre; embedding só quando fingerprint mudou/missing; remove reconcilia file_state.

- [x] Step 1: testes (2× sync → 0 reembeds; mudança de conteúdo → reembed).
- [x] Step 2: implementar.

## Task 5: Watch + sync delta

**Files:** Create `crates/second-brain-infra/src/watch.rs` (notify + debounce 250ms + `WatchHandle`); add `notify` (Cargo); Modify `application.rs` (`sync_file`/`apply_vault_event`).

**Interfaces**
- `Vault::watch(callback) -> Box<dyn WatchHandle>`; mapeia rename→deleted+created (se `old_path` perder).
- `Application::apply_vault_event(path, kind)`.

- [x] Step 1: teste de integração pragmático (tempdir, write, callback com timeout).
- [x] Step 2: implementar.

## Self-review
- Traversal no Task 2 (Review Focus 1) ✓ teste.
- Sem FM não cria (RF2) ✓ Task 1/3.
- `updated` só linha-diff (RF3) ✓ Task 1 teste (edit content → raw FM menos `updated`).
- Idempotência sync (RF4) ✓ Task 4.
- Conflito externo (RF5) ✓ Task 3.
- Spec: VAULT §§escrita/leitura/watch/detecção ✓ T1–T5; MATRIX rows 1–16 cobertos em T1; CONSISTENCY ordem .md→commit ✓ T3; IDENTITY rename delete+insert ✓ T5.