# SECOND BRAIN — LEGACY BASELINE

> **Artefato:** baseline congelado do legado TypeScript — base para a migração Rust.
> **Origem:** inspeção estática completa (36 arquivos `src` ≈ 6.400 linhas, 4 arquivos de teste, `bin/`, configs, CI — `git status` inalterado, **nenhuma alteração feita ao projeto**), banco de produção real (`/home/inetserver/vault/.memoryos/index.db`, leitura somente) e experimento controlado `/tmp/opencode/i1/i1.ts`.
> **Versão inspecionada:** worktree atual, contendo diff não commitado sobre HEAD (`package.json`, `cli.ts`, `SyncService.ts`, `MemorySearchIndex.ts`; `pnpm-lock.yaml`/`pnpm-workspace.yaml` removidos; `package-lock.json` novo). O baseline descreve **o código como está hoje no worktree**, com divergências vs HEAD anotadas.
> **Classificação de evidência:** CONFIRMADO / PROVÁVEL / HIPÓTESE / DESCONHECIDO.

---

## 1. ARQUITETURA (CONFIRMADO)

Sistema **monolito modular TypeScript, processo único, local**, sem rede HTTP. Três pontos de entrada sobre um núcleo compartilhado:

```text
USUÁRIO ──► CLI (commander, src/cli/cli.ts) ──┐
                                                ├──► infrastructure ──► sql.js (WASM, .db) + vault .md
AGENTE ──► MCP stdio (src/mcp/server.ts, SDK) ─┘          ▲
                                              domain (Note/ADR/Project/Session/VO) usado por todos
                                              application/ports/VaultPort   ◄── Vazio (1 arquivo, sem uso real)
```

- Camadas: `domain/` (entidades + value objects imutáveis) → `infrastructure/` (persistência, busca, sync, grafo, obsidian) → entradas `cli`/`mcp`.
- **Violação de fronteira (CONFIRMADO):** entradas importam infraestrutura diretamente; a lógica "transação + salvar + indexar" está duplicada em ≥8 pontos (17 ocorrências de `beginTransaction`).
- **CONFIRMADO:** repositório e índice abrem **duas conexões sql.js independentes** sobre o mesmo `.db` (`cli.ts:61-63`, `server.ts:74-76`; `openSqlite` em `MemoryNoteRepository.init` / `MemorySearchIndex.init`).
- **CONFIRMADO (prova em /tmp):** cada conexão grava `fs.writeFileSync(dbPath, db.export())` do arquivo inteiro (`connection.ts:55-59`). Efeitos detalhados na §15.
- Config por `process.cwd()/memory.config.json` (sem schema/validação) ou TOML/frontmatter legados (CONFIRMADO em `cli.ts:34-50`, `server.ts:37-51`).

## 2. MÓDULOS (CONFIRMADO)

| Módulo | Arquivo(s) | Responsabilidade |
|---|---|---|
| CLI | `src/cli/cli.ts` (1070 l.) | 25+ subcomandos |
| MCP | `src/mcp/server.ts` (802 l.) | 16 tools + 3 resources + 2 prompts |
| Domain | `src/domain/entities/{Note,ADR,Project,Session,KnowledgeGraph}.ts` | entidades imutáveis |
| Value objects | `src/domain/value-objects/` | NoteId, Tag, WikiLink, ProjectId, SessionId, Frontmatter, Metadata |
| Persistência | `infrastructure/persistence/MemoryNoteRepository.ts` (+ `sqlite/connection.ts`, `JSONNoteRepository*`, `SQLiteNoteRepository*`) | CRUD notas/backlinks + grava `.md` |
| Busca | `infrastructure/search/MemorySearchIndex.ts`, `EmbeddingService.ts` | keyword/semântico/híbrido + vetores |
| Sync | `infrastructure/file-system/sync/{SyncService,VaultWatcher,ReflectService,StudyLinkService}.ts` | ingestão, watch, sessões, auto-link |
| Grafo | `infrastructure/graph/GraphService.ts` | métricas (density, grau, componentes, centralidade) |
| Obsidian | `infrastructure/obsidian/{Config,URI,GraphExport,PluginGen}.ts` | detecção, URI, export, **geração de plugin** |

`*` código morto: `JSONNoteRepository` e `SQLiteNoteRepository` (este último excluído do `tsconfig`, importa `better-sqlite3` que não está nas dependências).

## 3. FUNCIONALIDADES (catálogo)

| ID | Funcionalidade | Implementação TS | Entrada | Saída | Dependências | Status |
|---|---|---|---|---|---|---|
| F1 | Busca por palavra-chave | `MemorySearchIndex.search` keyword | `query, tags?, links?, project?, limit?, offset?` | `SearchResult[]{noteId,score,matchedFields,snippet}` + total/page | `.db` `notes_fts` | CONFIRMADO (LIKE, §10) |
| F2 | Busca semântica | `searchSemantic` | `query, limit?` | rank por cosseno | `note_embeddings`, MiniLM | CONFIRMADO |
| F3 | Busca híbrida | `searchHybrid` | `query, page?` | keyword + rerank por cosseno do snippet | ambas | CONFIRMADO |
| F4 | Busca similar | tool `second_brain_similar` | `query, limit` | top-N cosseno | embeddings | CONFIRMADO |
| F5 | Ler nota | tool `read` / CLI `read` | `path` ou `id` | conteúdo, metadados, tags, links, backlinks | repo/backlinks | CONFIRMADO |
| F6 | Criar nota | tool `create` / CLI `create` | `path, content, title?, tags?, links?, project?` | grava `.md` + `.db` + índice + backlinks | repo/index + fs | CONFIRMADO (com defeito §15.1) |
| F7 | Editar nota | CLI `create` (upsert) | — | igual a criar | — | PROVÁVEL (edição implícita por `create`; sem tool de update dedicada) |
| F8 | Backlinks/outgoing | tool `backlinks` | `path` | `{note, backlinks[], outgoingLinks[]}` | `link_edges` | CONFIRMADO (dados degradados, §15.2) |
| F9 | Contexto RAG | tool `context` / prompt `context_summary` | `query, maxDocuments?, includeContent?` | docs com `SOURCE/TITLE/RELEVANCE` + conteúdo (truncado 500) | search + findById | CONFIRMADO |
| F10 | ADR create/list/numeração | tools `adr_create/list`, CLI `adr` | `title, context, problem, solution, alternatives?, consequences?` | `.md` em `ADR/` | ID | CONFIRMADO (numeração quebrada, §18) |
| F11 | Projeto create/list/show/link | tools `project_*`, CLI `project` | `name, description` / `id` / `projectId+notePath` | frontmatter `project` + nota overview | repo | CONFIRMADO |
| F12 | Stats | tool `stats`, CLI `stats` | — | totais de notas/tags/projetos | repo | CONFIRMADO |
| F13 | Grafo | tool `graph`, CLI `graph` | `path?` | nodes, edges, density, centralidade | `link_edges` | CONFIRMADO (sub-alimentado) |
| F14 | Sync manual | CLI `sync`, MCP start (`deferSync`) | vault | `{scanned,indexed,removed,errors,durationMs}` | fs + repo + index | CONFIRMADO |
| F15 | Reindex | CLI `reindex [--force]` | — | recria índice (embeddings) | index + embeddings | CONFIRMADO |
| F16 | Watch | CLI `watch` (chokidar+debounce 250ms) | vault | atualiza nota sob mudança + reflect | fs watcher | CONFIRMADO |
| F17 | Reflexão | CLI `reflect`, `autoReflect` | `summary?, tasks?, decisions?` | grava `Sessions/*.md` | repo/index/fs | CONFIRMADO |
| F18 | Doctor | CLI `doctor` | — | relatório de saúde | repo | CONFIRMADO (verificações exatas NÃO VALIDADAS) |
| F19 | Backup | CLI `backup` | `-o path` | `.zip` via **PowerShell** (Windows-only) | shell | CONFIRMADO (quebrado no Linux) |
| F20 | Open no Obsidian | CLI `open` | `path, --line` | levanta Obsidian via URI `obsidian://` | shell `start` (Windows) | CONFIRMADO (Windows-only) |
| F21 | Obsidian detect/plugin/graph | CLI `obsidian` | — | detecta vault; `plugin install` escreve plugin que chama **HTTP :3100** (inexistente) | fs, URL | CONFIRMADO (§18) |
| F22 | Study link | CLI `study link`, `SyncService` | — | auto-links `Studies/` ↔ `Knowledge/` | saveNote | CONFIRMADO |
| F23 | Config get/set | CLI `config` | `--get k`, `--set k=v` | lê/escreve `memory.config.json` | fs | CONFIRMADO |
| F24 | Exec shell via MCP | tool `second_brain_exec` | `command, timeout?, env?` | stdout/stderr/exit | shell `cmd.exe` | CONFIRMADO (risco §15.4) |

## 4. CONTRATOS

### 4.1 Contrato de arquivo (nota `.md`) — CONFIRMADO

- Fonte primária: arquivos Markdown no vault; layout de pastas livre.
- **ID = caminho relativo sem `.md`** (`SyncService.parseNote:128-129`). **Exceção observada no banco real:** 28 ids com sufixo `.md` (origem: caminho de criação direto via tools/CLI) → dois esquemas coexistem.
- **Frontmatter** (parser próprio, `Frontmatter.ts:19-53`, não-YAML completo): chaves `title, tags[], project, date, updated, aliases[], links[], [novas]`. Listas separadas por vírgula, sem suportar aninhamento/multilinha. Reescrita do frontmatter ao salvar (`Note.toMarkdown`, `Note.ts:184-210`): valores entre aspas; listas `["a", "b"]`; apenas campos não vazios.
- Tags: `#tag` inline (inclusive em codeblocks — risco §18) + frontmatter; normalização `Tag` (lowercase, validação).
- Wiki-links: `[[alvo]]` e `[[alvo|alias]]`, deduplicados no sync.
- Callouts Obsidian e blocos de código detectados pelo parser (§8).
- Convenção de pastas: `Knowledge/`, `Studies/`, `Documentations/`, `Sessions/`, `ADR/` usadas pelo auto-link e pelo parser — **sem enforcement** (qualquer path é aceito).

### 4.2 Contrato de dados `.db` (esquema, `connection.ts:61-158`) — CONFIRMADO

```sql
meta(key PK, value)
notes(id PK, path UNIQUE, title, content, project_id, tags JSON, links JSON,
      created_at INT, updated_at INT, version INT DEFAULT 1)   -- índices path/project/updated
link_edges(source_id FK→notes ON DELETE CASCADE, target, target_id)  -- sem UNIQUE/PK
notes_fts(note_id PK, title, content, tags)     -- TABELA COMUM (FTS5 indisponível, §10)
note_embeddings(note_id PK FK→notes, embedding TEXT base64 de Float32[384])
meta.schema_version = 3
```

Banco real: `notes=389`, `notes_fts=228`, `note_embeddings=229`, `link_edges=34`, `version=1` em 100% (CONFIRMADO, via python3 sqlite3 read-only).

## 5. CLI (CONFIRMADO — tree de comandos)

```text
second-brain <command>
  init      [-p, --path <vault>]                     # cria config default no cwd
  search    <query> [-t tags...] [-l links...] [-p project]
            [--strategy keyword|hybrid|semantic] [--semantic]
            [--page N] [--page-size N]
  read      <path-or-id>
  create    <path> [-t title] [-c content] [--tags ...] [--links ...]
  sync
  reindex   [--force]
  graph     [-p <note>] [--json]
  watch
  doctor
  stats
  reflect   [-s summary] [-t tasks...] [-d decisions...]
  mcp                                       # inicia o servidor MCP (stdio, bloqueante)
  adr create [--alternatives ...] [--consequences ...] / list / accept <id>
  session
  project   create [--overview <path>] [--architecture <path>] [--roadmap <path>]
            / list / show <id> / link <project-id> <note-path>
  config    [--get <key>] [--set <k=v>...]
  backup    [-o <path>]                       # Windows-only (PowerShell)
  open      <path> [--line N]                # Windows-only (start Obsidian)
  obsidian  detect / plugin install / graph [-o <path>]
  study link
```

- Exit codes: `process.exit(1)` em erros (0 no sucesso); sem convenção além disso (CONFIRMADO leitura; NÃO VALIDADO em execução).
- `cli.ts:67-68`: fecha `repo.close()` → `index.close()` (ordem que importa, §15.1).

## 6. MCP (CONFIRMADO — contrato integral)

Protocolo: stdio, SDK `@modelcontextprotocol/sdk@1.29.0`, `Server {name:"second-brain", version:"1.0.0"}`, capabilities `tools+resources+prompts`. Erros de tool em JSON `{error: msg}` (try/catch por handler).

**Tools (16)** — schemas completos em `server.ts:86-263`:

| Tool | Params (obrigatórios) | Retorno |
|---|---|---|
| `second_brain_search` | query(*) | nota com score/snippet/tags/links + total |
| `second_brain_similar` | query(*) | top-N cosseno |
| `second_brain_read` | path? \| id? | conteúdo + metadata+tags+links+backlinks |
| `second_brain_context` | query(*) | `{source,title,content,relevance}[]` + backlinks |
| `second_brain_create` | path(*), content(*) | `{created,id,path,title}` |
| `second_brain_backlinks` | path(*) | `{note,backlinks[],outgoingLinks[]}` |
| `second_brain_stats` | — | `{totalNotes,totalTags,totalProjects}` |
| `second_brain_graph` | path? | `{nodeCount,edgeCount,density,...}` |
| `second_brain_info` | — | `{vaultPath,dbPath,strategy,capabilities}` |
| `second_brain_adr_create` | title,context,problem,solution(*) + alternativas/consequências | `{path,title,content}` |
| `second_brain_adr_list` | — | lista de ADRs com status |
| `second_brain_project_create` | name(*), description(*) | projeto criado |
| `second_brain_project_list` | — | projetos + contagem |
| `second_brain_project_show` | id(*) | projeto + notas |
| `second_brain_project_link` | projectId(*), notePath(*) | status |
| `second_brain_exec` | command(*) + timeout(≤300000)/env | stdout/stderr/exitCode — **executa shell** |

**Resources (3):** `second-brain://notes` (JSON de todas as notas; `pageSize:9999`), `second-brain://stats`, `second-brain://config`.
**Prompts (2):** `context_summary(query)` → top-N com conteúdo truncado a 500 chars + `SOURCE/TITLE/RELEVANCE`; `search_and_read(query)` → top-1 com título+conteúdo completo (`server.ts:349-407`).
**Notas de contrato:** sem validação de schema (zod instalado, nunca usado); `searchStrategy` default MCP = `"hybrid"`, CLI = `"keyword"`; sync inicial dispara fire-and-forget (`deferSync()`, `server.ts:79`) e MCP **nunca fecha as conexões** (nenhum `.close()`).

## 7. VAULT (CONFIRMADO)

- Vault de produção: `/home/inetserver/vault` (apontado por `memory.config.json`, gitignored; contém `.stfolder` do Syncthing). Vault local do repo: `./vault` (vazio, só `Credentials/.age`).
- `216` arquivos `.md` em disco; **15 `.md` não presentes no banco** (nunca indexados) e **187-188 linhas `notes` sem arquivo no disco** (órifas) — CONFIRMADO (§15.1).
- Conteúdo sensível no vault de produção: `Credentials/` (GitHub, AWS, DockerHub, Vercel) e `sudo-password.md` em texto plano (lado a lado de `.age`) — CONFIRMADO, §15.4.
- Regras: `.memoryos/`, `.trash/`, `Templates/` (em qualquer nível) ignorados no sync (`SyncService.ts:52`); paths normalizados com `replace(/\\/g,"/")`.

## 8. PARSER (CONFIRMADO)

`src/utils/markdown-parser.ts` (264 l.), próprio (não gray-matter):
- Extrai **frontmatter** (`---\n...\n---` no topo), **título**, **tags inline** (`#...` em texto/código/frontmatter), **wiki-links** (com alias), **headings** (nível 1+), **code blocks** e **callouts Obsidian** (`> [!tipo]`).
- Totalmente **case-by-case**, sem YAML real (limitações na §16).
- 14 testes unitários o cobrem e passam.

## 9. INDEX (CONFIRMADO)

- Atualizado por: sync (upsert), create/edit (via `MemoryNoteRepository.saveNote`), reindex.
- Tabelas: `notes_fts` (título/conteúdo/tags) e `note_embeddings` (base64 Float32 384 dims, MiniLM-L6-v2 quantizado).
- **FTS5 indisponível na runtime** (CONFIRMADO experimentalmente: `sql.js` 1.14.1 → `no such module: fts5`; `compile_options` = FTS3/FTS3_PARENTHESIS). A migração v3 tenta FTS5, falha, cria **tabela comum** e marca `schema_version=3` para sempre. `MemorySearchIndex.init` detecta e liga `useFts5=false` (`:77-81`).
- O FTS nunca é persistido de forma estável se a última escrita vier do índice com snapshot velho (ver §15). No worktree o sync usa `indexMany(notes, {skipEmbeddings:true})` (diff não commitado): 1 de 202 notas "vivas" sem embedding.

## 10. SEARCH (CONFIRMADO)

- **Keyword = `SELECT ... WHERE title LIKE ? OR content LIKE ?`** com `%q%` em título/conteúdo/tags (`MemorySearchIndex.ts:367-373`), score = `|rank|` de subquery que sempre = 0 → **`score=0` para todos** (CONFIRMADO).
- **Semântico = scan linear** de todos os `note_embeddings`: base64→`Float32Array(384)` por linha, cosseno (`EmbeddingService.ts:76-89`). Custo O(n) por query; sem índice vetorial.
- **Híbrido = keyword + rerank por cosseno do snippet de 200 chars** (não do conteúdo completo) (`:259-273`).
- Filtros por tags/links/project: match pós-busca + `searchByTags` via `SELECT *` + `JSON.parse` por linha (N+1).
- `resolveTargetId` para backlinks usa `LIKE '%alvo%'` (`MemoryNoteRepository.ts:138-145`) → pode casar múltiplas notas (id observado `Knowledge/nestjs-complete-reference.md`).
- Degradação silenciosa: se embeddings falham, busca vira keyword sem avisar o chamador.
- Sem escapamento de `%`/`_`, sem COLLATE NOCASE (busca com acento em português falha).

## 11. STORAGE (CONFIRMADO)

- SQLite via **sql.js WASM em memória**; arquivo único `vault/.memoryos/index.db` (6.7 MB produção). `PRAGMA journal_mode=OFF`, `foreign_keys=ON`.
- Gravação: `fs.writeFileSync(dbPath, db.export())` **do arquivo inteiro**, sem WAL, sem temp+rename, sem atomicidade.
- Nota `.md` gravada **pelo repositório** (`writeNoteFile`, `MemoryNoteRepository.ts:127-136`) quando `setVaultPath` ativo — storage e banco acoplados, sem transação atômica entre os dois.
- Índice/embeddings/grafo são **derivados**; `.md` é a fonte de verdade do conteúdo.

## 12. MODELO DE DOMÍNIO (CONFIRMADO)

```text
Note       — id(path), path, title, content, tags[], wikiLinks[], projectId?, metadata(createdAt/updatedAt/version), frontmatter; imutável; toMarkdown() regrava frontmatter
ADR        — title, context, problem, solution, status, alternatives[]/consequences[] (registradas em ADR/ADR-NNNN.md)
Project    — id, name, description (manifesta via nota overview + frontmatter project)
Session    — resumos/tasks/decisions → Sessions/*.md
KnowledgeGraph — nodes, edges, density, averageDegree, connectedComponents, centrality (puro, testado)
VO: NoteId, Tag (normaliza), WikiLink (alias), ProjectId, SessionId, Frontmatter, Metadata(Timestamp, version)
```

## 13. DEPENDÊNCIAS (CONFIRMADO — `package.json` + lock)

Runtime: `@modelcontextprotocol/sdk 1.29.0`, `sql.js 1.14.1`, `@xenova/transformers 2.17.2`, `chokidar 3.6.0`, `fast-glob 3.3.3`, `commander 12.1.0`, `chalk 5.6.2`, `ora 8.2.0`, `execa 9.6.1`, `zod 3.25.76` (não importado no src), `gray-matter 4.0.3` (não importado).
Dev: `typescript 5.9.3`, `vitest 1.6.1`, `eslint 10.7.0`, `typescript-eslint 8.65.0`, `tsx 4.23.1`.
Node `>=20`. Migração de gerenciador pnpm→npm **não commitada**; CI (`.github/workflows/ci.yml`) ainda usa `pnpm install --frozen-lockfile` → **CI quebrado no worktree** (CONFIRMADO). `dist/` desatualizado (27/08 vs src 30/08). `node_modules` presente (446 MB).

## 14. TESTES (CONFIRMADO — 34 testes, 4 arquivos)

| Arquivo | Assunto | # test | Executados nesta inspeção |
|---|---|---|---|
| `tests/unit/parser.test.ts` | frontmatter, tags, wiki-links, headings, codeblocks, callouts + Note + Tag + WikiLink + NoteId | 14 | ✅ 14/14 passam |
| `tests/unit/knowledge-graph.test.ts` | grafo: counts, density, grau, componentes, centralidade, JSON | 11 | ✅ 11/11 passam |
| `tests/unit/embedding.test.ts` | similaridade semântica (cat/feline/volcano) | 3 | ⛔ **NÃO EXECUTADO** (baixa modelo HF) |
| `tests/integration/repository.test.ts` | save/retrieve, tags, FTS, count, backlinks, delete | 6 | ⛔ **NÃO EXECUTADO** (cria `tests/.tmp`) |

**Atenção crítica (CONFIRMADO):** o teste de integração **mascara o bug de persistência** ao forçar uma única conexão compartilhada (`Object.assign(repo,{conn,db})`, `Object.assign(index,{conn,db})` em `tests/integration/repository.test.ts:24-28`), cenário que a produção não usa.
Sem testes: server/MCP, CLI, sync, watch, reflect, estudo-link, numeração de ADR, `resolveTargetId`, fallback LIKE, `handleExec`, plugin Obsidian.
`npm run typecheck` → 0 erros; `npx eslint src` → 0 erros / 37 warnings (CONFIRMADO).

## 15. RISCOS (com evidência)

1. **CRÍTICO — duas conexões sql.js = perda de dados.** PROVADO em `/tmp/opencode/i1/i1.ts`: cada `save()` regrava o arquivo inteiro com o snapshot daquela conexão. Sequências observadas: `upsertMany`→`indexMany` deixa o `.db` **sem as linhas `notes`** (FTS órfão); `saveNote`→`indexMany` faz a **nota recém-criada sumir após restart** (`repo.exists=false`); re-sync em banco já mesclado restaura notas mas **perde o índice**; `repo.delete`→`index.removeFromIndexById` deixa **FTS órfão** se o repo estava vazio. `close()` também sobrescreve: CLI fecha `repo→index` (visão do índice vence); MCP nunca fecha. Em tempo de execução tudo "funciona" (leitura em memória); a corrupção só aparece no restart — **explica 188 notas órfãs / 27 FTS órfãos / 15 arquivos não indexados no banco real** (PROVÁVEL, pois o histórico de runs não é auditável).
2. **CRÍTICO — `upsertMany` não escreve `link_edges`** (`MemoryNoteRepository.ts:243-265`) → backlinks/grafo errados após sync (440 wiki-links vs 34 arestas reais; `link_edges` sem UNIQUE → duplicatas possíveis).
3. **ALTO — busca sem ranking** (FTS5 indisponível; LIKE com `score=0`; §10).
4. **ALTO — segurança:** tool `second_brain_exec` executa shell arbitrário (`cmd.exe /c`, env injetável, timeout 60s/300s, PATH com `C:\Users\luiso\...` hardcoded) (`server.ts:774-802`); credenciais do vault em texto plano indexadas e legíveis via `read/search/context`; `open`/`backup` com interpolação em `execSync`; sem validação de entrada.
5. **ALTO — empacotamento:** `bin/mcp.mjs` aponta para `../dist/src/mcp/server.js` (inexistente; build gera `dist/mcp/server.js`); bin publicado depende de `tsx` (devDependency); CI quebrado; `dist/` desatualizado.
6. **MÉDIO — corrida real:** `deferSync()` sem `await` + handlers concorrentes sobre o mesmo repo/index/transação (`transactionDepth` não é seguro a concorrência; `BEGIN` aninhado). Ocorrência em produção: **DESCONHECIDA**.
7. **MÉDIO — identidade:** numeração de ADR, ids `path` vs `.md`, `version` sempre 1, `pageSize:9999` em 9 lugares, N+1 em `handleProjectList`/`searchByTags`, rerank por snippet de 200 chars.
8. **MÉDIO — portabilidade:** comandos Windows-only rodando num host Linux (backup PowerShell, `open`/`exec` com `cmd.exe`).
9. **Segurança positiva (CONFIRMADO):** nenhum segredo versionado; `.gitignore` cobre `memory.config.json`, `**/vault*/`, `.db`, `debug-*`; logs apenas em stderr, sem conteúdo de notas.

## 16. DÍVIDA TÉCNICA

- `VaultPort` vazio + `VaultAdapter` importado sem uso (`cli.ts:7`).
- `JSONNoteRepository` / `SQLiteNoteRepository` mortos (o segundo excluído do build por `tsconfig.json:13`).
- `clearIndex` com ramos `useFts5`/`else` idênticos (`MemorySearchIndex.ts:191-195`).
- `void Frontmatter;` (`SyncService.ts:182`); interfaces duplicadas em `markdown-parser.ts`.
- Parser de frontmatter próprio e limitado (sem aninhamento/multilinha/`:` em valor) apesar de `gray-matter` instalado.
- Defaults divergentes: CLI `searchStrategy:"keyword"` vs MCP `"hybrid"`.
- 37 warnings de lint (`any`, unused); 108 usos de `console.*` misturados com `logger`.
- Duplicação da regra "salvar + indexar + transação" em ≥8 pontos (17 `beginTransaction`).
- `vitest 1.x` defasado (não atualizar sem planejamento); `dist/` desatualizado vs `src`.
- Trabalho não commitado sem decisão (migração pnpm→npm, `skipEmbeddings:true`, guarda `isNaN`, `searchStrategy:"keyword"`).

## 17. COMPORTAMENTO A PRESERVAR

- Contrato de arquivo: `.md` como fonte de verdade; `Note.toMarkdown()` regrava frontmatter com campos não vazios entre aspas.
- Contrato MCP: nomes das 16 tools, parâmetros, respostas com envelope de sucesso, resources `second-brain://notes|stats|config`, prompts `context_summary`/`search_and_read`.
- Contrato CLI: todos os comandos/flags da §5.
- Semânticas de busca: estratégias `keyword | hybrid | semantic`; filtros por tags/links/project; paginação `{page,pageSize,total,hasMore}`.
- Convenções de domínio: `NoteId = path sem .md`, tags normalizadas, alias em wiki-links, pastas `Knowledge/Studies/Documentations/Sessions/ADR`.
- Comportamento idempotente do sync (upsert + remoção de órfãos).
- Cadeia de degradação FTS5→LIKE, semântica→keyword (porém deve passar a ser **visível** ao chamador — melhoria aprovada).

## 18. COMPORTAMENTO SUSPEITO (validar antes de reproduzir em Rust)

- Dupla conexão + save do arquivo inteiro + ordem de `close()` (defeito, não "comportamento").
- `upsertMany` sem `link_edges` (defeito).
- Numeração de ADR sempre `ADR-0001` (`id.match(/^ADR-(\d+)/)` não casa ids por path) → risco de sobrescrita de ADR.
- Ids com `.md` duplicados (dois esquemas de id).
- Reranking híbrido por *snippet* e não pelo conteúdo.
- Tags capturadas dentro de code blocks; frontmatter não-YAML (multilinha/aninhado) com reescrita que pode destruir YAML válido não suportado.
- Score sempre 0 (sem relevância real) + degradação silenciosa.
- Corrida de `deferSync` com handlers.
- `pageSize:9999` / O(n²) em `project list`.
- Plugin Obsidian apontando HTTP `localhost:3100` num sistema sem servidor HTTP.
- `zod` e `gray-matter` instalados e não usados — usar ou remover.

## 19. DESCONHECIDOS (perguntas para o produto antes da arquitetura Rust)

- Qual a fonte de verdade após a migração (vault é sincronizado por Syncthing)? Implicação de segurança.
- É aceitável o MCP continuar bloqueando o processo principal (sync fire-and-forget)?
- `second_brain_exec` é requisito? (permite `rage`, `aws`, `docker`, `git`...) Quais comandos são legítimos?
- Qual o transporte esperado do plugin Obsidian (HTTP vs MCP)? — não existe hoje.
- O usuário usa o CLI com frequência, ou só o MCP? (indica prioridade de paridade)
- Há expectativa de rodar em Windows, Linux ou ambos?
- Backlinks/grafo devem ser "eager" (recalculados no sync) ou "lazy"?
- A busca precisa de ranking real/FTS? Em português com acentos?
- O vault pode conter YAML avançado no frontmatter que hoje é destruído?
- Quais são as metas de performance (vault atual: 389 notas)?

## 20. MATRIZ INICIAL DE PARIDADE (fase 7)

| Funcionalidade | TS | Rust | Teste de caracterização | Paridade |
|---|---|---|---|---|
| CLI (25+ comandos) | ✓ | ⬜ | catálogo F1-F24 | — |
| MCP (16 tools + 3 resources + 2 prompts) | ✓ | ⬜ | contratos §6 | — |
| Parser (frontmatter/tags/wiki/headings/codeblocks/callouts) | ✓ | ⬜ | 14 testes ts como spec executável | — |
| Nota → frontmatter (toMarkdown) | ✓ | ⬜ | snapshots | — |
| Indexação/upsert | ✓ | ⬜ | re-sync em vault de teste | — |
| Busca keyword/semântica/híbrida | ✓ | ⬜ | corpus fixture (validar score) | — |
| Criação/edição | ✓ | ⬜ | create → read → restart | — |
| Backlinks | ✓ | ⬜ | banco link_edges | — |
| ADR/Project/Session | ✓ | ⬜ | fluxos MCP | — |
| Grafo | ✓ | ⬜ | 11 testes ts | — |
| Watch/reflect/study | ✓ | ⬜ | e2e simulada | — |
| Persistência `.db`/`.md` | ✓ | ⬜ | read-only real + fixture | — |
| `exec`/backup/open/obsidian | ✓ | ⬜ | portável? | — |

---

## NÃO VALIDADO nesta fase

- Execução de `tests/integration/repository.test.ts` e `tests/unit/embedding.test.ts`.
- `npm run build` (escreveria em `dist/`) e `npm audit` (rede).
- Comportamento em execução do MCP (sem spawn do processo).
- Saída de todos os comandos CLI individualmente.

**Nenhuma alteração foi feita ao projeto durante a produção deste baseline** (salvo este próprio documento).