# SECOND BRAIN — MIGRATION LOG

> Registro vivo das fases da migração TS→Rust. Cada fase termina com: código + testes + evidência + diff de comportamento + este registro.
> Convenção de divergências: `BUG | MELHORIA | COMPORTAMENTO LEGADO INDESEJADO | DIFERENÇA INTENCIONAL | DESCONHECIDO`, sempre com CHANGE ID da arquitetura (§ PARITY de `docs/rust-architecture.md`).

---

## P0 — Contratos congelados + Fixtures de caracterização

**Data:** 2026-10-06. **Autorização:** produto ("Pode iniciar"). **Nenhum código Rust.** **Nenhuma alteração ao código TS legado** (somente leitura).

### Entregáveis

| Artefato | Conteúdo | Proveniência |
|---|---|---|
| `tests/fixtures/contracts/mcp-initialize.json` | `serverInfo: second-brain 1.0.0`, `capabilities {tools,resources,prompts}`, protocol `2024-11-05` | spawn real do servidor TS (`tsx src/cli/cli.ts mcp`, cwd isolado em `/tmp`) |
| `tests/fixtures/contracts/mcp-tools.json` | **16 tools** com schema `inputSchema` completo | `tools/list` do servidor real |
| `tests/fixtures/contracts/mcp-resources.json` | `second-brain://notes\|stats\|config` | `resources/list` do servidor real |
| `tests/fixtures/contracts/mcp-prompts.json` | `context_summary`, `search_and_read` | `prompts/list` do servidor real |
| `tests/fixtures/contracts/mcp-unmatched.json` | `[]` (nenhuma mensagem sem handler) | idem |
| `tests/fixtures/contracts/cli-help.txt` | help completo do CLI (32 linhas, 19 comandos) | `second-brain --help` |
| `tests/fixtures/contracts/cli-version.txt` | `1.0.0` | `second-brain --version` |
| `tests/fixtures/notes/sample-full.{md,note.json,golden.md}` | nota completa (frontmatter rico, headings, `#tag` inline e em codeblock, `[[wiki|alias]]`, callouts NOTA/IMPORTANT), vista parseada + `Note.toMarkdown()` | parser + Note do legado executados (read-only) |
| `tests/fixtures/notes/sample-minimal.{md,note.json,golden.md}` | nota mínima | idem |

### Contratos congelados (âncoras de compatibilidade)

1. **Arquivo `.md`** — fonte de verdade; ID canônico = path relativo sem `.md`; frontmatter com chaves `title/tags/project/date/updated/aliases/links/+novas`; `Note.toMarkdown()` regrava frontmatter com campos não vazios entre aspas (baseline §4.1, §7).
2. **CLI** — árvore de 19 comandos e flags = `cli-help.txt` (golden). Exit codes: 0 sucesso / 1 erro.
3. **MCP** — `second-brain` 1.0.0; 16 tools/3 resources/2 prompts com schemas exatos; erros em `{error: msg}`; logs em stderr; sync inicial fire-and-forget.
4. **Esquema `.db`** — baseline §4.2 (meta v3, notes, link_edges, notes_fts, note_embeddings) — usado como referência, nunca como destino.

### Baseline de caracterização (evidência executada)

| Item | Resultado |
|---|---|
| `vitest run` parser (14) + knowledge-graph (11) | ✅ **25/25 passam** |
| `second-brain --help` | ✅ saída capturada (exit 0) |
| `second-brain --version` | ✅ `1.0.0` |
| MCP `tools/list`/`resources/list`/`prompts/list` | ✅ capturados em JSON (16/3/2) |

**Comportamentos CONFIRMADOS capturados nas fixtures de nota (paridade futura):**
- `#tag` **dentro de codeblock é capturado** como tag (baseline §18 — agora com fixture `tag-in-code`).
- `Note.create` a partir de conteúdo com frontmatter faz `toMarkdown()` gerar **frontmatter duplo** (o gerado + o original preservado no body) — fixture `sample-full.golden.md`.
- Tags/links do frontmatter original **não são mesclados** na reescrita do `toMarkdown` (tags passam a ser as do corpo: `memoryos, design, tag-in-code`).
- Alias de wiki-link preservado (`architecture|Architecture Overview`).

### CHANGE IDs (registrados na arquitetura FREEZE 3)

| ID | Decisão | Estado |
|---|---|---|
| C1 | Windows PRIMARY/OFFICIAL, Linux SECONDARY/OFFICIAL, macOS fora | congelado |
| C2 | `dbPath` default fora do vault | congelado |
| C3 | default busca **hybrid** unificado + **fold de acentos ativo** (correção do strip-ASCII `MemorySearchIndex.ts:355`) | congelado |
| C4 | ranking híbrido **50/50** sobre conteúdo completo | congelado |
| C5 | remoção de `obsidian plugin` (HTTP `:3100`) | congelado |
| C6 | backup **somente vault** (`.zip` nativo) | congelado |
| C7 | macOS removido dos critérios | congelado |
| C8 | pendências resolvidas no gate | congelado |
| C9 | formato do id ADR (4 vs 3 dígitos) — **pendente de produto** | aberto |
| C-exec | `exec`: tool mantida presente, desabilitada por default, allowlist (lista de comandos **pendente**) | aberto |

### NÃO VALIDADO nesta fase (persiste do baseline)

- `tests/unit/embedding.test.ts` — baixaria modelo HuggingFace (rede/size).
- `tests/integration/repository.test.ts` — escreve `tests/.tmp` e mascara bug de conexão única (baseline §14).
- Saída em execução dos 19 comandos CLI (só `--help`/`--version`).
- Tools MCP com efeito de escrita (executar em Ptmp somente quando P5/P8 exigirem).

### Próxima fase: P1 — Domínio (core puro) + P2 — Application/Ports

- Fixtures de nome/build de VO e entidades derivadas dos 25 testes unitários já passando.

---

## P1 — Domínio Rust (core puro, std-only)

**Data:** 2026-10-06. **Autorização:** produto ("Pode iniciar P1"). **Saída:** `core puro verde`.

### Escopo entregue (IMPLEMENTATION PHASES — domínio puro, arquitetura FREEZE 3)

`crates/second-brain-core` (workspace Rust, resolver 2; **zero dependências externas** — serde/thiserror ficam para P2 no Application):

| Módulo | Espelha | Conteúdo |
|---|---|---|
| `domain::time` | `shared/utils.ts` | `now_ms`, `civil_from_days` (Hinnant), `to_iso_utc`/`to_date_utc`/`to_time_utc` — reprodução do `Date.toISOString()` do legado |
| `domain::metadata` | `Metadata.ts` | `Metadata` + `create_metadata(source)` (v1) |
| `domain::frontmatter` | `Frontmatter.ts` | mapa **ordenado** `Vec<(String, FmValue)>` (ordem de `Object.entries`), typed accessors, `add/remove` com dedup, `to_yaml` (estilo bloco do ADR), `parse` (formato próprio) |
| `domain::vo` | `Tag/NoteId/ProjectId/SessionId/WikiLink.ts` | normalizações e validações exatas |
| `domain::entities::note` | `Note.ts` | `create/reconstruct`, mutators, `to_markdown` (unificação `{...fm, tags, links}`) |
| `domain::entities::adr` | `ADR.ts` | `AdrStatus`, ciclo `accept/reject/supersede`, `to_markdown` template exato |
| `domain::entities::project` | `Project.ts` | id slug+normalizado, gestão de notes |
| `domain::entities::session` | `Session.ts` | acumulação + `end()`, `to_markdown` determinístico |
| `domain::entities::knowledge_graph` | `KnowledgeGraph.ts` | BFS componentes, métricas (density/avgDegree/centrality) |

### Evidência de validação (executada após a alteração)

| Comando | Resultado |
|---|---|
| `cargo build --workspace` | ✅ sem warnings |
| `cargo test --workspace` | ✅ **46/46 passam** |
| `cargo clippy --workspace --all-targets` | ✅ 0 warnings/errors |
| `cargo fmt --all -- --check` | ✅ limpo |

- Testes portados do legado TS: knowledge-graph **(11 fixtures originais)**, note (criação/`add`/`remove`/dedup/`toMarkdown` golden), VO (Tag/NoteId/ProjectId/SessionId/WikiLink), ADR, Session, Project, Frontmatter, time (inclui ISO conhecido + leap day).
- Nota de paridade: `#tag-in-code` permanece fora do escopo do domínio (parser de markdown é P4); fixtures `sample-*.golden.md` permanecem **âncoras não consumidas** até P4.

### Divergências de comportamento registradas (paridade)

| # | Componente | Divergência | Classificação |
|---|---|---|---|
| D-P1-1 | `Session.to_markdown` | legado usa `toLocaleDateString`/`toLocaleTimeString` (locale-dependente); Rust emite **UTC determinístico** | DIFERENÇA INTENCIONAL (corrige comportamento instável; feita em P1, fora das CHANGE C3–C6) |
| D-P1-2 | `NoteId.generate` / `SessionId.generate` | legado usa `crypto.randomUUID()`; Rust std-only gera id pseudo-único compatível em formato/prefixo, **não bit-igual** | PARYTY N/A (formato preservado; unicidade do legado só ocorre em runtime) |
| D-P1-3 | `Frontmatter::parse` | incluído em P1 (método de VO); o parser de markdown completo permanece P4 — parse não é usado pelo core nesta fase | ESCOPO |
| D-P1-4 | `Note.remove_wiki_link` | não altera `frontmatter.links` (idêntico ao legado `Note.ts:158`) | PARITY (confirmado) |

### NÃO VALIDADO nesta fase

- Round-trip `Note → markdown → Note` (requer parser de markdown, P4).
- `Density/avgDegree` confirmados apenas por assert aproximado; ponto flutuante idêntico ao legado só garantível via golden fixo (P4).

### Próxima fase: P2 — Application + Ports (serde/thiserror) + Config validado.

---

## P2 — Application + Ports + Config validado

**Data:** 2026-10-06. **Autorização:** produto ("Pode iniciar a P2"). **Saída:** casos de uso com ports stubs.

### Escopo entregue

`crates/second-brain-core` ganhou a camada `app/` (**domain permanece std-only**; `serde`/`serde_json`/`thiserror` entram aqui, conforme CRATE STRUCTURE — core = Domain + Application + ports + config + contratos):

| Módulo | Conteúdo |
|---|---|
| `app::config` | `Config` `camelCase` idêntico ao legado (`vaultPath/dbPath/watch/indexOnStartup/autoReflect/maxContextDocuments/searchStrategy`) + campos **desconhecidos preservados** (flatten, não-destruição aceite #8) + **schema validado** (F23): caminhos não vazios, `maxContextDocuments >= 1`, estratégia enum; `merge_defaults` pureza (loader infra P3 preenche `vaultPath/dbPath`) |
| `app::contract` | tipos de resposta/saída: `SyncResult`, `ReindexResult`, `GraphOutput` (= `toJSON(targetPath?)`), `VaultStats`, `DoctorFinding/Level`, `SessionSummary`, `SearchQuery`/`SearchResult` (`note_id/score/matched_fields/snippet`), `InitResult` |
| `app::ports` | traits: `VaultPort` (fs atômico), `StorePort` (single-writer/transações), `SearchPort`, `EmbedPort`, `ParserPort`, `ConfigStore`, `CommandRunner` (exec, C-exec) + `CommandSpec/CommandOutput` |
| `app::stubs` | in-memory: `MemoryStore`, `MemoryVault`, `MemorySearch` (substring + filtros), `MemoryEmbed`, `StubParser` (título=basename), `MemoryConfigStore`, `StubRunner` |
| `app::application` | `Application` com casos de uso: `init`, `create_note`, `read_note` (aceite #7), `update_content`, tags/links add-remove, `set_project`, `search`, `sync`, `reindex`, `graph`, `stats`, `doctor`, `reflect`, `list_sessions`, `create_adr` (numeração MAX+1, aceite #6), `transition_adr`, `create_project`, `config_get/set`, `run_command` |

**Regra de escrita arquitetural:** todos os mutators passam por `store.begin/commit` + `search.index` + grafo; `cli`/`mcp` nunca conhecerão infra (correção da violação `server.ts:786` via port `CommandRunner`).

### Evidência de validação (executada após a alteração)

| Comando | Resultado |
|---|---|
| `cargo build --workspace` | ✅ sem warnings |
| `cargo test --workspace` | ✅ **65/65 passam** (46 domínio + 19 app) |
| `cargo clippy --workspace --all-targets` | ✅ 0 warnings/errors |
| `cargo fmt --all -- --check` | ✅ limpo |

Testes novos de app com stubs: round-trip create/read (aceite #7), título default do filename, add/remove tag persistido, sync (scan/index/remove), **ADR MAX+1 sem duplicar** (aceite #6), transição accept (tags `status-*` + `**Status:**`), graph (nós/arestas/centrality por link), reflect (nota de sessão + list), project (index+architecture), config get/set + chaves desconhecidas, search com filtro de tag.

### Divergências de comportamento registradas

| # | Componente | Divergência | Classificação |
|---|---|---|---|
| D-P2-1 | Ports | traits com `&mut self` (single-writer explícito); assinaturas a revisitar em P5/P7 se necessário | DIFERENÇA INTENCIONAL |
| D-P2-2 | `adr create` | **BUG do legado**: regex `^ADR-(\d+)` não casava id `ADR/ADR-0001.md` → nextNumber sempre 1 (sobrescrevia). Rust calcula MAX+1 sobre `ADR/ADR-\d{4}` | BUG LEGADO INDESEJADO → corrigido (aceite #6) |
| D-P2-3 | Config | default `searchStrategy` **keyword** (entry point CLI); legado MCP usava hybrid — troca para hybrid será em P5/P7 (C4) | DIFERENÇA INTENCIONAL (temporária) |
| D-P2-4 | `read` | id com/sem `.md` resolvem igual (aceite #7); legado persistia id **com** `.md` e `findById` inconsistente | MELHORIA |
| D-P2-5 | grafo | `persist_note` atualiza só o nó da nota (delete+put) e insert de arestas wiki; rename/orphans completo fica P6 | ESCOPO |
| D-P2-6 | `reflect` | usa `Session.end()` + `toMarkdown` UTC determinístico (reforça D-P1-1); tags `date-{id}` | DIFERENÇA INTENCIONAL (consistente com D-P1-1) |

### NÃO VALIDADO nesta fase

- `StubParser` substitui o parser real (infra P4): `sync` armazena **título=basename e conteúdo bruto**, sem tags/links/frontmatter reais — vocabulário de parse só será fiel em P4.
- `stats.indexSizeKb`/`doctor` de diretórios reais (infra P3).
- `config get/set` usa o JSON bruto do `ConfigStore`; o resolved `Config` em memória não é recarregado após `set` (comportamento CLI real em P7).
- `CommandRunner` só no use case; allowlist/gate do `exec` aguarda C-exec (decisão de produto).

### Próxima fase: P3 — Store sqlite (FTS5+WAL, transações, rebuild) + persistência/restart (I1)
+ default de `dbPath` fora do vault (Windows `%LOCALAPPDATA%` primário) + aviso doctor + lock cross-platform.
---

## P3 — Store sqlite (FTS5+WAL, transações, locks) + dbPath default fora do vault

**Data:** 2026-10-06. **Autorização:** produto ("pode continuar para P3"). **Saída:** shutdown/restart consistente (I1).

Nova crate `crates/second-brain-infra` (adapter do `StorePort`); core permanece puro (só ganhou `app::paths` + aviso no `doctor`).

### Escopo entregue

| Item | Descrição |
|---|---|
| `infra::store::SqliteStore` | `StorePort` em rusqlite **bundled-full** (FTS5 + WAL nativo). Schema espelha connection.ts v3 (notes, link_edges, notes_fts fts5 porter/unicode61, note_embeddings) **+ frontmatter/metadata** (round-trip fiel) **+ graph_nodes/graph_edges** (cache do grafo). Transações `BEGIN/COMMIT/ROLLBACK` explícitas; `foreign_keys=ON`, `synchronous=NORMAL`, `busy_timeout=5000`; `ON CONFLICT` idempotente nos upserts; `resolveTargetId` = LIKE `%target%` em id OU path (legado) |
| Safe-open | Banco não-nosso (sem schema_version / versão ≠1) → **erro claro** orientando delete+reindex (banco é derivado, C2) |
| `infra::lock` | Lock advisory `<db>.lock` via **fs4** (`flock`/`LockFileEx`) exclusivo **between processes**; guard RAII; teste real com processo-filho |
| `core::app::paths` | `path_is_inside` (normalização lexical `./..`, `\`/`/`) + aviso no `doctor` quando `dbPath` está dentro do vault (**aceite #15 / R-A10**) |
| `infra::config` | Defaults **C2**: Windows `%LOCALAPPDATA%\Second Brain\index.db` (primário), Unix `$XDG_DATA_HOME/second-brain/index.db` (fallback `~/.local/share/...`); `dbPath` explícito legado (dentro do vault) **respeitado** + warning |
| Persistência | Reopen de `Application` sobre o mesmo arquivo de db → estado consistente (teste `restart_is_consistent`: notas, grafo, leitura) |

### Evidência de validação (após a alteração)

| Comando | Resultado |
|---|---|
| `cargo build --workspace` | ✅ sem warnings |
| `cargo test --workspace` | ✅ **85/85 passam** (core 69 + infra 16) |
| `cargo clippy --workspace --all-targets` | ✅ 0 |
| `cargo fmt --all -- --check` | ✅ limpo |

Testes novos: abrir db em WAL + FTS5 presente + schema_version=1; round-trip put/get/delete (tags/links/frontmatter/metadata); link_edges com target_id resolvido; recusa de banco estranho/legado; **lock exclusivo entre processos** (filho segura, pai falha, libera e re-adquire); double-open in-process; grafo persistido com dedup de arestas; **I1 restart consistente**; application via store real (search stub); doctor warning dbPath dentro/fora do vault.

### Divergências de comportamento registradas

| # | Componente | Divergência | Classificação |
|---|---|---|---|
| D-P3-1 | Schema | Rust ≠ legado: **adiciona** `frontmatter`/`metadata` (round-trip) e `graph_nodes`/`graph_edges`; banco legado/estranho é **recusado** com erro (C2 "banco derivado, rebuild a partir do vault") | DIFERENÇA INTENCIONAL |
| D-P3-2 | Journal | `WAL + synchronous=NORMAL + busy_timeout=5000` (arquitetura) vs legado sql.js `journal OFF` (crash-semantica diferente; R-A10) | DIFERENÇA INTENCIONAL (arquitetura) |
| D-P3-3 | Lock | Exclusivo **cross-process** via `flock`/`LockFileEx`; `fs4::try_lock_exclusive` retorna `Ok(false)` quando bloqueado (não usaErr) | DIFERENÇA INTENCIONAL (G1-G3; melhoria) |
| D-P3-4 | id | Store persiste `id` bruto da `Note` (com `.md` quando o caller passou com extensão); normalização canônica é da Application (read_note aceite #7) | ESCOPO (espelha legado) |
| D-P3-5 | Config default | `dbPath` default **fora do vault** (C2) vs legado `cwd/vault/.memoryos/index.db`; valor explícito legado respeitado + warning | DIFERENÇA INTENCIONAL (FREEZE 3) |
| D-P3-6 | `doctor` | novo finding `Warning` quando dbPath dentro do vault (aceite #15/R-A10) | MELHORIA |
| D-P3-7 | FTS5 | tabela criada e mantenida como scaffold; **busca real (bm25/hybrid) é P5** — `SearchPort` continua stub em P3 | ESCOPO |

### NÃO VALIDADO nesta fase

- Execução/CI no **Windows** (branches de caminho testadas via plataforma pura e `Platform::Windows`; lock via `LockFileEx` só verificável em CI windows-latest — P9).
- FTS5 consulta (rankings, bm25, hybrid C3/C4) — P5.
- Vault fs (replace atômico, watcher 250ms, `*.sync-conflict-*`) — P6 (P4 = parser).
- Embeddings (tabela scaffold; `ort` em P5).

### Próxima fase: P4 — Parser markdown (vocabulário do legado, sem pulldown-cmark: paridade por construção) + frontmatter round-trip (ADR-004)
+ fixtures do legado → **saída: parse idêntico no vocabulário** (habilita `sync` real com tags/links/frontmatter corretos).

---

## P4 — Parser markdown + frontmatter round-trip (ADR-004) + fixtures do legado

**Data:** 2026-10-06. **Autorização:** produto ("Pode prosseguir para P4"). **Saída:** parse idêntico no vocabulário ao legado.

`infra::parser` espelha byte a byte `src/utils/markdown-parser.ts` (`MarkdownParser.parse`) e a reconstrução
de `SyncService.parseNote` (`SyncService.ts:121`). O core **não mudou** (o parser é 100% adapter).

### Escopo entregue

| Item | Descrição |
|---|---|
| `MarkdownParser.parse` | Vocabulário do legado: frontmatter (regex `^---\n...\n---\n`), headings ATX `#`–`######` + slugificar, tags inline `#(...)` (inclusive dentro de code blocks — quirk preservado), wiki-links `[[target\|alias]]`, code blocks fenced (`\w*` language, "" → "text"), callouts Obsidian (17 tipos) com linhas `>` de conteúdo. Dedup por valor normalizado / por target com ordem da 1ª ocorrência. Parser **linear** (scan por linha/caractere), **sem pulldown-cmark** |
| `ParserPort` | `parse_note(path, content) -> Note`: id sem `.md` (aceite #7), title = `fm.title \|\| basename`, tags = união(fm, inline), wiki-links = inline dedup (1º guarda alias) + `fm.links` não vistos, project = `fm.project`, metadata com `created`/`updated` do FM (parse ISO, fallback `now_ms`) |
| Data handling | `new Date(str).getTime()` para `YYYY-MM-DD` (UTC meia-noite) e ISO-8601 completo com `Z`/offset; algoritmo `days_from_civil` (inverso do `civil_from_days` do core) — **std-only**, sem dependência nova |
| Frontmatter opaco | `Frontmatter::parse` com fallback para `Frontmatter::empty()` em YAML inválido/avançado ⇒ **lê como opaco** (conteúdo segue legível), alinhado à matriz FRONTMATTER ("não destrói nada"); aviso `doctor` + recusa de edição = camada de escrita P6 |
| Fixtures | Paridade de vocabulário contra `tests/fixtures/notes/*.md` → `*.note.json` (title, tags inline, wiki-links c/ alias, headings, code blocks, callouts, bodyFirstLine) + round-trip `parse → to_markdown → parse` idempotente no 2º pass |

### Evidência de validação (após a alteração)

| Comando | Resultado |
|---|---|
| `cargo build --workspace` | ✅ sem warnings |
| `cargo test --workspace` | ✅ **97/97 passam** (core 69 + infra 28) |
| `cargo clippy --workspace --all-targets` | ✅ 0 |
| `cargo fmt --all -- --check` | ✅ limpo |

Testes novos (13 em `parser.rs`): os 7 casos do `parser.test.ts` legado portados (frontmatter/body, tags fm+inline,
wiki-links com alias, headings, code blocks, callouts, Note create/markdown); paridade `sample-full` e `sample-minimal`
contra as fixtures; semântica `SyncService.parseNote` (id strip `.md`, união tags `[architecture, planning, memoryos, design, tag-in-code]`,
união links com `home`/`roadmap` anexados, project, `updated` 2026-10-06 → `1_791_244_800_000`); round-trip estável
(`to_markdown(parse(x))` replicável no 2º pass); `slugify` ASCII-compatível; parser de datas.

### Divergências de comportamento registradas

| # | Componente | Divergência | Classificação |
|---|---|---|---|
| D-P4-1 | Implementação | Parser **linear** (sem pulldown-cmark citado na arquitetura): garante paridade de vocabulário **por construção** com o parser de linha do legado; pulldown-cmark pode entrar em P7 se renderização estrutural exigir | DIFERENÇA INTENCIONAL (implementação; contrato preservado) |
| D-P4-2 | Frontmatter opaco | YAML inválido/avançado → `Frontmatter::empty()` (vocabulário vazio) + conteúdo legível; aviso `doctor`/recusa de edição = P6 (camada de escrita, matriz FRONTMATTER) | ESCOPO |
| D-P4-3 | datas | Parse de `created`/`updated` sem lib (std-only), formatos `YYYY-MM-DD`/ISO-8601; sem sufixo de timezone assume UTC (legado usaria local) | DIFERENÇA INTENCIONAL (documentada) |
| D-P4-4 | id | `parse_note` gera id **sem** `.md` (aceite #7); a fixture `.note.json` registrou `id` **com** `.md` (artefato do gerador P0) | CARACTERIZAÇÃO (fixture divergente do código) |
| D-P4-5 | callouts | Legado **junta** as linhas `>` (comportamento portado e testado); a fixture `callout.content` só traz a 1ª linha ("This is a note callout") | CARACTERIZAÇÃO (fixture truncava) |
| D-P4-6 | tags/links | `SyncService.parseNote` faz **união** fm + inline (home/roadmap anexados aos links, architecture/planning aos tags); fixtures `*.note.json` só capturaram os inline | CARACTERIZAÇÃO (fixture diverge do código) |
| D-P4-7 | goldens P0 | `*.golden.md` modelavam Note **sem frontmatter** (só tags/links da nota — ver teste P1); o round-trip real (parseNote) escreve a **união**, conforme o código legado | CARACTERIZAÇÃO (goldens não provam a união) |

### NÃO VALIDADO nesta fase

- Matriz FRONTMATTER completa (16 × 3 casos): reutilização do raw block byte a byte, edição mínima de linhas, preservação de comentários/comentários/âncoras/aninhados, datas como escritas — é responsabilidade da **camada de escrita** (`Vault` fs, P6/ADR).
- Integração `sync` real (Vault fs + Store + parser juntos) — P6.
- Vault fs (replace atômico, watcher 250ms, `*.sync-conflict-*`) — P6.
- Windows/CI* — parser é CPU-agnético (nenhuma dependência de plataforma); CI multi-plataforma em P9.

### Decisão registrada como nota

- **ADR-0004** criado em `vault/ADR/ADR-0004.md` no formato que `adr create` + `Note.to_markdown` produzem
  (tags flow `["adr", "status-proposed"]`, legíveis pelo parser P4; block-list YAML fica para P6).

### Próxima fase: P5 — Search real (FTS5 bm25 + hybrid/blending C3-C4) + Embeddings (`ort` ONNX, `all-MiniLM-L6-v2`)
+ default de `search.strategy` (keyword → hybrid em C3/C4) → saída: busca equivalente ao legado com ranking bm25/hybrid.

---

## P5a — Search real (FTS5 bm25 + hybrid 50/50 + degradação)

**Data:** 2026-10-06. **Autorização:** produto ("pode prosseguir", após ADR-0004). **Saída:** busca keyword/hybrid/semantic reais sobre a conexão única; contract fica aditivo (`degraded` + `strategy_used`). Embeddings reais entregues em **P5b** (decisão de produto: endpoint NVIDIA, ver adiante).

`infra::search` (`FtsSearch`) implementa `SearchPort` com FTS5 `porter unicode61` + `bm25()`,
filtros `tags`/`links` (ALL, como o legado), `project` exato em SQL (elimina o N+1 do legado),
paginação `page/page_size`, snippet com fold de acentos e normalização de score `0..1` (C4).
Hybrid = `0.5·norm(bm25) + 0.5·cos(consulta, conteúdo inteiro)` — **não** rerank por snippet (C4).
Sem embeddings (P5b/`ort`), `hybrid`/`semantic` degradam para keyword com `degraded=true` + `strategy_used` real.

### Escopo entregue

| Item | Descrição |
|---|---|
| `FtsSearch` | `index` (upsert FTS + embedding se embedder disponível), `remove`, `has_embedding`, `rebuild` (limpa FTS+embeddings e reindexa), `search` com estratégia por query ou default (C3: **hybrid**) |
| Keyword | `notes_fts MATCH` com prefix-wildcard por termo (espelha `runFts` do legado), preservando **letras não-ASCII** para o fold do `unicode61` (C3); `bm25(notes_fts, 0, 5, 1, 1)` (title boost); fallback LIKE explícito se FTS falhar (reportado via `degraded`) |
| Filtros | `tags`: nota precisa conter **todos** (legado `every`); `links`: **todas** as edges em `link_edges` (sem N+1 por round-trip ao banco único); `project`: igualdade exata |
| Contrato aditivo | `SearchResult` ganha `degraded` (`serde default`) e `strategy_used` (`skip_serializing_if none`) — SEARCH §fallback; aditivo, não quebra CLI/MCP |
| Config | `search_strategy` default **keyword → hybrid** (`#[default]` no enum + `Config::default`) — C3 (CLI/MCP/config unificados) |
| Store | `conn: Option<Rc<Connection>>` + `shared_connection()` (busca usa a **mesma** conexão — PERSISTENCE single-connection); `notes_fts` vira FTS5 self-contained (remove `content=''`) p/ `snippet()` + DELETE por note_id; `SCHEMA_VERSION` 1→2 (db derivado ⇒ reindex) |

### Evidência de validação (após a alteração)

| Comando | Resultado |
|---|---|
| `cargo build --workspace` | ✅ sem warnings |
| `cargo test --workspace` | ✅ **111/111 passam** (core 69 + infra 42) |
| `cargo clippy --all-targets` | ✅ 0 |
| `cargo fmt --check` | ✅ limpo |

Testes novos (14 em `search.rs`): ranking keyword real (score `0..1`, ordem, matched_fields, snippet);
fold de acentos `café`↔`cafe` e inverso (`sanação`↔`sanacao`); filtros `tags`+`project` AND; filtro `links`
(ALL targets); paginação 1×2 vs 2×2; `remove`; reindex idempotente; `rebuild` limpa/reindexa; degradação
hybrid **sem** embedder → keyword + `degraded`; degradação semantic → idem; hybrid **com** `MemoryEmbed`
→ `strategy_used=hybrid`, score 0..1 decrescente; semantic com embedder → cosine; `has_embedding` ao indexar/remover.

### Divergências de comportamento registradas

| # | Componente | Divergência | Classificação |
|---|---|---|---|
| D-P5-1 | Store/FTS | `notes_fts` de **contentless** (scaffold P3) → **self-contained** (armazena texto) para `snippet()` + `DELETE ... WHERE note_id`; `SCHEMA_VERSION` 1→2 (banco derivado, reindex resolve) | DIFERENÇA INTENCIONAL (implementação; contrato .db é referência, não destino) |
| D-P5-2 | Store | `SqliteStore` passa a `Option<Rc<Connection>>` + `shared_connection()`; busca emite SQL sobre a **mesma** conexão (sem "segunda conexão") | DIFERENÇA INTENCIONAL (PERSISTENCE single-connection; viabiliza search) |
| D-P5-3 | Filtros | stub `MemorySearch` (core) filtrava tags com ANY; legado (`matchesFilters`) e `FtsSearch` usam **ALL** (`every`) | CARACTERIZAÇÃO (stub divergia do legado; busca real segue o legado) |
| D-P5-4 | Paginação | stub usava `limit/offset`; contrato real = `page/page_size` (Default manual corrigido — `derive(Default)` dava `page_size=0` e `LIMIT 0`); snapshoted nos testes | DIFERENÇA INTENCIONAL (contrato F-shapes) |
| D-P5-5 | Score | score normalizado `0..1` (max-min sobre resultados rankeados); legado `Math.abs(rank)` não-limitado | DIFERENÇA INTENCIONAL (SEARCH score 0..1; C4) |
| D-P5-6 | Hybrid | peso `bm25` fixo `HYBRID_W_BM25=0.5` em constante; knob de config adiado (sem mudar shape do contrato) | ESCOPO |
| D-P5-7 | Janela hybrid | fusão sobre topo `limit+offset` do bm25 (como o legado rerankava); janela ampliada/segunda passada = melhoria futura | ESCOPO |
| D-P5-8 | Fold | `sanitize_fts_query` preserva letras não-ASCII (fold via `unicode61`) e remove só a sintaxe FTS5; legado strip-ASCII `\w` apagava acentos (defeito) | DIFERENÇA INTENCIONAL (C3, correção) |

### NÃO VALIDADO nesta fase

- Embeddings reais — entregues em **P5b** (decidido: endpoint NVIDIA free, não `ort` local).
- Default `search.strategy` no fluxo CLI/MCP (P7/P8) — o valor de config já é `hybrid`; a resolução de estratégia por camada sai junto.
- Integração `sync` (store + parser + search no mesmo ciclo) — P6.
- Windows/CI* — FTS/bundled e lock testados em CI multi-plataforma (P9).

---

## P5b — Embeddings reais (NVIDIA free endpoint) [troca de plano: `ort` local → API]

**Data:** 2026-10-07. **Autorização:** produto ("quero uma LLM da NVIDIA com free endpoint no lugar dessa" → escolheu `nvidia/llama-3.2-nv-embedqa-1b-v2`, depois **substituído por EOL** — ver D-P5b-5), sobrescrevendo o freeze `ort`+`all-MiniLM-L6-v2` do P5a. **Saída:** `EmbedPort` ganha modo passage/query (`embed_for`); `infra::embed::NvidiaEmbed` (ureq) atende o endpoint; `FtsSearch` degrada para keyword offline — **sem rede/key → busca continua funcionando** (SEARCH §fallback).

### Implementado

- `EmbedPort::embed_for(&mut self, texts, as_query)` (método obrigatório) + `embed()` default → modela `input_type: "query"|"passage"` exigido pelo modelo NVIDIA; `reindex` (core) continua usando `embed()` (passage).
- `NvidiaEmbed`: POST `{base}/v1/embeddings`, Bearer `NVIDIA_API_KEY` (env apenas), body `{input, model, input_type, encoding_format:"float", truncate:"NONE"}`, timeout 30s (Agent ureq 3), parsing OpenAI-compatible; `is_available` = key presente; `vector_size` = 2048. `from_env()` → `None` sem key.
- `FtsSearch`: no `index`, falha de embedding **não** quebra sync (swallow); no `search` semantic/hybrid, falha do embed (offline/429) → degrade para keyword com `degraded=true` + `strategy_used="keyword"` (fallback já existente usado de forma mais robusta).
- Testes offline: mock HTTP local (TcpListener) — 4 em `embed.rs` (passthrough `query`/`passage`, parse multi-item, sem key unavailable, contagem de vetores incompatível → erro) + 1 integração `FtsSearch`+`NvidiaEmbed` (`strategy_used="hybrid"`, `degraded=false`, ranking por cosseno). **Nenhum teste toca a rede.**

### Evidência de validação (após a alteração)

| Comando | Resultado |
|---|---|
| `cargo test` | **116 testes ok** (core 69 + infra 47), 0 falhas |
| `cargo clippy --all-targets` | 0 avisos (infra: ureq 3 + serde derive adicionados) |
| `cargo fmt --check` | limpo |

### Divergências de comportamento registradas

| # | Componente | Divergência | Classificação |
|---|---|---|---|
| D-P5b-1 | Embeddings | plano local `ort`+`all-MiniLM` **substituído** por endpoint NVIDIA free (`nvidia/nemotron-3-embed-1b`, dim 2048, NIM API) — decisão explícita do produto (preço/custo zero vs build nativo/bundle de modelo) | DECISÃO DE PRODUTO (sobrescreve freeze ADR/P5a) |
| D-P5b-5 | Modelo NVIDIA | o `nvidia/llama-3.2-nv-embedqa-1b-v2` escolhido atingiu **EOL em 2026-05-18** (410 Gone, confirmado ao vivo); pesquisado `GET /v1/models` e sondados os embeddings disponíveis → substituído por **`nvidia/nemotron-3-embed-1b`** (1B, dim 2048, multilíngue, mesmo contrato `input_type`) — default muda via constante `DEFAULT_NVIDIA_EMBED_MODEL`, sem quebra de API | DECISÃO DE PRODUTO (modelo substituído por EOL; incerteza: disponibilidade no free tier muda com o tempo) |
| D-P5b-2 | Segurança/dados | conteúdo das notas é **enviado à NVIDIA**; chave via env `NVIDIA_API_KEY` (nunca config/repo); sem rede/key/cota → degrada para keyword, nunca falha a busca | DECISÃO DE PRODUTO + requisito (review-security: dados sensíveis saem da máquina; online-only p/ semantic/hybrid) |
| D-P5b-3 | Port API | `EmbedPort` ganha `embed_for(texts, as_query)` por causa do `input_type` do modelo NVIDIA (não é um shape de contrato visível ao usuário; `reindex` continua com `embed` = passage) | DIFERENÇA INTENCIONAL (estende port interno) |
| D-P5b-4 | Vectors | embeddings em `note_embeddings` são **JSON float de dim 2048** (não `Float32Array` do legado Neo4j); a dim é a do modelo NVIDIA e pode mudar sem invalidação de contrato (aditivo, `has_embedding` por nota) | DIFERENÇA INTENCIONAL (PERSISTENCE; banco derivado) |

### NÃO VALIDADO nesta fase

- ~~Chamada real ao endpoint NVIDIA~~ — **VALIDADO em 2026-10-07** com `NVIDIA_API_KEY` real (sessão, não persistida): `nemotron-3-embed-1b` respondeu 2 vetores × dim 2048 em ~520ms; busca híbrida real "banco de dados" → `strategy_used="hybrid"`, `degraded=false`, score 0.729 (1 resultado, só o FTS-match). Instrumentação de latência/cota fica para P7.
- Cota free /**rate limit** (~40 RPM) e latência real sob carga; batching multi-textos (`input` array) ainda sem límite/agrupamento — knob de P7+.
- Config de model/base_url/url de NIM próprio — `DEFAULT_NVIDIA_*` constantes; parametrização junto com configuração do produto (P7).
- "LLM de chat rápido" (ex.: `deepseek`/`llama-3.3` via NVIDIA, resposta por streaming) — **pendência aberta de produto**, escopo separado de embedding (P7+).
- Teste real de degradação sob 429 com embedder configurado (coberto por unidade via mock, não com quota real).

## P6 — Vault fs (escrita) + integração sync

**Data:** 2026-10-07. **Saída:** o vault `.md` passa a ser a **fonte de verdade de fato**: `Application` grava notas/ADRs **antes** do commit do índice (G1/G2), as edições preservam o frontmatter byte-a-byte (matriz FM), e o `sync` re-embeda só o que mudou (fingerprint mtime+size → `file_state` com flag `embedded`). Watcher real (notify, debounce 250ms) com `sync_file`/`apply_vault_event` no `Application`.

### Implementado

- **`MarkdownEditor`** (core, `app/markdown_editor.rs`): `rewritten_body` (strip/replace de bloco FM), `fm_is_editable` (YAML plano, sem sub-lista/bloco/`anchors`/`aliases`/dup keys), `apply_note` com a matriz FRONTMATTER (reuso raw, edição mínima linha-a-linha, recusa FM ao stat inválido/avançado, SEM criar FM em edição de conteúdo).
- **`Vault` (infra)**: escrita atômica (`temp → sync_all → rename`, cleanup do temp em falha), **no-op se conteúdo igual** (não toca mtime — protege o embed-guard), path-safety (rejeita `..`, absoluto, vazio), listagem recursiva ignorando `.memoryos`/`.trash`/`Templates`, delete idempotente, `stat` = (mtime_ms, size), `ensure_structure` com os 12 dirs do legado.
- **`VaultPort::stat`** + `VaultStat{f mtime_ms, size}` — fingerprint do arquivo para o embed-guard (`MemoryVault.stat` deriva do próprio conteúdo; `Vault.stat` lê metadata do fs).
- **`Application`**: `create_note`/`create_adr` gravam `.md` (antes do tx); novo `persist_note` com **detecção de conflito** externo (lê raw on-disk, compara `strip_frontmatter`/FM; se alterado fora do app → `AppError::Vault("conflito: externo")` e aborta a escrita); fix de data-loss via `effective_entries` (união fm ∪ tags ∪ links para a checagem de alterações).
- **P6/T4 embed-guard**: `StorePort.{put,get,delete}_file_state` + `FileState{mtime_ms, size, embedded}` no `SqliteStore` (tabela `file_state`), `SearchPort::index_skip_embeddings` no `FtsSearch` (`index_impl` com flag `with_embedding`), stub `MemorySearch` contabilizando chamadas; `sync` reescrito (stat+file_state → `indexed` só no que mudou) e `reindex` sincronizando o stat depois.
- **Watcher (T5)**: extensão do `Vault` com `watch(callback)` → thread de debounce (250ms, mesma do legado), mapeando `notify` → `VaultEvent` (`Rename(From)→Deleted`, `Rename(To)→Created`, `Rename(Both)`
  → `Deleted(old)+Created(new)`, `Modify*`→`Modified`, excluindo `.memoryos`/`.trash`/`Templates` e `.tmp` do write atômico); `WatchHandleImpl::stop` para o loop ≤250ms e **não engole o lote pendente** (flush no stop agora é determinístico).
- **`Application::sync_file`/`apply_vault_event`** — delta sem walk: `Created`/`Modified` indexam (embed-guard), `Deleted` limpa nota+embedding+file_state, `Renamed` = delete+create (IDENTITY). Teste de integração `watch_emits_md_events_debounced_and_ignores_db` usa fólos reais do fs (rename atômico → Created; `fs::write` em lugar → Modified; delete → Deleted) com barreira de registro para o race do inotify ao criar novos diretórios.

### Evidência de validação

| Comando | Resultado |
|---|---|
| `cargo test` | **148 testes ok** (core 90 + infra 58), 0 falhas |
| `cargo clippy --all-targets` | 0 avisos |
| watch real | 3× runs OK do teste de integração (TempDir, eventos Created/Modified/Deleted + filtro `.memoryos`/temp) |

### Divergências de comportamento registradas

| # | Componente | Divergência | Classificação |
|---|---|---|---|
| D-P6-1 | Editor de markdown | `MarkdownEditor` vive no **core** (não na infra) — é regra de produto usada por `persist_note`; o core não pode importar infra (freeze arquitetural) | DIVERGÊNCIA DE ARQUITETURA (intencional) |
| D-P6-2 | Watcher | eventos para arquivos recém-criados dentro de **dir recém-criado** podem se perder no inotify (recursive-add assíncrono) — é race conhecido da API; o watcher faz barreira de registro apenas nos testes (não acopla à callback real) | LIMITAÇÃO DE PLATAFORMA (documentada) |
| D-P6-3 | Debounce | dedupe coalesce por (path, tipo) na mesma janela; tipos distintos são preservados; janela fixa 250ms (legado) | DIFERENÇA INTENCIONAL (igualdade com o legado) |
| D-P6-4 | Renomeação | `Rename` vira `Deleted(old)+Created(new)` (IDENTITY delete+insert); o cb nunca emite `VaultEventType::Renamed` do adapter | DIFERENÇA INTENCIONAL |

### NÃO VALIDADO nesta fase

- Binário/`memory watch` ainda **sem wiring** (o binário ainda não existe; está fora do escopo do P6 — sinalizar como próximo passo).
- Watcher com volume alto (>1k arquivos) e filesystems não-inotify (Windows/macOS) — `RecommendedWatcher` troca o backend; a assersion do legado é só Linux/inotify.
- E2E do `sync_file` contra aplicação real (com embedding da NVIDIA) — o teste do guard é com stubs.

## P6.5 — Correções de contrato + validação da plataforma primária

**Data:** 2026-10-07. **Origem:** auditoria P6 (`docs/p6-audit-2026-10-07.md`) — classificada "VALIDADA COM RESSALVAS". Esta fase corrige **somente** os gaps confirmados A-01..A-04, mitiga W-1/W-2 no código (sem host Windows disponível nesta sessão) e caracteriza A-11.

### Implementado

- **A-01 (file_state)** — `Application::record_file_state(&note)` chamado após o commit em `create_note`, `persist_note` e `create_adr`: fingerprint físico do que está no vault + flag real de embedding (`has_embedding`). Arquivos criados/atualizados não re-embedam no próximo sync.
- **A-02 (parse errors)** — loop do `sync` reestruturado: erro de leitura OU de parse OU de stat cai em `errors[]` e **continua** com as demais notas. Contrato INDEX/F14 restaurado.
- **A-03 (rollback)** — três camadas: (1) `Application::within_tx` envolve o corpo transacional de `create_note`/`persist_note`/`create_adr` e faz `rollback()` em qualquer erro interno; (2) `SqliteStore::commit` com erro faz rollback best-effort e libera `in_txn`; (3) `impl Drop for SqliteStore` reverte tx abandonada no descarte. Self-check: legado TS `cli.ts:253-258` usa o mesmo padrão try/rollback.
- **A-04 (G1 dir fsync)** — `Vault::write_note` após o rename faz fsync do diretório pai (Linux: `File::open(dir).sync_all()` funciona; Windows: std não expõe `FILE_FLAG_BACKUP_SEMANTICS` → abertura falha e o passo é pulado silenciosamente — **diferença de plataforma documentada, não validada em execução Windows**).
- **W-1 (código)** — escrita do temp agora usa o **mesmo handle aberto para escrita** (`File::create` → `write_all` → `sync_all`), eliminando o padrão read-only+fsync que falharia em `FlushFileBuffers` no Windows (exige `GENERIC_WRITE`).
- **W-2 (código)** — `map_events` agora trata `RenameMode::Both` como `Deleted(from)+Created(to)`, cobrindo backends que entregam rename em evento único.
- **A-11 (paridade legado)** — caracterizado: **PARIDADE**. O `create` do TS (cli.ts:224-267, mcp/server.ts:560+) não verifica existência do path; o repo é upsert (`ON CONFLICT DO UPDATE`) e `VaultAdapter.writeNote` é `writeFile` direto. Overwrite de path existente = comportamento do legado (não-regressão). Observação: o CLI legado nem sempre escrevia o `.md` no create — o Rust escreve antes do commit por decisão arquitetural G1/G2 (melhoria intencional já registrada).

### Evidência de validação (após as alterações)

| Comando | Resultado |
|---|---|
| `cargo test` | **157 testes ok** (core 96 + infra 61), 0 falhas |
| `cargo clippy --all-targets` | 0 avisos |
| `cargo fmt --check` | limpo |

### Divergências/observações registradas

| # | Item | Nota | Classificação |
|---|---|---|---|
| D-P6.5-1 | W-1 execução Windows | padrão problemático removido do código; execução real em Windows indisponível neste ambiente | **DESCONHECIDO** (runtime) |
| D-P6.5-2 | W-2 execução Windows | mapeamento `Both → Deleted+Created` adicionado e coberto por unit-test sintético; stream real de eventos do Windows não executável aqui | **DESCONHECIDO** (runtime) |
| D-P6.5-3 | A-11 | `create_note` sobrescrevendo path existente = paridade com legado | PARIDADE |

## P7 — Binário `memory` (wiring CLI end-to-end)

**Data:** 2026-10-07. **Escopo:** compor os adapters reais sobre o `Application` (restrições da P6.5 mantidas: sem MCP/exec, sem alteração de contratos).

### Implementado

- Infra: `FsConfigStore` (load/save atômico de `memory.config.json`) e `ProcessRunner` (std::process com timeout por `try_wait`), ambos com testes.
- Bin `second-brain-bin` (`memory`): `init`, `create`, `read`, `search`, `sync`, `watch`, `stats`, `doctor`, `reindex [--force]`; `run()` testável (retorna texto); `dispatch`/`main` só adaptam argv/exit code; `watch` em loop com sink + `max_events` para teste.
- Composição: `SqliteStore::open` (FileLock exclusivo) → `shared_connection` → `FtsSearch` com `NvidiaEmbed::from_env()` (None → degrade keyword); `NullEmbed` no `Application` quando sem chave (reindex falha honestamente).
- Config: arquivo do cwd ou defaults da plataforma (vault=<cwd>/vault; db fora do vault por `resolve_default_db_path`); merge via `Config::from_json().merge_defaults()` + `validate()`.

### Defeito de contrato encontrado na integração (IDENTITY)

`Note::create` não normalizava o id; o parser P4 "strippa" `.md`. No fluxo real (create via CLI → sync), ids duplos (`Knowledge/x.md` vs `Knowledge/x`) quebravam UNIQUE(path). Fix: `Note::create` faz `id = path.strip_suffix(".md")` (idempotente, path intacto); `apply_vault_event` deleta/remove pelo id canônico; expectativas de teste atualizadas para o id canônico. **Classificação:** correção de bug (alinhamento ao FREEZE), não mudança de contrato.

### Evidência de validação

| Comando | Resultado |
|---|---|
| `cargo test --workspace` | **163 testes ok** (bin 3, core 96, infra 64) |
| `cargo clippy --all-targets -- -D warnings` | limpo |
| `cargo fmt --all -- --check` | limpo |

Bin testa init→create→read→search→stats→sync→doctor(+reindex offline erra honestamente) e watch com evento real via fs em tempdir.
# P7b — Chave NVIDIA via .env

**Data:** 2026-10-08.

## Mudanças

- `memory` agora carrega `{cwd}/.env` provisoriamente (`KEY=VALUE`, sem sobrescrever env real) — conveniência para uso local (bench MCP/CLI). Nunca versionado (.gitignore).

## Verificação de produção (ao vivo)

- Sync com chave: 3 notas embedadas em 1.9s (dim 2048).
- Search híbrida: 'redes neurais e inteligência artificial' → a nota de embeddings rankeada via keyword+semantic, degraded=false.
- Re-sync imediato: `indexadas 0/3` — embed-guard **evita** re-chamadas com file_state.embedded=true.
- MCP (opencode) lê a mesma `.env` do diretório de trabalho do servidor (`mcp-demo/`)


# P7c — MCP: similar, context, backlinks, adr_list

**Data:** 2026-10-08.

## Mudanças

- `StorePort::find_backlinks(target_id)` / `find_outgoing_links(source_id)` (SQL
  em `link_edges` + stub em memória) — espelham `MemoryNoteRepository` do legado.
- `Application::similar(query, limit)`, `Application::context(query, max_docs,
  include_content, strategy)`, `Application::backlinks(id)`, `Application::adr_list()`
  + contratos `NoteRef/BacklinksOutput/ContextDoc/AdrSummary`.
- MCP: 4 tools novas (total 11/16 implementadas). Sem cancelamento de contrato.

## Verificação (testes)

- 8 testes bin (4 novos MCP via infra real em tempdir), 97 core (+1), 64 infra.
- Prova ao vivo com embeddings reais (vault `/tmp/opencode/nv-test`):
  `similar "receita de bolo"` → Bolo 0.544 / Embeddings 0.143 / Home 0.045
  (ranking por cosseno); `context "semântica"` → SOURCE/TITLE/RELEVANCE + snippet;
  backlinks/adr_list respondem estruturas corretas.

# P7d — MCP project_* + correção de falso conflito (double-encoding de FM)

**Data:** 2026-10-08.

## Mudanças

- **BUG (double-encoding de FM, `crates/second-brain-infra/src/store.rs`)**: `frontmatter_to_json` serializava valores `Str` com `{:?}` sobre um JSON já serializado (`format!("{}:{:?}", to_string(k), to_string(s))`), gravando `{"title":"\"Beta\""}`; a releitura recuperava `Str("\"Beta\"")` com aspas literais. Corrigido para `{}` (o braço `List` já estava correto). Regressão: `frontmatter_json_roundtrip_preserves_string_values`.
- **BUG (falso conflito em `persist_note`, `crates/second-brain-core/src/app/application.rs`)**: o check de conflito externo comparava o FM on-disk (que inclui `tags`/`links`, mesclados por `to_markdown`) contra `previous.frontmatter()`, que em notas criadas por `Note::create` guarda só `title` (tags/links ficam em colunas/vetores). Resultado: qualquer `set_project`/`update` em nota com tags abortava com "conflito: ... alterado externamente". Corrigido: `fm_differs` substituído por `fm_entries_differ` comparando `frontmatter_from_raw(&raw).entries()` contra `effective_entries(&previous)` (FM ∪ tags ∪ links), mantendo o guard `faithful`. `effective_entries` passou a `pub(crate)`.
- **MCP `project_*` (4 tools)**: `project_create`/`project_list`/`project_show`/`project_link` em `mcp.rs`; contratos `ProjectSummary/ProjectNote/ProjectShow` (camelCase). Total 15/16 — falta apenas `second_brain_exec` (decisão explícita do usuário).

## Verificação (testes + prova ao vivo)

- 9 bin (+1), 98 core (+1), 65 infra (+1) = **172 testes**; `clippy -D warnings` e `fmt` limpos.
- Core: `project_list_show_and_link` passa (StubParser não pegava o bug por `faithful=false`).
- MCP/infra real: `project_create_list_show_and_link` passa (era o teste que expunha o falso conflito).
- Prova ao vivo (`/tmp/opencode/probe`): `project_link` vinculou `Knowledge/b.md` sem falso conflito; arquivo ganhou `project`/`updated` corretamente.

# P7e — MCP `second_brain_exec` (configurável + allowlist; contrato 16/16)

**Data:** 2026-10-08.

## Mudanças

- **DIFERENÇA INTENCIONAL (C-exec / F24, FREEZE 3)**: o legado executava shell
  arbitrário (`cmd.exe /c`, `server.ts:786`). Aqui: **desabilitado por default**,
  **allowlist obrigatória**, **sem shell** e **sem `env` do chamador**.
- `Config.exec: ExecPolicy` (`enabled` default `false`, `allowed: []`,
  `timeoutMs` default 30s, `maxTimeoutMs` default 300s), camelCase.
- `CommandSpec.cwd` (novo) — `ProcessRunner` aplica `current_dir`; `cwd` = vault.
- `Application::exec(command_line, timeout_ms)`: tokeniza a linha **sem shell**
  (`parse_command_line`: aspas simples/duplas + `\`; `|`,`&&`,`;`,`>`,`$`,backtick
  são literais), valida allowlist por **basename** (`git` ≡ `/usr/bin/git` ≡
  `git.exe`), fixa `cwd`/timeout e roda via port `CommandRunner`.
- MCP `second_brain_exec`: rejeita `env` do chamador; saída em texto (formato
  legado `stdout:/stderr:/exit code:`) — o wrapper MCP passa payload string cru
  (objetos continuam JSON). **Contrato 16/16 tools completo.**
- `Application::run_command` (passthrough sem gate) removido — substituído por `exec`.

## Verificação (testes + prova ao vivo)

- **182 testes** (13 bin, 104 core, 65 infra); `clippy -D warnings` e `fmt` limpos.
- Core: `parse_command_line_*` (quotes/escape/metachar literal/erros), `exec_disabled_by_default_in_core`, `exec_runs_when_allowlisted` (StubRunner); config: `exec_disabled_by_default_and_parses_policy` (basename/`.exe`).
- Bin (infra real): `exec_disabled_by_default`, `exec_rejects_non_allowlisted_program`, `exec_rejects_caller_env`, `exec_runs_allowlisted_command_in_vault`.
- Prova ao vivo JSON-RPC (`/tmp/opencode/exec-probe`, `exec.allowed=["git"]`):
  `git init -q` → `exit code: 0` **e `.git` criado dentro do vault** (cwd=vault);
  `git status` → bloco `stdout:` correto; `curl …` → erro de allowlist;
  `env` do chamador → erro; config sem `exec` → "exec desabilitado".

## Pendência remanescente (produto)

- A **lista** default de comandos/candidatos (`git`, `docker`, `aws`, `rage`,
  `vercel`) e quais rodam no Windows segue decisão de produto. O mecanismo está
  pronto: basta preencher `exec.allowed` na config.
