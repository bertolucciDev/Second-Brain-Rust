# SECOND BRAIN — RUST ARCHITECTURE DECISION SPEC

> Artefato da fase **ARCHITECTURE** do gate de migração TS→Rust.
> Fontes: Contexto de Produto (normativo) + `docs/legacy-baseline.md` (factual).
> Nenhum código foi implementado. Itens sem requisito suficiente estão marcados **DECISÃO PENDENTE DO PRODUTO**.
>
> **FREEZE 2 aplicado** (gate de revisão pós-1): emendas 1–12 incorporadas — modelo de consistência multi-processo, garantias G1–G3 (sem pretensão ACID FS↔SQLite), matriz de frontmatter, estados de embeddings, identidade/rename, `version` preservada, fold de acentos reclassificado como decisão de produto, gate = Linux, e lista de decisões pendentes reclassificada por fase de bloqueio. Após este freeze, mudanças entram como FIX / IMPROVEMENT / ARCHITECTURAL CHANGE / NEW FEATURE com CHANGE ID.
>
> **FREEZE 3 aplicado** (DECISION GATE 3): correções C1–C9 incorporadas a partir das decisões do produto — **Windows PRIMARY/OFFICIAL, Linux SECONDARY/OFFICIAL, macOS fora** (C1); `dbPath` default fora do vault (C2); default de busca **hybrid unificado** (C3); fold de acentos **ativado** (C3); ranking híbrido **50/50** (C4); exceção de tree CLI: `obsidian plugin` **removido** (C5); backup = **somente vault** (C6); macOS removido dos critérios (C7); pendências resolvidas (C8); nota sobre formato de id ADR (C9). Restam **3 confirmações pontuais** do produto (lista de `exec`, matriz de suporte de sync, formato de ADR) — nenhuma delas bloqueia arquitetura.

---

## EXECUTIVE SUMMARY

O Second Brain é uma infraestrutura pessoal local: **o vault Markdown é a fonte de verdade**; banco, índice, embeddings e grafo são **dados derivados e reconstruíveis**. O defeito mais grave do legado — duas conexões sql.js gravando snapshots distintos do mesmo arquivo (perda de dados após restart, provada em `/tmp/opencode/i1/i1.ts`) — é eliminado por design na arquitetura Rust: **uma única conexão SQLite nativa (WAL), escrita serializada em uma camada única (Application), e todos os derivados reconstruíveis a partir do vault a qualquer momento**. Se o banco corromper, o pior caso é um reindex; o conteúdo do usuário permanece intacto.

A migração é **reconstrução arquitetural, não port**: preserva contrato (16 tools MCP, 3 resources, 2 prompts, CLI completa, formato de nota), corrige defeitos confirmados (persistência, `link_edges` nunca alimentado, busca `score=0`, numeração de ADR, ids `.md` misturados, degradação silenciosa de embeddings, comandos Windows-only quebrados no Linux) e preserva como **decisões de produto já respondidas** tudo o que antes estava aberto (plataformas, default de busca, acentuação, ranking, backup, distribuição, Obsidian HTTP, `version`); permanecem abertos apenas 3 itens pontuais (lista do `exec`, matriz de sync de vault, formato de id ADR — ver OPEN PRODUCT DECISIONS).

**Plataformas (decisão de produto, FREEZE 3):** Windows **PRIMARY/OFFICIAL** (também era o alvo original do legado — `cmd.exe`, PowerShell, `C:\Users\...`); Linux **SECONDARY/OFFICIAL**; macOS **fora do suporte oficial**.

Bibliotecas escolhidas com justificativa: `rusqlite (bundled-full, FTS5+WAL)`, `rmcp` (SDK MCP oficial), `pulldown-cmark` + extração própria de wiki-links/tags/callouts, `serde_yml` para frontmatter com preservação round-trip, `notify` + debounce 250ms, `ort`/ONNX para embeddings locais, `clap`, `tracing` (stderr). Nada implementado nesta fase.

---

## LEGACY → TARGET DECISIONS

Legenda: **A** = preservar · **B** = preservar contrato, reimplementar · **C** = corrigir durante migração · **D** = decisão de produto · **E** = remover.

| ID | Funcionalidade | Decisão | Motivo |
|----|----------------|---------|--------|
| F1 | Busca keyword | **B** | Contrato preservado; passa de LIKE/`score=0` para FTS5 real com bm25 |
| F2 | Busca semântica | **B** | Contrato preservado; runtime de embeddings trocado (ONNX local) |
| F3 | Busca híbrida | **B** | Contrato preservado; fusão **50/50** (bm25 normalizado + cosseno sobre conteúdo completo) — decisão de produto; corrige rerank-por-snippet |
| F4 | similar | **B** | Mesma correção de embeddings/rank |
| F5 | Ler nota | **B** | Contrato preservado; id canônico (C) elimina falha de lookup |
| F6 | Criar nota | **B** | Contrato preservado; `.md` com rename atômico (fsync); derivados reconstruíveis; conexão única. **Sem atomicidade cruzada FS↔SQLite** (ver CONSISTENCY) |
| F7 | Editar nota (upsert por create) | **B** | Preserva create-como-upsert; adiciona edição sem tocar frontmatter |
| F8 | Backlinks/grafo | **C** | `link_edges` passa a ser derivado corretamente no sync (defeito #2) |
| F9 | Contexto RAG | **B** | Contrato SOURCE/TITLE/RELEVANCE preservado; relevância real |
| F10 | ADR | **C** | Numeração segura (defeito de sobrescrita) |
| F11 | Project | **A** | Comportamento válido |
| F12 | Stats | **A** | Válido |
| F13 | Graph | **C** | Arestas corretas (depende de F8); métricas preservadas |
| F14 | Sync | **C** | Persistência corrigida; contrato `{scanned,indexed,removed,errors,durationMs}` preservado |
| F15 | Reindex | **B** | Agora = reconstrução de derivados a partir do vault (idempotente) |
| F16 | Watch | **B** | `notify` + debounce 250ms (contrato preservado) |
| F17 | Reflect | **A** | Válido (corrigir apenas slots de escrita) |
| F18 | Doctor | **B** | Mantém espaço de diagnóstico; adiciona verificação de derivados |
| F19 | Backup | **C** | Contrato `-o` preservado; **conteúdo = somente vault Markdown** (decisão de produto — o legado incluía `index.db`, derivado que não é fonte); `.zip` nativo cross-platform (elimina PowerShell) |
| F20 | Open no Obsidian | **B** | URI `obsidian://` preservada; abre via `xdg-open`/`open`/`start` |
| F21 | obsidian detect/plugin/graph | **E (plugin) / A-B (detect, graph)** | **Decidido (produto, FREEZE 3):** manter `detect`/`graph export`/`open`; **remover `obsidian plugin install`** (gerava cliente p/ HTTP `:3100`, servidor nunca existiu). Transporte HTTP futuro = NEW FEATURE |
| F22 | Study link | **B** | Mantém regra; execução eager no sync |
| F23 | Config | **B** | Mesmas chaves; schema validado (o legado nunca validou) |
| F24 | exec | **D (direção decidida)** | Ver SECURITY — **direção fechada (produto): configurável + allowlist**; tool desabilitada até configurada; lista de comandos pendente (bloqueia P8) |
| MCP tools/resources/prompts | | **A (contrato)** | Nomes/params/estruturas preservados; execução dos handlers corrigida |
| CLI comandos | | **A (contrato)** | Nomes/flags preservados; lógica movida de handlers para Application |
| ID de nota (path sem .md) | | **C** | Canônico; `.md`-sufixo eliminado por reconstrução (derivados reintegram) |
| Frontmatter | | **C** | Round-trip preservativo (ver VAULT) |
| Degradação silenciosa de embeddings | | **C** | Vira degradação **visível** (status em logs/stats) |

Nenhum item foi classificado como **E** integralmente sem justificativa: apenas `obsidian plugin install` (geração do cliente HTTP `:3100`) é removido — **decisão de produto** (FREEZE 3), com mudança de contrato registrada na paridade como `DIFERENÇA INTENCIONAL` + CHANGE ID.

---

## ARCHITECTURE

Dependência invertida em relação ao legado (corrige a violação CLI→Infra / MCP→Infra):

```text
                 ┌────────────┐   ┌────────────┐
                 │    CLI     │   │    MCP     │   ← entry points (binários; sem lógica de negócio)
                 └─────┬──────┘   └─────┬──────┘
                       │                │
                       └───────┬────────┘
                               ▼
                        ┌──────────────┐
                        │ APPLICATION  │  ← casos de uso, transações, orquestração (crates/core)
                        └──────┬───────┘
                               ▼
                        ┌──────────────┐
                        │    DOMAIN    │  ← entidades/VO puras (std only)
                        └──────┬───────┘
                               ▲
                    ┌──────────┼───────────┐
                    │ (ports)  │           │
               ┌────┴────┐ ┌───┴────┐ ┌────┴────┐
               │ VAULT  │ │ STORAGE│ │ SEARCH  │   ← traits definidos no core
               │(infra) │ │(infra) │ │(infra)  │     implementações em crates/infra
               └─────────┘ └────────┘ └─────────┘
```

- **Regra de dependência:** `cli` e `mcp` só conhecem `core` (Application pública), nunca infra diretamente. `infra` implementa traits (ports) que o core define; core não importa infra.
- **Regra de escrita:** todo caminho que altera `.md` ou banco passa pelo **Application** → **Store** (único proprietário da conexão). Não há segundo dono da conexão (elimina a raiz do bug R1).
- Validação do diagrama proposto pelo gate: o diagrama obrigatório (CLI/Application/Domain/Vault/Storage/Search/MCP) **é adequado com uma adaptação**: o MCP não pode ficar "embaixo" como se fosse adaptador de Storage; ele é **entry point irmão da CLI** (diagrama acima corrige isso sem mudar o princípio). Vault/Storage/Search permanecem no mesmo nível de adapters.
- Módulos (dentro dos crates): `config`, `note`, `parser`, `frontmatter`, `ingest` (sync/watch), `search`, `embed`, `graph`, `adr`, `project`, `session`, `store` (persistência), `obsidian`, `backup`, `stats`, `mcp_api` (handlers), `cli_api` (parsing de comandos → chama Application), `runner` (`exec`: port **`CommandRunner`** no core + adapter `std::process` na infra — evita MCP→infra direto, violação do legado `server.ts:786`).

---

## DOMAIN

Entidades **preservadas** do legado (sem criar novas "apenas para parecer útil"):

```text
Note(id, path, title, content, tags, wikiLinks, projectId?, metadata(createdAt, updatedAt, version), frontmatter)
ADR(id, title, context, problem, solution, status, alternatives, consequences)
Project(id, name, description)
Session(summary, tasks, decisions)
KnowledgeGraph(nodes, edges) + métricas puras (density, averageDegree, connectedComponents, centrality)
VOs: NoteId, Tag (normaliza), WikiLink(alvo+alias), ProjectId, SessionId, Frontmatter, Metadata(Timestamp, version)
```

Regras adicionadas (decisões de design, não requisito inventado):

- **NoteId canônico = caminho relativo sem `.md`**, rejeitando ids com `.md` na criação (correção do esquema duplicado). Semântica completa de rename/move/delete/links em **IDENTITY**.
- `version` **preserva a semântica legada** (default 1 — no banco real é sempre 1, ver baseline). Incrementar em edição seria mudança de contrato sem requisito observado → classificada como **NEW FEATURE** com CHANGE ID, fora desta migração (FREEZE 2).
- Métricas de grafo seguem **puras e testáveis** como no legado (11 testes viram spec).

Diferença intencional: nenhuma entidade nova; `Source`/`Index`/`SearchResult` não viram entidades de domínio — são tipos de Application/contrato (no legado eram DTOs).

---

## IDENTITY

**Validação do FREEZE 2:** `NoteId = caminho relativo sem .md` é a única escolha consistente com "o vault é fonte de verdade e não pode ser modificado" — um UUID exigiria gravar metadado no vault, o que a proteção do vault proíbe. Id derivado de path é reconstruível a partir do próprio vault (invariante G3).

| Situação | Semântica decidida |
|---|---|
| rename/move | id **muda** (derivado do path); derivados reindexados (evento `rename` → delete+insert; sem evento → full rescan); `[[antigo]]` em outras notas **não é reescrito automaticamente** (reescrever = alterar vault alheio, proibido) → `link_edges.target_id = NULL` + exposto em `doctor`/`stats` |
| delete | derivados removidos na mesma transação; rescan confirma |
| ids legados com `.md` | normalização na **entrada** (strip `.md` em todo input `path`/`id`); os 28 rows divergentes somem por rebuild (é derivado, não migração silenciosa) |
| compatibilidade MCP | `second_brain_read`, `backlinks`, `graph` aceitam **ambos** os formatos e resolvem igual (comportamento do legado preservado) |
| melhoria futura | resolver rename antigo via `aliases` do frontmatter → `IMPROVEMENT` (CHANGE ID), não faz parte da migração |

---

## VAULT

Fonte de verdade: **arquivos `.md`**. Responsabilidades do adapter `Vault`:

- **Leitura:** scan via glob de `**/*.md`; ignorar `.memoryos/`, `.trash/`, `Templates|templates` (contrato F14 preservado); ler por `fs::read`.
- **Escrita (criar/editar):** `write temp → fsync → replace-atômico` **cross-platform** (no Windows `rename` falha se o destino existe — requisito PLATFORM; teste de overwrite no aceite 14); **não reescrever o arquivo se o conteúdo não mudou**; frontmatter intocado em edições que não o afetam.
- **Concorrência:** nenhuma escrita externa é controlada (Obsidian/Syncthing escrevem lado a lado) — tratamos via detecção, não lock. Sync usa fingerprint `mtime+len` (e hash opcional) para idempotência.
- **Detecção de mudanças:** watcher (`notify`) com debounce 250ms (contrato legado); eventos `rename`/`remove`/`create`/`modify` mapeados a `upsert`/`delete`/`re-scan`.
- **Renomeado:** evento `rename` gera delete+insert; fallback = full rescan leve (reconciliação de órfãos), mesmo algoritmo do sync — barato em 389 notas.
- **Links quebrados:** `[[alvo]]` sem nota correspondente **não** é erro; vira `link_edges.target_id = NULL` e é exposto em `doctor`/`stats` (comportamento do legado de não quebrar por isso é preservado).
- **Arquivos externos / removidos:** sincronização remove derivados (não apaga `.md`); `remove` do vault real só por ação explícita do usuário. Regra do gate mantida: **nunca alterar o vault real em testes** — fixtures/snapshots.

---

## FRONTMATTER MATRIX (round-trip)

Requisito: **não destruir nada que o sistema não tenha editado.**

**Preservar sempre:** chaves desconhecidas, ordem das chaves, comentários, estilo de lista (flow vs bloco), strings multilinha (`|`/`>`), âncoras/aliases YAML, datas como escritas, tipagem original.
**Pode normalizar:** apenas os campos efetivamente editados (`tags`/`links`/`project`/`title`), e somente quando a edição mínima linha-a-linha for impossível.

| Situação | Comportamento |
|---|---|
| FM inalterado, conteúdo editado | **reutiliza o raw block byte a byte** |
| nota nova (create) | serialização canônica no formato legado (`k: "v"` / `k: ["a"]`) |
| edição só de tags/links | edição mínima das linhas dessas chaves; se falhar → reescrita preservando as demais |
| campo desconhecido | preservado no raw; copiado em qualquer reescrita |
| YAML inválido | lê como **opaco** (raw preservado); aviso em `doctor`; leitura/conteúdo funcionam; **edição de frontmatter recusada com erro claro** (nunca "conserta" sozinho) |
| YAML avançado (âncoras, aninhado, multi-doc) | raw preservado; edição de FM restrita a escalares/listas top-level; detectado → erro na edição FM |
| comentários | preservados via raw; se reescrita total for inevitável → perdidos **+ aviso explícito** |
| sem frontmatter | não cria FM ao editar conteúdo (só `create` cria) |
| `---` dentro de string | fixture específica (parser de blocos) |

**Matriz de fixtures para provar a decisão** (16 × 3 ações = 48 casos; leitura / edição de conteúdo / edição de FM): (1) FM simples · (2) sem FM · (3) chaves desconhecidas · (4) comentário no FM · (5) multilinha `|` · (6) lista flow · (7) lista bloco · (8) mapa aninhado · (9) âncora/alias · (10) valor com `:` e vírgula · (11) aspas não fechadas (inválido) · (12) chaves duplicadas · (13) acentos/unicode · (14) datas como escritas · (15) `aliases` Obsidian · (16) `---` em string.

---

## PERSISTENCE

Objetivo: eliminar a causa raiz (múltiplas conexões/snapshots fantasma) e tornar o banco **descartável** (derivado).

Decisão central: **uma única conexão SQLite nativa (WAL), escrita exclusivamente pelo Application via um `Store` serializado**; banco = cache derivado, reconstruível.

- **Conexão:** `rusqlite` com feature `bundled-full` (compila SQLite completo com **FTS5** e WAL). A conexão é aberta **uma vez** no processo; `cli` e `mcp` a recebem por composição do Application — impossível "segunda conexão" acidental. Qualquer outra entrada no processo seria leitura para diagnóstico, sem `writeFileSync` de arquivo inteiro.
- **Transações:** todas as mutações no Store em transações `immediate`; escrita no `.md` ocorre **antes** do commit dos derivados (o `.md` é fonte; se o banco falhar, os derivados são reconstruíveis — não há perda de conhecimento).
- **WAL** + `synchronous=NORMAL`: gravações incrementais, sem reescrita do arquivo inteiro (o defeito do legado não terá nem o mecanismo).
- **Recuperação/corrupção:** `PRAGMA integrity_check` leve + `schema_version` no startup; em caso de falha: `doctor` oferece **rebuild** (delete+recria derivados a partir do vault). *(Não confundir com o comando `backup` da CLI, que é somente-vault — ver F19.)*
- **Rebuild do índice:** um único comando `reindex` idempotente reconstrói `notes` (derivado de metadados), `notes_fts`, `note_embeddings`, `link_edges`.
- **Por que evita o problema legado:** (1) não existe export+write do arquivo inteiro; (2) um único dono de escrita; (3) WAL garante atomicidade parcial; (4) se algo corromper, a restauração é "reindexar", não "recuperar snapshot".

Fonte de verdade × derivados:

```text
FONTE:  vault/*.md
DERIVADOS: notes(metadados) · notes_fts · note_embeddings · link_edges · stats · graph(calculado)
RECONSTRUÇÃO: sync/reindex → reparseia tudo a partir do vault → regrava derivados (idempotente)
```

---

## CONSISTENCY & MULTI-PROCESS POLICY (FREEZE 2)

**Garantia formal (é o que o sistema promete — não promete ACID entre filesystem e SQLite):**

- **G1** — Após retorno de sucesso, o `.md` está durável (fsync do arquivo **e** do diretório) e visível.
- **G2** — O sistema **nunca** perde, corrompe ou sobrescreve conteúdo do vault por ação própria (só rename atômico; nunca overwrite baseado em conteúdo desatualizado).
- **G3** — Derivados (`notes`, `notes_fts`, `note_embeddings`, `link_edges`, stats) podem estar **defasados ou perdidos**, mas são **sempre reconstruíveis** a partir do vault. Worst case = `reindex`, nunca perda de conhecimento.

**Ordem de escrita:** `write .md (temp → fsync → rename) → commit SQLite`. Nunca o inverso: derivado ausente se repara sozinho; nota fantasma no índice (arquivo inexistente) é pior.

**Casos de queda:**

| Queda | Resultado | Reparo |
|---|---|---|
| antes do fsync | temp descartado; nada visível | operação não reportada como sucesso |
| após fsync, antes do commit | `.md` durável, linha ausente | reconciliação de startup reindexa |
| durante o commit | WAL: transação completa ou nenhuma (garantia do SQLite) | — |
| `.db` corrompido | `integrity_check` falha | descartar derivados + rebuild do vault |
| `.db` inexistente/desatualizado | — | criação/rebuild + fingerprint no startup |

**Política multi-processo (7 respostas):**

1. **WAL é suficiente?** Só para o banco (locks de arquivo, leitura concorrente, atomicidade de cada transação). **Não** resolve rebuilds concorrentes, atomicidade FS↔SQLite nem duas escritas na mesma nota.
2. **Lock interprocesso?** **Obrigatório para mutar.** Lock de arquivo advisory (`<db>.lock` + PID + heurística de processo vivo) adquirido por **todo processo que muta** (sync, reindex, create, watch tick). Leitura sem lock. Não obtido → espera `busy_timeout`; processo vivo não cede → erro claro (não inicia segundo escritor).
3. **Lock do vault?** **Nunca.** Obsidian/Syncthing não saberiam que ele existe → deadlock/perda de sync. Conflito é detectado, não impedido.
4. **Quem escreve?** `.md`: qualquer processo do Second Brain (rename atômico) **e** processos externos (não-controláveis). Banco: **apenas processos do Second Brain, um por vez**; externos nunca tocam o `.db`.
5. **Detecção de conflito externo:** fingerprint por nota (`mtime + size + sha256`); antes de commit de edição, hash on-disk ≠ hash lido → **aborta com conflito** (não sobrescreve). Re-scan no startup + debounce detecta mudanças externas; rename sem evento (Syncthing puxando de outra máquina) → full rescan reconcilia.
6. **Divergência vault × banco:** o banco **sempre é reconstruído** do vault. Nenhum caminho "corrige" o vault a partir do banco — proibido.
7. **Estado que sempre vence: o VAULT.** Invariante central da arquitetura.

**Local do banco (DECIDIDO — produto, FREEZE 3):** `dbPath` é **configurável** e o **default fica fora do vault** (banco é derivado; vault pode ser sincronizado). *Semântica de migração (recomendação técnica congelada):* `dbPath` explícito em `memory.config.json` **é respeitado** (compatibilidade) com **aviso no `doctor`** se apontar para dentro do vault; ausente → resolve o data dir da plataforma (Windows primário: `%LOCALAPPDATA%\Second Brain\index.db`; Linux secundário: `$XDG_DATA_HOME/second-brain/index.db`). Se mantido dentro do vault → orientar `.stignore` de `.memoryos/`. Mudar o local custa apenas um reindex (é derivado).

**Riscos derivados:** leitura de `.md` parcialmente escrito por writer externo → parse falha = retry pós-debounce, nunca erro fatal; MCP longo segurando lock → transações curtas + `busy_timeout`; WAL sem checkpoint → checkpoint no shutdown; dois binários com config divergente → config types no core + teste (R-A8).

---

## INDEX

- **Quando indexar:** em `sync` (full), em `watch` (delta por nota), em `create/update` (nota única) — sempre pelo mesmo caminho Application→Store.
- **Quando remover:** nota deletada do vault (evento/scan) → remover nota e derivados na mesma transação.
- **Reindexar:** `reindex [--force]` reconstrói tudo (default: só o que falta, como no legado).
- **Detecção de alterações:** fingerprint `mtime+len`; `--force` ignora fingerprint.
- **Falhas:** transação faz rollback; notas com erro de parse entram em `errors[]` do `SyncResult` (contrato F14), nunca abortam o sync inteiro (comportamento preservado).
- **Embeddings indisponíveis:** degradação para FTS-only **visível** — nota marcada como `unembedded`, `stats`/`doctor` reportam débito, log em stderr (correção do comportamento silencioso do legado).
- **Idempotência:** objetivo explícito — rodar sync/reindex N vezes produz o mesmo estado (teste de caracterização).

---

## SEARCH

Contrato preservado (limit, offset, filtros tags/links/project, estratégias `keyword|hybrid|semantic`, shape `SearchResult{noteId,score,matchedFields,snippet}` + paginação). Rank real, nunca `score=0`.

- **Default unificado (decisão de produto, FREEZE 3):** `strategy` default = **`hybrid`** em CLI, MCP e config — resolve a divergência legada (CLI `keyword` em `cli.ts:47` × MCP `hybrid` em `server.ts:48`; a config real do usuário já era `hybrid`). Mudança de comportamento da CLI → paridade classificada `DIFERENÇA INTENCIONAL` + CHANGE ID.
- **Keyword:** FTS5 `unicode61` (título/conteúdo/tags) com `bm25()`; snippet via conteúdo real.
- **Acentuação (decidido — produto, FREEZE 3):** **fold de acentos ATIVADO** em todos os caminhos (FTS e fallback LIKE — o legado quebrava aqui: strip ASCII do query em `MemorySearchIndex.ts:355`, LIKE sem fold em `:367`). CHANGE ID registrado; teste: `café` ↔ `cafe` em keyword/semantic/hybrid.
- **Semantic:** embedding da query e das notas (mesmo pipeline de pooling do MiniLM), cosseno; scan linear como fallback óbvio em 389 notas (sem índice vetorial nesta fase — sem otimização especulativa).
- **Hybrid (decidido — produto, FREEZE 3):** fusão **50/50** — `score = 0.5·norm(bm25) + 0.5·cos(consulta, conteúdo_inteiro)`; ambos normalizados antes de combinar; `score` permanece em 0..1 (não quebra consumidor). Pesos ajustáveis por config **sem** mudar o shape do contrato (revisão futura preservada). Substitui o rerank-por-snippet do legado (`MemorySearchIndex.ts:259-268`) — este defeito é corrigido pela fusão sobre conteúdo completo.
- **Filtros/paginação:** filtros como restrições na query (não mais `SELECT *` + `JSON.parse` por linha — elimina N+1).
- **Fallback e erros:** sem FTS5 (build sem `bundled-full`) → fallback LIKE **explícito e reportado**; sem modelo/embeddings → retorna keyword com `degraded=true` no resultado (aditivo, não quebra consumidor atual).

---

## EMBEDDINGS & DEGRADATION STATES (FREEZE 2)

Decisões: embeddings são **opcionais/degradáveis** (o sistema é 100% funcional sem eles — FTS-only); **substituíveis** (port `Embedder`; modelo configurável por caminho local; **sem API externa** — privacidade); modelo **baixado uma única vez** com checksum para o data dir (nunca a cada execução — mitiga o "hang do HuggingFace" do legado). Empacotar vs baixar = decisão de produto (não bloqueante — isolada pelo port).

| Estado | Comportamento |
|---|---|
| modelo disponível | indexa e busca `semantic`/`hybrid` normalmente |
| modelo indisponível (não baixado, offline) | sync **continua**; nota marcada `unembedded`; busca degrada para keyword; `stats`/`doctor` reportam o débito; aviso em stderr |
| runtime indisponível (ort não carrega / arch) | idem, log `runtime_error`; busca keyword |
| modelo corrompido (checksum) | 1× re-download; falha de novo → keyword |
| embedding falha (OOM/timeout) | retry 1× por nota; falha por nota **nunca aborta o sync** (entra em `errors[]` do `SyncResult`) |

**Fallback FTS verificável (obrigatório em teste):** respostas de `search`/`similar` carregam campos aditivos `degraded` + `strategy_used`; teste automatizado = rodar sem o diretório do modelo → esperar resultados keyword, `degraded=true`, `unembedded > 0`, exit 0.

---

## GRAPH

- **Estratégia: eager durante sync** (389 notas; custo trivial). Arestas derivadas: `outgoing` = wiki-links parseados; `link_edges` reconstruído **integralmente** a cada sync (`DELETE` + `INSERT`, com `UNIQUE(source_id,target)`), repondo a falha histórica do `upsertMany`.
- **Backlinks** = consulta inversa sobre `link_edges` (não armazenar cópia).
- **Centralidade/componentes/density** = cálculo sob demanda no `graph` (puro, testado — preserva os 11 testes do legado).
- **Notas órfãs / links quebrados:** `target_id NULL` p/ alvo inexistente; exposto em `doctor`/`stats`.
- Justificativa (respondendo §12 do gate): eager porque o volume é pequeno e a correção elimina a classe de bug; lazy só compensaria com vaults ≥10⁴ notas (métricas futuras podem reavaliar — anotado em ADR, não implementado).

---

## SYNC

- Pipeline único reutilizado por `sync`, `watch` e startup: **scan → parse → upsert derivados → reconciliar órfãos → (eager) study-link → derivados de grafo**. Retorna `SyncResult{scanned,indexed,removed,errors,durationMs}` (contrato).
- Correção: após o pipeline, **o banco em disco é igual ao estado do vault** (o teste I1 vira teste de integração: rodar sync, reler do disco, assertar consistência).
- Startup do MCP: o `deferSync` fire-and-forget do legado (que mascava transações) é substituído por **sync inicial em fila única no Store**: nenhum handler muta enquanto o sync corre; leituras podem ocorrer em estado anterior com flag `indexing=true` em `info`.
- Watch/reflect/study operam pelo mesmo Application/Store — sem handlers tocando `repo`/`index` diretamente.

---

## CLI

- Framework: `clap` (derive). **Tree de comandos idêntica à §5 do baseline com UMA exceção formal (FREEZE 3):** `obsidian plugin <install|...>` **removido** (decisão de produto — não reproduz o cliente HTTP `:3100`); demais comandos preservados (init, search, read, create, sync, reindex, graph, watch, doctor, stats, reflect, mcp, adr, session, project, config, backup, open, obsidian detect/graph, study) — flags preservadas; `--json` onde existia.
- **Defaults alterados por decisão de produto (registrados na paridade):** `search` sem `--strategy` → `hybrid` (era `keyword`); `backup` → conteúdo somente-vault (era vault + `index.db`).
- **Handlers finos:** `cli_api` só converte args → chama Application; nenhuma regra de negócio (numeração de ADR, `resolveTargetId`, transações) vive na CLI (correção do legado).
- Exit codes formalizados (0 sucesso / 1 erro / 2 invariante do vault) apenas se produto aprovar (default: manter 0/1).
- `mcp` continua subcomando que inicia o servidor MCP em stdio.

---

## MCP

- Biblioteca: **`rmcp`** (SDK Rust oficial) sobre stdio. Transporte preservado (clientes existentes não mudam).
- **Múltiplos clientes (decisão de produto, FREEZE 3):** arquitetura independente de cliente — nenhum código do Core/infra conhece cliente (Claude, ChatGPT, Cursor, VS Code ou outros). Evidência: o legado já é neutro (`capabilities: {tools,resources,prompts}` em `server.ts:83`; sem `clientInfo`/`sampling`/`roots`). Validação em P8: e2e com **≥2 clientes MCP distintos** + Inspector (conformidade de protocolo).
- **16 tools com nomes, parâmetros e shapes de retorno idênticos** (tabela §6 do baseline vira spec de teste). Resources `second-brain://notes|stats|config`; prompts `context_summary`/`search_and_read`.
- Correções contratuais (não quebram cliente): validação de schema em todos os handlers (o legado nunca validou); `id`/`path` normalizados para o NoteId canônico (`second_brain_read` aceita ambos e resolve igual); erros continuam `{error: msg}`.
- `second_brain_exec` tratado em SECURITY.
- Os dois prompts usam o mesmo pipeline de contexto (top-N com SOURCE/TITLE/RELEVANCE; truncamento 500 chars no `context_summary` — contrato preservado).

---

## SECURITY

Modelo de ameaça assumido e declarado: **processo local; agente pode ser não-confiável; ferramentas têm privilégio de processo.** Nada secreto decidido nesta fase.

Análise para `second_brain_exec` (respondendo §15 do gate):

| Alternativa | Segurança | Compatibilidade | UX | Complexidade |
|---|---|---|---|---|
| 1. shell arbitrário (legado) | Ruim | Alta | Alta | Nenhuma |
| 2. allowlist de comandos | Boa | Média (exige config) | Média | Baixa |
| 3. capabilities declarativas (arg-schema por comando) | Boa | Média | Boa | Média |
| 4. desabilitado por padrão + enable explícito | Boa | Baixa (default muda) | Média | Baixa |
| 5. remover | Máxima | Baixa | Perde `rage/aws/git` | Nenhuma |
| 6. mecanismo separado (tool só dispara script registrado) | Alta | Média | Média | Média |

**Direção decidida (produto, FREEZE 3): alternativas 4 + 2 combinadas — configurável + allowlist.** Tool presente mas **desabilitada por default** (sem allowlist configurada → erro explícito); quando habilitada, **allowlist** com paths absolutos resolvidos (sem `PATH` legado hardcoded `C:\Users\luiso\...`, sem `cmd.exe/c` livre), timeout por comando (default 30s, max via config), **sem `env` injetável pelo chamador** (o legado aceitava `a.env` — `server.ts:782`), cwd = vault, args livres somente dentro do permitido.

**Pendência de produto (não arquitetura) — bloqueia apenas P8:**
1. Confirmar a lista de comandos candidatos e seus subcomandos: `git`, `docker`, `aws`, `rage`, `vercel` (candidatos = descrição do próprio legado `server.ts:252`; uso real **UNKNOWN** — sem telemetria).
2. Confirmar quais precisam rodar **no Windows** (primário) e se argumentos podem ser livres dentro de cada entrada.
3. Allowlist por executável; subcomando restrito só se produto pedir (complexidade média).
4. Remoção (alternativa 5) **sai do rol de pendências**: mudaria o contrato congelado de **16 tools** → classificada como **ARCHITECTURAL CHANGE** (exige re-freeze). A tool permanece presente, desabilitada por default, com allowlist vazia até a lista ser definida; a pendência se resume ao item 1 (lista).

Outras decisões:
- **Segredos:** o índice **nunca** indexa conteúdo de paths sensíveis configuráveis (`Credentials/`, `**/*.age`, `sudo-password.*`) por default (comportamento: bloqueio por padrão, config com schema); `read/search/context` não expõem esses caminhos. Correção do risco §15.4 do baseline.
- **Não SQLi:** todas as queries parametrizadas (bind) — comportamento já presente no legado, preservado.
- **Nada de secrets no binário/config/repo:** `.gitignore` equivalente; config validado por schema (sem `cwd` arbitrário decidindo `dbPath`/`vaultPath` sem validação).
- Logs: `tracing` → **stderr** (preserva o requisito do canal stdio MCP).

---

## OBSIDIAN

- **Permanecer (decidido — produto, FREEZE 3):** `obsidian detect`, `obsidian graph [-o]` (export JSON), `open <path>` via URI `obsidian://` (cross-platform).
- **REMOVER (decidido — produto, FREEZE 3):** `obsidian plugin <install|...>` e a geração do cliente que presume HTTP `localhost:3100`. Evidência: o servidor **nunca existiu** no legado → a funcionalidade nunca funcionou → não há usuário dependente. **Não reproduzir `:3100`.** Mudança de contrato registrada na paridade como `DIFERENÇA INTENCIONAL` + CHANGE ID; não é mais pendência.
- **Integração real:** Obsidian escreve `.md` → watcher detecta (cobre 100% do fluxo). Qualquer **novo transporte HTTP** exige **decisão futura específica** (`NEW FEATURE`, `axum` + auth token) — não faz parte desta migração.
- **MCP no lugar do plugin:** o MCP cobre o uso declarado (consulta por agentes); nenhum requisito do plugin HTTP permanece.

---

## PLATFORM

| Recurso | Windows (PRIMARY/OFICIAL) | Linux (SECONDARY/OFICIAL) | macOS |
|---|---|---|---|
| Core/CLI/MCP/Storage/Search | **Gate de CI obrigatório**; release prioritário | **CI de build+testes obrigatória**; artefato de release compatível | **FORA DO SUPORTE OFICIAL** — sem CI, sem release, sem garantia (nenhuma menção de suporte em docs) |
| Watch (`notify`) | suportado (`ReadDirectoryChangesW`) | suportado (inotify) | não prometido |
| Embeddings ONNX (`ort`) | suportado | suportado | não prometido |
| `backup` / `open` | `.zip` nativo (sem PowerShell); `open` via `start` | `.zip` nativo; `open` via `xdg-open` | não prometido |
| Testes de CI | **obrigatório (gate primário)** | **obrigatório (gate secundário oficial)** | nenhum |

Caminhos hardcoded `C:\Users\luiso\...` e `cmd.exe` do legado **não são reproduzidos** — mas o **alvo original era Windows** (CONFIRMADO), o que torna Windows a plataforma primária uma continuidade de requisito, não uma inversão arbitrária.

**Acceptance gate (FREEZE 3):** **Windows é a plataforma PRIMÁRIA/OFICIAL** (CI obrigatória, release prioritário); **Linux é SECONDARY/OFICIAL** (CI obrigatória e distribuição compatível — não "compatibilidade teórica"); **macOS fora do suporte oficial**. Consequências de "secondary/official" para Linux: build + testes automatizados na CI, artefato publicável, docs com instruções reais, e os mesmos critérios de aceite aplicáveis (sem exceções).

**Requisitos técnicos derivados da plataforma primária (Windows) — congelados:**
- **Escrita atômica com replace:** `std::fs::rename` **falha se o destino existir** no Windows → o port `Vault` deve implementar *replace-atômico* nativo (não reescrever in-place) e ter **teste de overwrite** obrigatório.
- **Lock cross-platform:** não há `flock` no Windows → usar crate de lock advisory portável (`LockFileEx`/`fs4`-`fs2`); requisito do port `Store`.
- **Filesystem case-insensitive:** preservar o path exatamente como está no disco (sem normalizar caixa) para estabilidade do `NoteId`.
- `open`/`backup` sem depender de `start`/PowerShell além do estritamente necessário.

---

## PERFORMANCE

- Volume hoje: ~389 notas / 216 `.md`. **Sem otimização especulativa.** Derivados reconstruídos no sync (eager) são baratos nessa escala.
- Métricas a medir depois (definidas agora; usadas como critérios de aceite, não como metas antes da medida): startup do MCP até pronto; sync full (sem e com embeddings); watch round-trip (evento→índice); query keyword / semântica p95; reindex full; RSS em carga; espaço do banco (WAL).
- Crescimento: revalidar custo linear do scan semântico e decidir índice vetorial (decisão de produto/métrica, não agora).
- Embeddings: CPU batch via **rayon**; modelo cacheado em data dir (uma vez), nunca baixado a cada execução (mitiga o "hang do HuggingFace" do legado).

---

## CRATE STRUCTURE

Workspace Rust (evita microcrates; 4 crates; justificativa por crate):

| Crate | Responsabilidade | Depende de | Motivo/Benefício | Risco |
|---|---|---|---|---|
| `second-brain-core` | Domain + Application + ports (traits) + config types + contratos de resposta | std + libs puras (`serde`, `thiserror`) | Núcleo independente de adapters; testável puro; **corrige a violação de fronteira legada** | — |
| `second-brain-infra` | adapters: vault fs, parser, store sqlite, search fts, embed ort, watch notify, obsidian, backup, runner (process) | core + crates de adapter | Único lugar com I/O; trocar SQLite/embedder não toca core | Aporte de embeddings (ort/tokenizers) é o maior risco técnico |
| `second-brain-cli` | binário CLI (clap) | core, infra | Contrato CLI preservado, handlers finos | — |
| `second-brain-mcp` | binário MCP (rmcp+stdio) | core, infra | Contrato MCP preservado; processo isolado | Compatibilidade de protocolo com clientes existentes |

Alternativa rejeitada: single crate gigante (perde a fronteira core/infra, que é o ponto central da correção); e 8+ microcrates (sem benefício real nessa escala). Reavaliar se core crescer.

---

## DEPENDENCIES

| Necessidade | Candidatos | Escolha | Justificativa |
|---|---|---|---|
| SQLite | `rusqlite` (sync), `sqlx` (async), `libsql` | **`rusqlite` + feature `bundled-full`** | FTS5+WAL nativos; API síncrona casa com single-writer local; `bundled-full` garante FTS5 (elimina `no such module: fts5` do legado); `sqlx` traria async sem resolver problema |
| MCP | `rmcp` (oficial), `mcp-rs` (community), mão | **`rmcp`** | SDK oficial mantido; contratos por tipo seguro; mão = risco de protocolo |
| CLI | `clap`, `lexopt` | **`clap` (derive)** | Maduro p/ tree idêntica ao legado (subcomandos aninhados); `lexopt` é mínimo demais p/ 25+ comandos |
| Markdown | `pulldown-cmark`, `markdown`, hand-rolled | **`pulldown-cmark` (estrutura)** + lexer próprio (≤100 linhas) para `[[wiki]]`, `#tag`, callouts | Preserva semântica testada do legado (14 testes) sem reescrever parser inteiro |
| YAML/frontmatter | `serde_yaml` (arquivado), `serde_yml` (fork mantido), `yaml-rust` | **`serde_yml`** + preservação de campos desconhecidos + "não tocar raw block se não mudou" | Requisito forte é round-trip/não-destruição |
| File watcher | `notify`, `hotwatch` | **`notify`** + debounce próprio (250ms) | `hotwatch` engessa controle; legado = chokidar+250ms |
| ZIP (backup vault-only) | `zip`, `tar` | **`zip`** | Backup somente-vault em `.zip` nativo cross-platform (substitui PowerShell; sem dependência de `Compress-Archive`) |
| Lock interprocesso | `fs4`, `fs2`, mão | **`fs4`/`fs2`** | Lock advisory portável (`LockFileEx` no Windows — `flock` não existe); requisito do port `Store` |
| Embeddings | `ort`/ONNX, `candle`, `tract`, `ggml` | **`ort`** + `tokenizers` p/ o mesmo `all-MiniLM-L6-v2` | Runtime ONNX maduro/CPU; mesmo modelo do legado; `candle` é reserva se ort der problema de build (anotar em ADR). **Paridade de logits NÃO garantida**: testes comportamentais (cosseno > 0.85 em corpus fixture) |
| HTTP (opcional) | `axum`, `actix-web` | **só se decisão de produto; `axum`** | Evitar HTTP sem requisito |
| Async/runtime | `tokio` + `rayon` | **`tokio` (leve) + `rayon` (embeddings)** | MCP/watch querem async leve; embeddings são CPU |
| Erros | `thiserror` + `anyhow` | escolhidos | Padrão idiomático |
| Logging | `tracing` + `tracing-subscriber`, `env_logger` | **`tracing`** | Logs → stderr (contrato stdio MCP); spans p/ sync/busca |
| Testes | built-in `#[test]`, `assert_cmd`, `proptest`, `rstest` | escolhidos | Caracterização do legado + e2e CLI |

Nenhuma dependência de cloud/LLM remoto.

---

## MIGRATION STRATEGY

Ordem (adaptada do §26 do gate, justificada — contrato primeiro porque é a âncora de compatibilidade; domain antes de infra para testar sem I/O):

```text
1. congelar contratos (baseline → fixtures de tools/respostas)
2. testes de caracterização (rodar legado TS: snapshot de parse/busca/CLI; F1-F24)
3. core: domain puro + testes (porta: 14 parser + 11 graph + VOs do legado)
4. core: application + ports (Store/Vault/Search/Embedder traits)
5. infra: store sqlite (FTS5+WAL) + moldura de `reindex`
6. infra: parser (pulldown-cmark + lexer wiki/tag/callout) + frontmatter round-trip
7. infra: search (bm25) + embed (ort) com degradação visível
8. infra: graph (edges derivados no sync, eager)
9. infra: vault (scan/write atômica/watch 250ms)
10. sync/reindex end-to-end (idempotência + teste I1 como integração)
11. cli (clap, handlers finos, contrato de comandos)
12. mcp (rmcp, 16 tools/3 resources/2 prompts, validação de schema)
13. integração e2e (sync→search→read→create→restart→consistência)
14. paridade (galeria de cenários TS vs Rust, classificar divergências)
15. melhorias (somente após paridade e aprovação; cada uma com CHANGE ID §24 do gate)
```

Cada fase termina com: código + testes + evidência + diff de comportamento + registro em `docs/migration-log.md`. **Nada de reescrita monolítica.**

---

## PARITY STRATEGY

- Galeria de cenários executável: para cada funcionalidade, um par `{input, output}` congelado do legado TS (rodado ANTES da migração, enquanto temos o runtime) comparado ao Rust. Ferramenta de diff dedicada em `tests/parity/`.
- Classificação de divergências obrigatória (§37 do gate): `BUG | MELHORIA | COMPORTAMENTO LEGADO INDESEJADO | DIFERENÇA INTENCIONAL | DESCONHECIDO`.
- **Divergências dirigidas por decisão de produto** classificam-se como **`DIFERENÇA INTENCIONAL` + CHANGE ID**, nunca como `BUG`. Lista FREEZE 3: remoção de `obsidian plugin` (C5); default de busca **hybrid** unificado (C3); **fold de acentos ativado** e correção do strip-ASCII do query (C3); ranking **50/50** sobre conteúdo completo (C4); **backup somente-vault** (C6); remoção eventual de `exec` se a lista vier vazia (C-exec); `version` preservada; F6 (escrita sem atomicidade cruzada). Itens cuja decisão ainda não existe ficam **fora da galeria** até congelarem — a paridade não é medida contra contrato não decidido.
- **Atenção especial a paridades "não bit-a-bit":** busca (FTS5 bm25 real vs LIKE score 0), embeddings (similaridade comportamental, não vetor igual), parser (mesmo vocabulário de fixture). Divergências esperadas e **intencionais** (correções), listadas explicitamente na matriz.
- Matriz:

| Funcionalidade | TS | Rust | Teste | Resultado |
|---|---|---|---|---|
| CLI (25+ comandos) | ✓ | ⬜ | assert_cmd + golden outputs | — |
| MCP (16+3+2) | ✓ | ⬜ | e2e stdio com fixture de Request/Response | — |
| Parser (frontmatter/tags/wiki/headings/codeblocks/callouts) | ✓ | ⬜ | 14 testes legados viram fixtures | — |
| Note→frontmatter (toMarkdown) | ✓ | ⬜ | snapshots byte-a-byte (onde o legado é determinístico) | — |
| Indexação/upsert/delete idempotentes | ✓ | ⬜ | sync→sync; delete→restart (reproduz I1 como regressão) | — |
| Busca keyword/semantic/hybrid/similar | ✓ | ⬜ | corpus fixture; score real vs 0 = divergência intencional | — |
| Create/read com ids `.md` e canônicos | ✓ | ⬜ | ambos os formatos de input resolvem igual | — |
| Backlinks/grafo | ✓ | ⬜ | contagem de arestas = wiki-links parseados | — |
| ADR/Project/Session | ✓ | ⬜ | fluxos MCP (numeração via scan de `ADR-*`, sem sobrescrita) | — |
| Watch/reflect/study | ✓ | ⬜ | e2e simulada | — |
| Persistência/restart | ✓ | ⬜ | teste I1 (consistência pós-restart) | — |
| exec/backup/open/obsidian | ✓ | ⬜ | conforme decisão de produto (D) | — |

---

## OPEN PRODUCT DECISIONS

Tudo que **não** pode ser decidido tecnicamente. **FREEZE 3:** a maioria das pendências do FREEZE 2 foi **respondida pelo produto**; restam **3 confirmações pontuais**, todas bloqueadoras de **fase** — nenhuma bloqueia a arquitetura.

**Abertos (bloqueadores de fase):**

1. `second_brain_exec` — **lista** de comandos/subcomandos e plataformas → **bloqueia P8**. *Direção já decidida:* configurável + allowlist, desabilitada por default, sem `env` injetável. Candidatos do legado: `git`, `docker`, `aws`, `rage`, `vercel` (`server.ts:252`); uso real UNKNOWN.
2. **Matriz de suporte de sync de vault** → **bloqueia P9** (docs/testes). Arquitetura cobre S1/S2/S4 com `.db` local por máquina; S3 (`.db` compartilhado/sincronizado) **não suportado**. *Recomendação técnica:* oficializar S1+S2+S4, S3 proibido. *Fato a confirmar:* a outra máquina sincronizada também roda o Second Brain?
3. **Formato do id de ADR** criado pela ferramenta: preservar `ADR-0001` (**4 dígitos**, contrato legado — `server.ts:666`) ou `ADR-010` (exemplo 3 dígitos do produto). *Reclassificada:* **não bloqueia P0** — fixtures de caracterização capturam o legado (4 dígitos); só a fixture-golden de `create_adr` depende do formato (1 linha). *Recomendação:* manter 4 dígitos; regra **MAX+1** vale para ambos (sem preencher gaps).

**Resolvidos pelo produto (FREEZE 3 — não são mais pendências):** plataformas (Windows primary / Linux secondary / macOS fora); local do `.db` (default fora do vault, configurável); default de busca (**hybrid**); acentuação (**fold ativado**); ranking (**50/50**); Obsidian (sem HTTP/plugin); backup (**somente vault**); `version` (preservada); distribuição (GitHub Releases + Portable ZIP); MCP (**múltiplos clientes**); ADR (**MAX+1**); migração (parity + critical fixes → controlled improvements).

Se um dos 3 itens chegar sem resposta na fase que ele bloqueia, **a IA deve parar e perguntar** (regra §43 do gate).

---

## ARCHITECTURAL RISKS

| # | Risco | Mitigação |
|---|---|---|
| R-A1 | Driver de embeddings (ort/tokenizers vs transformers.js): paridade de similaridade não garantida bit-a-bit | teste de paridade **comportamental** (cosseno > threshold) + fallback keyword visível + ADR |
| R-A2 | Contrato MCP: clientes existentes — semântica muda? | baseline de fixtures; e2e com cliente real antes de trocar |
| R-A3 | FTS5/`bundled-full` (build) indisponível em algum target | verificar na fase de store; fallback LIKE explícito com `degraded` (decisão tomada) |
| R-A4 | Round-trip de frontmatter: normalizar demais | estratégia "não tocar raw block até que editado"; snapshots de fixtures do vault real (anonymizado) |
| R-A5 | Watch + escrita própria: loop de eventos ao reescrever `.md` | fingerprint antes/depois; ignorar eventos próprios por hash |
| R-A6 | Migração trocar arquitetura sem paridade provada → perda funcional | paridade por fase (nada mergeado sem teste) + binários `cli`/`mcp` coexistindo na transição |
| R-A7 | Custo de construir as 16 tools em rmcp | mapeamento 1:1 a partir de fixtures; validação de schema compartilhada |
| R-A8 | Dois binários divergirem em config/comportamento | config types no core; testes de contrato em CI para ambos |
| R-A9 | **Windows primário**: `fs::rename` falha sobre destino existente; `flock` inexistente | port `Vault` com replace-atômico + teste de overwrite em Windows; lock portável (`LockFileEx`/`fs4`); CI `windows-latest` como gate |
| R-A10 | `.db` dentro de vault sincronizado (config legada explícita) → WAL replicado = corrupção | default fora do vault; `doctor` avisa se `dbPath` dentro do vault; S3 declarado não suportado |
| R-A11 | Paridade confundir melhorias decididas com regressões (hybrid default, fold, 50/50, backup-only-vault) | CHANGE IDs registrados **antes** de rodar a galeria (coluna PARITY) |
| R-A12 | Colisão/sobrescrita de ADR (regex legado × id canônico) | MAX+1 sobre padrão `ADR/ADR-\d{4}`; teste de duas criações consecutivas (P0/P7) |

---

## REQUIRED ADRs

Criar antes da implementação (formato Contexto/Problema/Opções/Decisão/Justificativa/Consequências/Alternativas):

1. **ADR-Art-001** — Arquitetura em camadas com inversão de dependência (core/infra/cli/mcp; razão para não reproduzir `CLI→Infra`).
2. **ADR-Persist-002** — `rusqlite` + `bundled-full` (WAL, FTS5, conexão única, banco derivado descartável). Eliminação da dupla conexão do legado.
3. **ADR-Id-003** — Identidade canônica `NoteId = caminho sem .md`; reconciliação via rebuild, não migração de dados.
4. **ADR-Front-004** — Frontmatter round-trip (não tocar raw + serde_yml; edição mínima quando necessário).
5. **ADR-Busca-005** — FTS5 unicode61 + bm25; **default híbrido unificado**; **fold de acentos ativado**; fusão **50/50** (bm25 normalizado + cosseno sobre conteúdo completo), pesos ajustáveis sem quebrar shape; `score=0` nunca.
6. **ADR-Embed-006** — ONNX/ort + tokenizers, MiniLM, cache local, degradação visível; paridade comportamental.
7. **ADR-Graph-007** — Arestas derivadas eager no sync com `UNIQUE(source_id,target)` (repõe `upsertMany`).
8. **ADR-Seg-008** — Paths sensíveis não indexados + modelo de ameaça local; `exec` **configurável + allowlist** (desabilitada por default, sem `env` injetável) — direção decidida, lista de comandos pendente (P8).
9. **ADR-MCP-009** — `rmcp`; validação de schema; ferramentas 1:1 com o legado; **independência de cliente (múltiplos clientes)**, e2e com ≥2 clientes.
10. **ADR-Sync-010** — Sync/watch unificado por Application/Store serializado; `deferSync` eliminado.
11. **ADR-Plataforma-011** — **Windows PRIMARY/OFICIAL (continua o alvo do legado); Linux SECONDARY/OFICIAL; macOS fora do suporte**; implicações de CI (windows-latest + ubuntu-latest), release, write-replace atômico e lock cross-platform.
12. **ADR-Obsidian-012** — **Decidido:** manter `detect`/`graph`/`open`; **remover** plugin/HTTP `:3100`; novo transporte HTTP = NEW FEATURE futura.
13. **ADR-Perf-013** — Métricas baseline (startup/sync/query/reindex/RSS) com metas só após medição.
14. **ADR-Consist-014** — Modelo de consistência multi-processo: lock advisory obrigatório para mutação, fingerprints de conflito, "vault sempre vence", garantias G1–G3 (sem atomicidade cruzada FS↔SQLite), default do `dbPath` fora de vault sincronizado.
15. **ADR-Backup-015** — Backup oficial = **somente vault Markdown**, `.zip` nativo cross-platform; derivados nunca são fonte (o legado incluía `index.db`).
16. **ADR-Dist-016** — Distribuição via **GitHub Releases + Portable ZIP**; prioridade Windows; Linux com artefato compatível.
17. **ADR-Exec-017** (mantido separado do 008 — sem fusão) — Allowlist do `exec`: formato de config, política de args/env/cwd/timeout, desabilitado por default, port `CommandRunner`.
18. **ADR-VaultSync-018** — **A criar após confirmação do produto**: matriz oficial de suporte S1–S4 (S1/S2/S4 suportados com `.db` local; S3 proibido).

---

## IMPLEMENTATION PHASES

Timeline em fases, cada uma com critério de saída; sem datas (decisão de orquestração):

```text
P0  Congelar contratos + rodar legado TS para gerar fixtures de caracterização   [saída: fixtures + baseline coverage]
    + CHANGE IDs de C3(hybrid/fold)/C5(plugin)/C6(backup) anotados nas fixtures; lista de ADR precisa da resposta C9
P1  Workspace Rust + core de domínio (Note/ADR/Project/Session/graph/VOs) + testes [saída: core puro verde]
P2  Application + ports + Config validado                                           [saída: casos de uso com ports stubs]
P3  Store sqlite (FTS5+WAL, transações, rebuild) + teste de persistência (I1)       [saída: shutdown/restart consistente]
    + default de dbPath fora do vault (Windows %LOCALAPPDATA% primário) + aviso doctor; lock cross-platform
P4  Parser + frontmatter round-trip (ADR-004) + fixtures do legado                  [saída: parse idêntico no vocabulário]
P5  Search (bm25) + Embed (ort) + degradação visível                                [saída: ranking real; fallback reportado]
    + default hybrid, fold de acentos, fusão 50/50 (pesos config) — CHANGE IDs
P6  Graph derivado + Vault (fs atômico, watch 250ms, rename/orphans)                [saída: sync/watch idempotentes]
    + replace-atômico testado em Windows; doctor detecta *.sync-conflict-* e dbPath dentro do vault
P7  CLI completa (contrato)                                                         [saída: golden outputs]
    + sem `obsidian plugin`; search default hybrid; backup somente-vault zip nativo
P8  MCP completo (16 tools/3 resources/2 prompts, validação)                        [saída: e2e com ≥2 clientes]
    + exec allowlist (lista pendente); info expõe estratégia default hybrid
P9  Integração e2e + paridade (galeria TS-Rust) + documentação mínima               [saída: matriz de paridade preenchida]
    + CI windows-latest + ubuntu-latest; release Portable ZIP (Windows) + artefato Linux; docs sem macOS
P10 Melhorias aprovadas (CHANGE ID) + Validação final                               [saída: critérios de aceite]
    + aceite reescrito para Windows primary/Linux secondary; sem menção/critério macOS
```

Ordem justificada: domínio antes de adapters (testes sem I/O); store cedo (mata o risco de persistência); parser antes de search (dependência); CLI/MCP por último (superfícies que dependem de tudo). Isso difere da ordem do gate apenas no ponto de colocar CLI/MCP ao final (igual à preferência do gate).

---

## ACCEPTANCE CRITERIA

A migração só pode ser considerada concluída com evidência de **todos**:

1. Compilação e CI verdes do workspace no gate **obrigatório duplo**: `windows-latest` (**primary**) e `ubuntu-latest` (**secondary oficial**). **Nenhum** critério/CI/menção de suporte a **macOS** (fora do suporte oficial).
2. Testes de caracterização do legado rodando como fixtures: parser (14), graph (11) e contratos MCP/CLI com outputs golden.
3. **Sem mais perda de dados por persistência**: teste `sync → restart → estado consistente` verde (regressão do bug I1), e banco descartável (rebuild só a partir do vault).
4. Busca retornando `score > 0` relevantes; nenhum `score=0` sem explicação; degradação (sem FTS/embeddings) **visível** ao chamador.
5. `link_edges` com contagem = wiki-links parseados (sem o hiato 440 vs 34).
6. ADR não sobrescreve: duas criações → dois ids **distintos** (formato a confirmar — pendência C9; recomendação 4 dígitos `ADR-0001`/`ADR-0002`); regra **MAX+1** sobre o padrão `ADR/ADR-\d{4}`, sem preencher gaps.
7. IDs canônicos: input com `.md` resolve igual ao sem `.md`.
8. Frontmatter: edição de conteúdo via create não destrói campos/metadata desconhecidos (fixture anexada sem perder).
9. `second_brain_exec` **desabilitado por default**; quando habilitado, allowlist sem `env`/`PATH` injetável (o legado aceitava `a.env`); segredos do vault não indexados.
10. CLI e MCP rodando a galeria de paridade com divergências classificadas (nenhuma `DESCONHECIDO` não endereçada); CHANGE IDs de C3/C4/C5/C6 registrados.
11. Vault real do usuário intacto durante a migração (nunca tocado; validação sempre em fixtures/snapshot).
12. Documentação mínima (`docs/{product-context,legacy-baseline,architecture,contracts,parity-matrix,migration-log}` + ADRs) e nenhuma funcionalidade removida sem decisão registrada.
13. Métricas baseline medidas (startup/sync/query/reindex/memória) antes de qualquer otimização.
14. **Windows (plataforma primária):** teste de escrita atômica **com overwrite** (substituição de arquivo existente) e teste de lock interprocesso **cross-platform** verdes na CI `windows-latest`.
15. **`.db` default fora do vault**; `doctor` avisa quando `dbPath` aponta para dentro do vault; nenhuma configuração padrão grava derivados dentro do vault sincronizado.
16. **Backup contém somente o vault Markdown** (sem `index.db`/derivados), `.zip` gerado nativamente e verificável.
17. **Busca:** default `hybrid` em CLI/MCP/config; fold de acentos ativo (`café`↔`cafe`); fusão 50/50 com pesos normalizados; `score` em 0..1.