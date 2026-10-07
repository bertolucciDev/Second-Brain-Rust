# Fixtures de Contrato e Caracterização (P0)

Capturados do **runtime TS legado** em `2026-10-06` (testes: `vitest run` 25/25; MCP via spawn real em cwd isolado `/tmp`; CLI via `--help`/`--version`). **Read-only sobre o projeto e o vault de produção.**

Os scripts de captura ficam fora do repo (em `/tmp/opencode/p0/`) para não poluir a árvore; os `INPUT` (`.md`) e `OUTPUT` congelados (`.json`/`.golden.md`) são a âncora de paridade TS↔Rust.

## Layout

```text
contracts/
  mcp-initialize.json   initialize → serverInfo second-brain 1.0.0 (protocol 2024-11-05)
  mcp-tools.json        tools/list → 16 tools + inputSchema
  mcp-resources.json    resources/list → 3 URIs
  mcp-prompts.json      prompts/list → 2 prompts
  mcp-unmatched.json    [] (nenhuma msg sem handler)
  cli-help.txt          second-brain --help (golden)
  cli-version.txt       second-brain --version
notes/
  sample-full.md        INPUT — nota completa (fm rico, headings, tags inline+code, wiki+alias, callouts)
  sample-full.note.json OUTPUT — vista parseada (frontmatter/tags/wikiLinks/headings/codeBlocks/callouts)
  sample-full.golden.md OUTPUT — Note.toMarkdown()
  sample-minimal.*      INPUT/OUTPUTs mínimos
```

## Como regenerar (runtime TS)

```bash
# contrato MCP (cwd do servidor em dir isolado com memory.config.json para /tmp)
node /tmp/opencode/p0/capture-mcp.mjs tests/fixtures/contracts
# fixtures de nota
node_modules/.bin/tsx /tmp/opencode/p0/char-note.mjs tests/fixtures/notes
# CLI
node_modules/.bin/tsx src/cli/cli.ts --help > tests/fixtures/contracts/cli-help.txt
```

## Convenções

- `INPUT` (`.md`) é reproduzível; `OUTPUT` congelado é a **referência** — nunca editado manualmente sem repetir o runtime e re-auditar.
- Divergências intencionais (C3–C6) entram na galeria de paridade com CHANGE ID, não por edição destes fixtures.