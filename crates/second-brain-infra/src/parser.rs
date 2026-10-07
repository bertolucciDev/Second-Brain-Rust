//! Parser markdown — vocabulário idêntico ao legado (P4).
//!
//! Espelha byte a byte:
//! - `src/utils/markdown-parser.ts` (`MarkdownParser.parse`) — frontmatter,
//!   headings ATX + slug, tags inline (inclusive dentro de code blocks, quirk do
//!   legado), wiki-links com alias, code blocks e callouts Obsidian.
//! - `SyncService.parseNote` (`src/infrastructure/file-system/sync/SyncService.ts:121`)
//!   — reconstrução do `Note`: id sem `.md` (aceite #7), title = fm || basename,
//!   tags = união(fm, inline) com ordem preservada, wiki-links = inline dedup por
//!   target (primeiro guarda alias) + fm.links não vistos, project = fm.project,
//!   metadata com created/updated do frontmatter (fallback `now_ms`).
//!
//! Decisão de implementação: parser **linear** (scan por linha/caractere), sem
//! pulldown-cmark, para garantir PARIDADE DE VOCABULÁRIO por construção (saída
//! P4: "parse idêntico no vocabulário"). Divergência vs. a menção a pulldown-cmark
//! na arquitetura registrada em `docs/migration-log.md` (D-P4-3).

use second_brain_core::app::error::Result;
use second_brain_core::app::ports::ParserPort;
use second_brain_core::domain::entities::Note;
use second_brain_core::domain::frontmatter::{FmValue, Frontmatter};
use second_brain_core::domain::metadata::Metadata;
use second_brain_core::domain::time::now_ms;
use second_brain_core::domain::vo::{NoteId, ProjectId, Tag, WikiLink};

/// Cabeçalho extraído do frontmatter (ATX `#`–`######`).
#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub level: u32,
    pub text: String,
    pub anchor: String,
}

/// Bloco de código cercado por ``` (fenced).
#[derive(Debug, Clone, PartialEq)]
pub struct CodeBlock {
    pub language: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// Callout Obsidian `> [!TIPO]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Callout {
    pub r#type: String,
    pub title: Option<String>,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// Resultado de `MarkdownParser.parse` — mesmo vocabulário do `ParsedMarkdown` TS.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMarkdown {
    pub frontmatter: Frontmatter,
    pub content: String,
    pub headings: Vec<Heading>,
    pub tags: Vec<Tag>,
    pub wiki_links: Vec<WikiLink>,
    pub code_blocks: Vec<CodeBlock>,
    pub callouts: Vec<Callout>,
}

impl ParsedMarkdown {
    /// Primeira linha não-vazia do conteúdo (equivalente ao `bodyFirstLine` das
    /// fixtures P0).
    pub fn body_first_line(&self) -> Option<&str> {
        self.content.lines().map(str::trim).find(|l| !l.is_empty())
    }
}

/// Adapter do port `ParserPort` (vocabulário do legado).
#[derive(Default)]
pub struct MarkdownParser;

impl ParserPort for MarkdownParser {
    fn parse_note(&mut self, path: &str, content: &str) -> Result<Note> {
        let parsed = MarkdownParser::parse(content);
        let rel = path.to_string();

        let id_source = rel.strip_suffix(".md").unwrap_or(&rel);
        let id = NoteId::create(id_source)?;

        let title = parsed
            .frontmatter
            .get_title()
            .map(str::to_string)
            .unwrap_or_else(|| basename_without_md(&rel).to_string());

        let mut tags_values: Vec<String> = parsed.frontmatter.get_tags();
        for tag in &parsed.tags {
            let value = tag.value();
            if !tags_values.iter().any(|t| t == value) {
                tags_values.push(value.to_string());
            }
        }
        let tags: Vec<Tag> = tags_values
            .iter()
            .map(|t| Tag::create(t))
            .collect::<std::result::Result<Vec<_>, second_brain_core::domain::error::DomainError>>(
            )?;

        let mut seen: Vec<String> = Vec::new();
        let mut wiki_links: Vec<WikiLink> = Vec::new();
        for link in &parsed.wiki_links {
            if !seen.iter().any(|t| t == link.target()) {
                wiki_links.push(WikiLink::from_target(link.target(), link.alias()));
                seen.push(link.target().to_string());
            }
        }
        for target in parsed.frontmatter.get_links() {
            if !seen.iter().any(|t| t == &target) {
                wiki_links.push(WikiLink::from_target(&target, None));
                seen.push(target);
            }
        }

        let project_id = parsed
            .frontmatter
            .get_project()
            .map(ProjectId::create)
            .transpose()?;

        let now = now_ms();
        let created_at = fm_timestamp(&parsed.frontmatter, "created").unwrap_or(now);
        let updated_at = fm_timestamp(&parsed.frontmatter, "updated").unwrap_or(now);
        let metadata = Metadata {
            created_at,
            updated_at,
            version: 1,
            tags: tags_values.clone(),
            source: None,
        };

        Ok(Note::reconstruct(
            id,
            rel,
            title,
            parsed.content.clone(),
            parsed.frontmatter,
            tags,
            wiki_links,
            project_id,
            metadata,
        ))
    }
}

impl MarkdownParser {
    /// `MarkdownParser.parse(content)` do legado.
    pub fn parse(content: &str) -> ParsedMarkdown {
        let (frontmatter, body) = extract_frontmatter(content);
        ParsedMarkdown {
            frontmatter,
            headings: extract_headings(&body),
            tags: extract_tags(&body),
            wiki_links: extract_wiki_links(&body),
            code_blocks: extract_code_blocks(&body),
            callouts: extract_callouts(&body),
            content: body,
        }
    }
}

/// `Frontmatter.parse` tolerante: YAML inválido/avançado lê como **opaco**
/// (raw preservado; vocabulário vazio) — o conteúdo continua legível (matriz
/// `FRONTMATTER MATRIX`: "YAML inválido → lê como opaco"). O aviso no `doctor` e
/// a recusa de edição de FM inválido são da camada de escrita (P6/ADR).
fn extract_frontmatter(content: &str) -> (Frontmatter, String) {
    if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(i) = rest.find("\n---\n") {
            let yaml = &rest[..i];
            let body = &rest[i + 5..];
            // JS `frontmatterMatch[1] && frontmatterMatch[2]`: blocos vazios não
            // são tratados como frontmatter (o corpo todo vira conteúdo).
            if !yaml.is_empty() && !body.is_empty() {
                let fm = Frontmatter::parse(yaml).unwrap_or_else(|_| Frontmatter::empty());
                return (fm, body.to_string());
            }
        }
    }
    (Frontmatter::empty(), content.to_string())
}

/// `^(#{1,6})\s+(.+)$` sobre cada linha (apenas ATX; `#SemEspaço` não captura).
fn extract_headings(body: &str) -> Vec<Heading> {
    let mut headings: Vec<Heading> = Vec::new();
    for line in body.lines() {
        if line.is_empty() {
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes == 0 || hashes > 6 {
            continue;
        }
        let rest = &line[hashes..];
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let text = rest.trim();
        if text.is_empty() {
            continue;
        }
        headings.push(Heading {
            level: hashes as u32,
            text: text.to_string(),
            anchor: slugify(text),
        });
    }
    headings
}

/// `/ #([a-zA-Z0-9_\\-\\/]+)/g` com dedup por valor normalizado (ordem da 1ª
/// ocorrência). Roda sobre o **corpo** (inclui tags dentro de code blocks —
/// quirk preservado). Tags que `Tag.create` rejeita são ignoradas.
fn extract_tags(body: &str) -> Vec<Tag> {
    let chars: Vec<char> = body.chars().collect();
    let mut out: Vec<Tag> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            let mut end = i + 1;
            while end < chars.len()
                && matches!(
                    chars[end],
                    'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' | '/'
                )
            {
                end += 1;
            }
            if end > i + 1 {
                let raw: String = chars[i + 1..end].iter().collect();
                if let Ok(tag) = Tag::create(&raw) {
                    if !out.iter().any(|t| t.value() == tag.value()) {
                        out.push(tag);
                    }
                }
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// `[[target|alias]]` / `[[target]]` — dup por target (primeiro guarda o alias),
/// como em `SyncService.parseNote`. Sem `[[`→ skip. Target/alias trimados.
fn extract_wiki_links(body: &str) -> Vec<WikiLink> {
    let chars: Vec<char> = body.chars().collect();
    let mut links: Vec<WikiLink> = Vec::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '[' && chars[i + 1] == '[' {
            let mut j = i + 2;
            let mut inner = String::new();
            while j < chars.len() && chars[j] != ']' {
                inner.push(chars[j]);
                j += 1;
            }
            if j + 1 < chars.len() && chars[j] == ']' && chars[j + 1] == ']' {
                let (target, alias) = match inner.split_once('|') {
                    Some((t, a)) => (t.trim().to_string(), Some(a.trim().to_string())),
                    None => (inner.trim().to_string(), None),
                };
                if !target.is_empty() {
                    links.push(WikiLink::from_target(&target, alias.as_deref()));
                }
                i = j + 2;
            } else {
                i = j.max(i + 1);
            }
        } else {
            i += 1;
        }
    }
    links
}

/// Blocos de código: linha de abertura `^```(\w*)` (language = "" → "text"),
/// fechamento `line.startsWith("```")`. Linhas vazias são ignoradas (quirk do
/// legado) e os índices 0-based preservam `split("\n")`.
fn extract_code_blocks(body: &str) -> Vec<CodeBlock> {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut blocks: Vec<CodeBlock> = Vec::new();
    let mut in_block = false;
    let mut language = String::new();
    let mut start_line = 0usize;
    let mut block_content: Vec<String> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let fenced = line.starts_with("```");
        if fenced && !in_block {
            in_block = true;
            language = fence_language(line);
            start_line = i;
            block_content = Vec::new();
        } else if fenced && in_block {
            in_block = false;
            blocks.push(CodeBlock {
                language,
                content: block_content.join("\n"),
                start_line,
                end_line: i,
            });
            language = String::new();
        } else if in_block {
            block_content.push((*line).to_string());
        }
    }
    blocks
}

fn fence_language(line: &str) -> String {
    let rest = &line[3.min(line.len())..];
    let lang: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if lang.is_empty() {
        "text".to_string()
    } else {
        lang
    }
}

const CALLOUT_TYPES: [&str; 17] = [
    "note",
    "tip",
    "important",
    "warning",
    "caution",
    "info",
    "example",
    "todo",
    "question",
    "fail",
    "missing",
    "danger",
    "bug",
    "quote",
    "abstract",
    "summary",
    "tldr",
];

/// Callouts Obsidian `> [!TYPE] titulo` + linhas `>` de conteúdo (join "\n",
/// trim). Mesmo fluxo do legado: linha vazia não fecha; linha não-`>` fecha.
fn extract_callouts(body: &str) -> Vec<Callout> {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut callouts: Vec<Callout> = Vec::new();
    let mut in_callout = false;
    let mut callout_type = String::new();
    let mut callout_title: Option<String> = None;
    let mut callout_content: Vec<String> = Vec::new();
    let mut start_line = 0usize;

    for (i, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        if let Some((ty, title)) = callout_prefix(line) {
            if in_callout {
                push_callout(
                    &mut callouts,
                    &callout_type,
                    callout_title.as_deref(),
                    &callout_content,
                    start_line,
                    i.saturating_sub(1),
                );
            }
            in_callout = true;
            callout_type = ty;
            callout_title = title;
            callout_content = Vec::new();
            start_line = i;
        } else if in_callout {
            if let Some(rest) = line.strip_prefix('>') {
                callout_content.push(rest.trim_start().to_string());
            } else {
                in_callout = false;
                push_callout(
                    &mut callouts,
                    &callout_type,
                    callout_title.as_deref(),
                    &callout_content,
                    start_line,
                    i.saturating_sub(1),
                );
            }
        }
    }
    if in_callout {
        push_callout(
            &mut callouts,
            &callout_type,
            callout_title.as_deref(),
            &callout_content,
            start_line,
            lines.len().saturating_sub(1),
        );
    }
    callouts
}

fn push_callout(
    callouts: &mut Vec<Callout>,
    ty: &str,
    title: Option<&str>,
    content: &[String],
    start_line: usize,
    end_line: usize,
) {
    callouts.push(Callout {
        r#type: ty.to_lowercase(),
        title: title.map(str::to_string),
        content: content.join("\n").trim().to_string(),
        start_line,
        end_line,
    });
}

/// `^>\s*\[!(TIPO)\](\s+(.*))?` case-insensitive (17 tipos do legado).
fn callout_prefix(line: &str) -> Option<(String, Option<String>)> {
    if !line.starts_with('>') {
        return None;
    }
    let after_gt = line[1..].trim_start();
    if !after_gt.starts_with("[!") {
        return None;
    }
    let mid = &after_gt[2..];
    let (letters, consumed) = take_ascii_letters(mid);
    let word = letters.to_lowercase();
    if word.is_empty() || !CALLOUT_TYPES.contains(&word.as_str()) {
        return None;
    }
    let after_word = &mid[consumed..];
    if !after_word.starts_with(']') {
        return None;
    }
    let after_bracket = &after_word[1..];
    let title = if after_bracket.starts_with(char::is_whitespace) {
        let t = after_bracket.trim();
        (!t.is_empty()).then(|| t.to_string())
    } else {
        None
    };
    Some((word, title))
}

fn take_ascii_letters(s: &str) -> (String, usize) {
    let mut out = String::new();
    let mut count = 0;
    for c in s.chars() {
        if c.is_ascii_alphabetic() {
            out.push(c);
            count += 1;
        } else {
            break;
        }
    }
    (out, count)
}

/// `slugify` do legado (JS): lowercase → remove não-`\w`/espaço-hífen (ASCII) →
/// runs de whitespace/_/- → `-` → limpa `-` das pontas.
fn slugify(text: &str) -> String {
    let filtered: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace() || *c == '-')
        .collect();
    let joined: String = filtered
        .split(|c: char| c.is_ascii_whitespace() || c == '_' || c == '-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    joined.trim_matches('-').to_string()
}

/// Nome do arquivo sem `.md` (fallback de title em `parseNote`).
fn basename_without_md(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
}

/// `new Date(str).getTime()` para os formatos que produzimos/consumimos:
/// `YYYY-MM-DD` (UTC meia-noite, como o JS) e ISO-8601 completo com `Z`/offset.
/// Sem sufixo de timezone → assume UTC (falha segura). `None` = datas não parseáveis.
fn fm_timestamp(fm: &Frontmatter, key: &str) -> Option<i64> {
    let value = fm.get(key)?;
    let FmValue::Str(s) = value else {
        return None;
    };
    parse_date_ms(s)
}

fn parse_date_ms(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let y = sub(bytes, 0, 4)?;
    let m = sub(bytes, 5, 7)?;
    let d = sub(bytes, 8, 10)?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut ms = days_from_civil(y, m as u32, d as u32) * 86_400_000;
    if bytes.len() == 10 {
        return Some(ms);
    }
    if bytes[10] != b'T' {
        return None;
    }
    let h = sub(bytes, 11, 13)?;
    let mi = sub(bytes, 14, 16)?;
    let se = sub(bytes, 17, 19)?;
    if !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..60).contains(&se) {
        return None;
    }
    ms += (h * 3600 + mi * 60 + se) * 1000;

    let mut idx = 19;
    let mut frac = 0u32;
    let mut count = 0;
    if idx < bytes.len() && bytes[idx] == b'.' {
        idx += 1;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() && count < 3 {
            frac = frac * 10 + u32::from(bytes[idx] - b'0');
            count += 1;
            idx += 1;
        }
    }
    while count < 3 {
        frac *= 10;
        count += 1;
    }
    ms += i64::from(frac);

    if idx < bytes.len() {
        let (sign, offset_ms) = if bytes[idx] == b'Z' {
            (0i64, 0i64)
        } else if bytes[idx] == b'+' || bytes[idx] == b'-' {
            let sign = if bytes[idx] == b'+' { 1i64 } else { -1i64 };
            let oh = sub(bytes, idx + 1, idx + 3)?;
            let om = sub(bytes, idx + 4, idx + 6)?;
            (sign, (oh * 3600 + om * 60) * 1000)
        } else {
            return Some(ms); // sem sufixo → UTC (geração própria sempre emite Z)
        };
        if sign != 0 {
            ms -= sign * offset_ms
        } else {
            ms -= offset_ms;
        }
    }
    Some(ms)
}

fn sub(bytes: &[u8], start: usize, end: usize) -> Option<i64> {
    if bytes.len() < end {
        return None;
    }
    let slice = std::str::from_utf8(&bytes[start..end]).ok()?;
    if !slice.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    slice.parse().ok()
}

/// `days_from_civil` (inverso de `civil_from_days` do core, algoritmo de Hinnant).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/../../tests/fixtures/notes/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
    }

    #[test]
    fn extracts_frontmatter_and_body() {
        let content = "---\ntitle: Test Note\ntags: [planning, api]\nproject: my-project\n---\nThis is the body with #inline-tag and [[linked-note]].\n";
        let parsed = MarkdownParser::parse(content);
        assert_eq!(parsed.frontmatter.get_title(), Some("Test Note"));
        assert!(parsed.content.contains("This is the body"));
    }

    #[test]
    fn extracts_tags_from_frontmatter_and_inline() {
        let content = "---\ntags: [architecture]\n---\n#memoryos #design";
        let parsed = MarkdownParser::parse(content);
        assert_eq!(parsed.frontmatter.get_tags(), vec!["architecture"]);
        let tag_values: Vec<String> = parsed.tags.iter().map(|t| t.value().to_string()).collect();
        assert!(tag_values.iter().any(|t| t == "memoryos"));
        assert!(tag_values.iter().any(|t| t == "design"));
    }

    #[test]
    fn extracts_wiki_links_with_alias() {
        let content =
            "---\nlinks: [home]\n---\nSee [[ADR-001]] and [[architecture|Architecture Overview]].\n";
        let parsed = MarkdownParser::parse(content);
        assert_eq!(parsed.frontmatter.get_links(), vec!["home"]);
        let target: Vec<String> = parsed
            .wiki_links
            .iter()
            .map(|l| l.target().to_string())
            .collect();
        assert!(target.iter().any(|t| t == "ADR-001"));
        assert!(target.iter().any(|t| t == "architecture"));
        let alias = parsed
            .wiki_links
            .iter()
            .find(|l| l.target() == "architecture")
            .and_then(|l| l.alias());
        assert_eq!(alias, Some("Architecture Overview"));
    }

    #[test]
    fn extracts_headings() {
        let content = "# Title\n\n## Section One\n### Subsection\n## Section Two\n";
        let parsed = MarkdownParser::parse(content);
        assert_eq!(parsed.headings.len(), 4);
        assert_eq!(parsed.headings[0].level, 1);
        assert_eq!(parsed.headings[2].level, 3);
        assert_eq!(parsed.headings[0].anchor, "title");
    }

    #[test]
    fn extracts_code_blocks() {
        let content = "```ts\nconst x = 1;\n```\n\n```\nplain\n```\n";
        let parsed = MarkdownParser::parse(content);
        assert_eq!(parsed.code_blocks.len(), 2);
        assert_eq!(parsed.code_blocks[0].language, "ts");
        assert!(parsed.code_blocks[0].content.contains("const x = 1"));
    }

    #[test]
    fn extracts_callouts() {
        let content = "> [!NOTE] Title\n> This is a note\n> with two lines\n\n> [!IMPORTANT]\n> Important callout\n";
        let parsed = MarkdownParser::parse(content);
        assert_eq!(parsed.callouts.len(), 2);
        assert_eq!(parsed.callouts[0].r#type, "note");
        assert_eq!(parsed.callouts[0].title.as_deref(), Some("Title"));
        assert!(parsed.callouts[0].content.contains("with two lines"));
        assert_eq!(parsed.callouts[1].r#type, "important");
    }

    #[test]
    fn sample_full_fixture_vocabulary_parity() {
        let content = fixture("sample-full.md");
        let parsed = MarkdownParser::parse(&content);
        assert_eq!(
            parsed.frontmatter.get_title(),
            Some("Architecture Patterns")
        );
        assert_eq!(
            parsed.frontmatter.get_tags(),
            vec!["architecture", "planning"]
        );
        assert_eq!(parsed.frontmatter.get_project(), Some("second-brain"));
        assert_eq!(
            parsed.frontmatter.get_aliases(),
            vec!["Padrões de Arquitetura"]
        );
        assert_eq!(parsed.frontmatter.get_links(), vec!["home", "roadmap"]);

        assert_eq!(
            parsed
                .tags
                .iter()
                .map(|t| t.value().to_string())
                .collect::<Vec<_>>(),
            vec!["memoryos", "design", "tag-in-code"]
        );

        assert_eq!(
            parsed
                .wiki_links
                .iter()
                .map(|l| (l.target().to_string(), l.alias().map(str::to_string)))
                .collect::<Vec<_>>(),
            vec![
                ("ADR-001".to_string(), None),
                (
                    "architecture".to_string(),
                    Some("Architecture Overview".to_string())
                ),
                ("db".to_string(), None),
            ]
        );

        assert_eq!(
            parsed
                .headings
                .iter()
                .map(|h| (h.level, h.text.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, "Architecture Patterns".to_string()),
                (2, "Section One".to_string()),
                (2, "Section Two".to_string()),
                (3, "Subsection".to_string()),
            ]
        );

        assert_eq!(parsed.code_blocks.len(), 1);
        assert_eq!(parsed.code_blocks[0].language, "ts");
        assert_eq!(
            parsed.code_blocks[0].content.lines().next().map(str::trim),
            Some("const x = 1; // #tag-in-code should still be captured by legacy")
        );

        assert_eq!(parsed.callouts.len(), 2);
        assert_eq!(parsed.callouts[0].r#type, "note");
        assert_eq!(parsed.callouts[0].title.as_deref(), Some("Context"));
        // Legado junta as linhas `>`; a fixture `.note.json` registrou apenas a
        // 1ª linha (limitação do gerador) — ver migration-log P4 (D-P4-5).
        assert_eq!(
            parsed.callouts[0].content,
            "This is a note callout\nwith two lines"
        );
        assert_eq!(parsed.callouts[1].r#type, "important");
        assert_eq!(parsed.callouts[1].title, None);
        assert_eq!(parsed.callouts[1].content, "Important callout");

        assert_eq!(parsed.body_first_line(), Some("# Architecture Patterns"));
    }

    #[test]
    fn sample_minimal_fixture_vocabulary_parity() {
        let content = fixture("sample-minimal.md");
        let parsed = MarkdownParser::parse(&content);
        assert_eq!(parsed.frontmatter.get_title(), Some("Minimal"));
        assert!(parsed.frontmatter.get_tags().is_empty());
        assert!(parsed.tags.is_empty());
        assert!(parsed.wiki_links.is_empty());
        assert!(parsed.headings.is_empty());
        assert!(parsed.code_blocks.is_empty());
        assert!(parsed.callouts.is_empty());
        assert_eq!(parsed.body_first_line(), Some("Body."));
    }

    #[test]
    fn parse_note_matches_syncservice_semantics() {
        let content = fixture("sample-full.md");
        let mut parser = MarkdownParser;
        let note = parser
            .parse_note("Knowledge/sample-full.md", &content)
            .unwrap();

        // Aceite #7: id normalizado na entrada (strip `.md`). A fixture
        // `.note.json` registrou id com `.md` (artefato do gerador P0) — D-P4-6.
        assert_eq!(note.id().value(), "Knowledge/sample-full");
        assert_eq!(note.path(), "Knowledge/sample-full.md");
        assert_eq!(note.title(), "Architecture Patterns");

        let tags: Vec<String> = note.tags().iter().map(|t| t.value().to_string()).collect();
        assert_eq!(
            tags,
            vec![
                "architecture",
                "planning",
                "memoryos",
                "design",
                "tag-in-code"
            ]
        );

        let links: Vec<String> = note
            .wiki_links()
            .iter()
            .map(|l| l.target().to_string())
            .collect();
        // `parseNote` união inline + fm.links (home/roadmap anexados). A fixture
        // `.note.json` registrou apenas os inline (artefato do gerador P0) — D-P4-6.
        assert_eq!(
            links,
            vec!["ADR-001", "architecture", "db", "home", "roadmap"]
        );
        assert_eq!(note.project_id().unwrap().value(), "second-brain");
        assert!(note.content().contains("# Architecture Patterns"));
        assert_eq!(note.metadata().version, 1);
        // `updated: 2026-10-06` → UTC meia-noite (como `new Date("2026-10-06")`).
        assert_eq!(note.metadata().updated_at, 1_791_244_800_000);
    }

    #[test]
    fn round_trip_is_stable_after_first_pass() {
        let content = fixture("sample-full.md");
        let mut parser = MarkdownParser;
        let n1 = parser
            .parse_note("Knowledge/sample-full.md", &content)
            .unwrap();
        let md1 = n1.to_markdown();
        let n2 = parser.parse_note("Knowledge/sample-full.md", &md1).unwrap();
        let md2 = n2.to_markdown();
        // 1º pass reordena tags/links (união escreve ao FM); 2º pass é idempotente.
        assert_eq!(md2, md1);
        // `created_at` usa `now_ms()` a cada parse (sem chave `created` no FM),
        // como `Date.now()` do legado — a estabilidade do round-trip é verificada
        // na forma serializada, não no `Note` completo.
        let n3 = parser.parse_note("Knowledge/sample-full.md", &md2).unwrap();
        assert_eq!(n3.to_markdown(), md2);
    }

    #[test]
    fn slugify_matches_legacy() {
        assert_eq!(slugify("Architecture Patterns"), "architecture-patterns");
        assert_eq!(slugify("  Hello _World_  "), "hello-world");
        assert_eq!(slugify("A---B  C"), "a-b-c");
        assert_eq!(slugify("Çom Acento"), "om-acento"); // \w ASCII (não-\w removido)
    }

    #[test]
    fn date_parser_matches_js_date() {
        const BASE: i64 = 1_791_244_800_000; // 2026-10-06T00:00:00Z
        assert_eq!(parse_date_ms("2026-10-06"), Some(BASE));
        assert_eq!(
            parse_date_ms("2026-10-06T22:00:00.123Z"),
            Some(BASE + 22 * 3_600_000 + 123)
        );
        assert_eq!(
            parse_date_ms("2026-10-06T19:00:00-03:00"),
            Some(BASE + 22 * 3_600_000)
        );
        assert_eq!(parse_date_ms("not-a-date"), None);
        assert_eq!(parse_date_ms(""), None);
    }
}
