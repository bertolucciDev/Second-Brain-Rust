//! Camada de escrita markdown com preservação de frontmatter (P6) — matriz
//! `FRONTMATTER MATRIX` (`docs/rust-architecture.md` §FRONTMATTER).
//!
//! Regra central: **não destruir nada que o sistema não tenha editado.**
//! - Conteúdo editado + FM inalterado → **edição mínima**: reutiliza o bloco
//!   raw **byte a byte**, alterando apenas a linha `updated` (auto-tocada).
//! - Mudança em `title`/`tags`/`links`/`project` → edição **linha-a-linha**
//!   dessas chaves; comentários/chaves desconhecidas/ordem preservados.
//! - YAML inválido/avançado (aninhado, âncora/alias, bloco `|`/`>`, chaves
//!   duplicadas, aspas quebradas) → **edição de FM recusada** com erro claro;
//!   edição de conteúdo funciona (raw opaco preservado, `updated` não tocado).
//! - Sem frontmatter + edição só de conteúdo → **não cria** bloco FM.
//!
//! Vive no **core** (não na infra) pois é regra de produto; o `Application`
//! usa `MarkdownEditor::apply_note` ao persistir (T3).

use crate::app::error::{AppError, Result};
use crate::domain::entities::Note;
use crate::domain::frontmatter::{FmValue, Frontmatter};

pub struct MarkdownEditor;

impl MarkdownEditor {
    /// Reutiliza o bloco de frontmatter raw (byte a byte) trocando apenas o
    /// corpo. Sem FM → apenas `new_body` (matriz: "sem frontmatter não cria FM").
    pub fn rewritten_body(original: &str, new_body: &str) -> String {
        match split_blocks(original) {
            Some((yaml, _)) => format!("---\n{yaml}\n---\n{new_body}"),
            None => new_body.to_string(),
        }
    }

    /// O YAML é editável pela nossa ferramenta? Apenas FM simples do formato
    /// legado (`key: scalar` / `key: [a, b]`). Qualquer coisa além disso é
    /// preservada como opaca e **recusada** em edição de FM.
    pub fn fm_is_editable(yaml: &str) -> bool {
        if Frontmatter::parse(yaml).is_err() {
            return false;
        }
        let mut seen: Vec<&str> = Vec::new();
        for line in yaml.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if line.starts_with(char::is_whitespace) {
                return false; // aninhado (lista/valor em bloco)
            }
            let Some(colon) = trimmed.find(':') else {
                return false;
            };
            if colon == 0 {
                return false;
            }
            let key = trimmed[..colon].trim();
            if seen.contains(&key) {
                return false; // chaves duplicadas
            }
            seen.push(key);
            let raw_value = trimmed[colon + 1..].trim();
            // Âncora/alias, bloco escalar, lista em bloco (`key:` vazio) → avançado.
            if raw_value.starts_with('&')
                || raw_value.starts_with('*')
                || raw_value.starts_with('|')
                || raw_value.starts_with('>')
            {
                return false;
            }
        }
        true
    }

    /// Produz o conteúdo final a gravar para a nota, aplicando a matriz.
    ///
    /// Erros (`AppError::Vault`) = **recusa** de edição de FM em YAML
    /// inválido/avançado — o arquivo não é tocado nesse caso (G2).
    pub fn apply_note(original: &str, note: &Note) -> Result<String> {
        let Some((yaml, _)) = split_blocks(original) else {
            return Self::apply_without_fm(note);
        };

        if fm_edits_required(yaml, note) {
            if !Self::fm_is_editable(yaml) {
                return Err(AppError::Vault(
                    "frontmatter inválido/avançado impediu a edição (nada foi alterado)"
                        .to_string(),
                ));
            }
            return Ok(with_fm(yaml, note, note.content()));
        }

        // Edição de conteúdo: reutiliza o raw; refresca a linha `updated` apenas
        // quando o FM é editável (opaco → raw intacto, sem tocar nada).
        if Self::fm_is_editable(yaml) && fm_updated_differs(yaml, note) {
            return Ok(with_fm(yaml, note, note.content()));
        }
        Ok(Self::rewritten_body(original, note.content()))
    }

    /// Arquivo sem bloco de FM: edição de conteúdo **não cria** FM; operação
    /// explícita sobre FM (tags/links/title/project) cria por serialização
    /// canônica (não há raw a preservar).
    fn apply_without_fm(note: &Note) -> Result<String> {
        let fm_is_touched = effective_entries(note)
            .iter()
            .any(|(k, _)| !matches!(k.as_str(), "updated" | "created"));
        if fm_is_touched {
            Ok(format!("{}{}", canonical_block(note), note.content()))
        } else {
            Ok(note.content().to_string())
        }
    }
}

/// Alguma edição **de FM** (fora do auto-`updated`) é necessária?
/// YAML inválido/avançado → note carrega FM vazio (opaco); qualquer campo não
/// `updated/created` no alvo então implica edição de FM (→ será recusada).
fn fm_edits_required(yaml: &str, note: &Note) -> bool {
    let Ok(current) = Frontmatter::parse(yaml) else {
        return !effective_entries(note)
            .iter()
            .all(|(k, _)| matches!(k.as_str(), "updated" | "created"));
    };
    let effective = effective_entries(note);
    for (k, v) in &effective {
        if matches!(k.as_str(), "updated" | "created") {
            continue;
        }
        if current.get(k) != Some(v) {
            return true;
        }
    }
    // Chaves presentes no arquivo mas ausentes do alvo → edição de FM.
    for (k, _) in current.entries() {
        if matches!(k.as_str(), "updated" | "created") {
            continue;
        }
        if !effective.iter().any(|(ek, _)| ek == &k) {
            return true;
        }
    }
    false
}

/// `updated` do alvo difere do atual (auto-touch de `update_*`)?
fn fm_updated_differs(yaml: &str, note: &Note) -> bool {
    match Frontmatter::parse(yaml) {
        Ok(current) => current.get("updated") != note.frontmatter().get("updated"),
        Err(_) => false,
    }
}

/// Entradas efetivas do frontmatter alvo: o FM da nota **mais** o reflexo de
/// `tags`/`links` guardados nos vetores da entidade quando o FM não os carrega
/// (caso das notas criadas por `Note::create`/`toMarkdown`, onde tags/links
/// saem dos vetores e não do FM). O arquivo está sempre com esses campos; se o
/// editor os ignorasse, uma edição os apagaria do arquivo (data loss P6).
pub(crate) fn effective_entries(note: &Note) -> Vec<(String, FmValue)> {
    let mut out = note.frontmatter().entries();
    let has_tags = out.iter().any(|(k, _)| k == "tags");
    if !has_tags {
        let tags: Vec<String> = note.tags().iter().map(|t| t.value().to_string()).collect();
        if !tags.is_empty() {
            out.push(("tags".into(), FmValue::List(tags)));
        }
    }
    let has_links = out.iter().any(|(k, _)| k == "links");
    if !has_links {
        let links: Vec<String> = note
            .wiki_links()
            .iter()
            .map(|l| l.target().to_string())
            .collect();
        if !links.is_empty() {
            out.push(("links".into(), FmValue::List(links)));
        }
    }
    out
}

fn with_fm(yaml: &str, note: &Note, body: &str) -> String {
    format!("---\n{}\n---\n{}", rewrite_fm_lines(yaml, note), body)
}

/// Reescreve apenas a(s) linha(s) da(s) chave(s) que mudaram; o resto do YAML
/// (comentários, chaves desconhecidas, ordem) é preservado verbatim. Chaves
/// novas do alvo são anexadas ao final, na ordem do alvo.
fn rewrite_fm_lines(yaml: &str, note: &Note) -> String {
    let current = Frontmatter::parse(yaml).unwrap_or_default();
    let target = effective_entries(note);

    let mut out: Vec<String> = Vec::new();
    let mut kept_or_rewritten: Vec<String> = Vec::new();

    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push(line.to_string());
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let key = trimmed[..colon].trim().to_string();
        match target.iter().find(|(k, _)| *k == key) {
            None => continue, // chave removida do alvo → linha cai
            Some((_, v)) => {
                let line = if current.get(&key) == Some(v) {
                    line.to_string()
                } else {
                    serialize_line(&key, v)
                };
                if !line.is_empty() {
                    out.push(line);
                }
                kept_or_rewritten.push(key);
            }
        }
    }
    // Chaves do alvo que não existiam no YAML original (ordem do alvo).
    for (k, v) in target {
        if kept_or_rewritten.iter().any(|x| x == &k) {
            continue;
        }
        let line = serialize_line(&k, &v);
        if !line.is_empty() {
            out.push(line);
        }
        kept_or_rewritten.push(k);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// `k: "v"` / `k: ["a", "b"]` — estilo canônico do legado (`toMarkdown`).
/// Valores vazios ⇒ linha omitida.
fn serialize_line(key: &str, value: &FmValue) -> String {
    match value {
        FmValue::Str(s) if !s.is_empty() => format!("{key}: \"{s}\""),
        FmValue::List(items) if !items.is_empty() => format!(
            "{key}: [{}]",
            items
                .iter()
                .map(|i| format!("\"{i}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => String::new(),
    }
}

/// Bloco canônico `---\n...\n---\n` para arquivos sem FM com operação explícita
/// de FM (mesmo formato de `Note::to_markdown`).
fn canonical_block(note: &Note) -> String {
    let lines: Vec<String> = effective_entries(note)
        .iter()
        .filter_map(|(k, v)| {
            let line = serialize_line(k, v);
            (!line.is_empty()).then_some(line)
        })
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!("---\n{}\n---\n", lines.join("\n"))
}

/// `(yaml_raw, body)` quando há bloco de FM no formato legado; `None` = sem FM
/// (mesmo detector do parser P4: bloco/body vazios não contam como FM).
fn split_blocks(content: &str) -> Option<(&str, &str)> {
    if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(i) = rest.find("\n---\n") {
            let yaml = &rest[..i];
            let body = &rest[i + 5..];
            if !yaml.is_empty() && !body.is_empty() {
                return Some((yaml, body));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::{ProjectId, Tag};

    /// Nota inicial "parseada" de um conteúdo hipotético: o frontmatter é o que
    /// `Frontmatter::parse` extrairia (o parser real é infra — core não depende
    /// dele, então os fixtures constroem direto).
    fn note_from(raw: &str) -> Note {
        match split_blocks(raw) {
            Some((yaml, body)) => {
                let fm = Frontmatter::parse(yaml).unwrap_or_default();
                let title = fm
                    .get_title()
                    .map(str::to_string)
                    .unwrap_or_else(|| "T".into());
                Note::create("x/note.md", &title, body, Some(fm), &[], &[], None, None).unwrap()
            }
            None => Note::create(
                "x/note.md",
                "T",
                raw,
                Some(Frontmatter::from_pairs(&[])),
                &[],
                &[],
                None,
                None,
            )
            .unwrap(),
        }
    }

    #[test]
    fn content_edit_reuses_raw_fm_replacing_only_updated() {
        let original = "---\ntitle: \"T\"\ntags: [a, b]\naliases: [\"Ancillary\"]\nproject: p\n---\nold body\n";
        let note = note_from(original).update_content("new body");
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.contains("tags: [a, b]"));
        assert!(out.contains("aliases: [\"Ancillary\"]"));
        assert!(out.contains("project: p"));
        assert!(out.contains("title: \"T\""));
        assert!(out.ends_with("new body"));
        // ---, title, tags, aliases, project, updated, ---, body = 8 linhas.
        assert_eq!(out.lines().count(), 8);
    }

    #[test]
    fn no_fm_content_edit_does_not_create_fm() {
        let original = "just body\n";
        let note = note_from(original).update_content("changed");
        assert_eq!(
            MarkdownEditor::apply_note(original, &note).unwrap(),
            "changed"
        );
    }

    #[test]
    fn no_fm_tag_edit_creates_canonical_fm() {
        let original = "just body\n";
        let note = note_from(original).add_tag(&Tag::create("c").unwrap());
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.starts_with("---\n"));
        assert!(out.contains("tags: [\"c\"]"));
        assert!(out.trim_end().ends_with("just body"));
    }

    #[test]
    fn tag_edit_rewrites_only_tags_line() {
        let original = "---\ntitle: \"T\"\ntags: [a]\nnote: keep-me\n# a comment\n---\nbody\n";
        let note = note_from(original).add_tag(&Tag::create("c").unwrap());
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.contains("tags: [\"a\", \"c\"]"));
        assert!(out.contains("note: keep-me"));
        assert!(out.contains("# a comment"));
        assert!(out.contains("title: \"T\""));
    }

    #[test]
    fn unknown_comment_and_ordering_preserved() {
        let original = "---\n# lead\nzzz: \"prime\"\ntags: [a]\ntitle: \"T\"\n---\nbody\n";
        let note = note_from(original).remove_tag(&Tag::create("a").unwrap());
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.contains("# lead"));
        assert!(out.contains("zzz: \"prime\""));
        assert!(!out.contains("tags:"));
        assert!(out.contains("title: \"T\""));
    }

    #[test]
    fn advanced_fm_content_ok_fm_edit_refused() {
        let advanced = "---\nsummary: |\n  linha 1\n  linha 2\ntags: [a]\n---\nbody\n";
        assert!(!MarkdownEditor::fm_is_editable("summary: |\n  linha1\n"));
        let note = note_from(advanced).update_content("new");
        // Conteúdo: raw reutilizado (updated NÃO é tocado em FM opaco).
        let out = MarkdownEditor::apply_note(advanced, &note).unwrap();
        assert!(out.contains("summary: |"));
        assert!(out.contains("  linha 1"));
        assert!(out.ends_with("new"));
        // Edição de FM → recusa (erro claro; nada alterado).
        let note = note_from(advanced).add_tag(&Tag::create("b").unwrap());
        let err = MarkdownEditor::apply_note(advanced, &note).unwrap_err();
        assert!(err.to_string().contains("frontmatter"));
        assert!(err.to_string().contains("avançado"));
    }

    #[test]
    fn anchor_alias_and_duplicate_keys_refuse_fm_edit() {
        assert!(!MarkdownEditor::fm_is_editable("base: &b v\nx: *b\n"));
        assert!(!MarkdownEditor::fm_is_editable("tags: [a]\ntags: [b]\n"));
        let original = "---\nbase: &b v\nx: *b\n---\nbody\n";
        let note = note_from(original).update_content("c");
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.contains("base: &b v"));
    }

    #[test]
    fn structurally_invalid_yaml_content_ok_fm_refused() {
        // Linha sem ':' (ex.: "title only") → parse falha → FM opaco.
        let invalid = "---\ntitle only\ntags: [a]\n---\nbody\n";
        assert!(!MarkdownEditor::fm_is_editable("title only\n"));
        let note = note_from(invalid).update_content("c");
        let out = MarkdownEditor::apply_note(invalid, &note).unwrap();
        assert!(out.starts_with("---\n"));
        assert!(out.contains("title only"));
        let note = note_from(invalid).add_tag(&Tag::create("b").unwrap());
        assert!(MarkdownEditor::apply_note(invalid, &note).is_err());
    }

    #[test]
    fn unclosed_quote_is_opaque_but_safely_replaceable() {
        let weird = "---\ntitle: \"unclosed\ntags: [a]\n---\nbody\n";
        let note = note_from(weird).update_content("c");
        let out = MarkdownEditor::apply_note(weird, &note).unwrap();
        assert!(out.contains("title: \"unclosed"));
        let note = note_from(weird).update_title("Novo");
        let out = MarkdownEditor::apply_note(weird, &note).unwrap();
        assert!(out.contains("title: \"Novo\""));
        assert!(!out.contains("unclosed"));
    }

    #[test]
    fn accents_and_dates_preserved() {
        let original = "---\ntitle: \"Café — Ação\"\ncreated: 2026-10-06\n---\nbody\n";
        let note = note_from(original).update_content("nova");
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.contains("title: \"Café — Ação\""));
        assert!(out.contains("created: 2026-10-06"));
        assert!(out.ends_with("nova"));
    }

    #[test]
    fn block_list_is_advanced_but_content_ok() {
        let original = "---\ntags:\n  - a\n  - b\n---\nbody\n";
        assert!(!MarkdownEditor::fm_is_editable("tags:\n  - a\n"));
        let note = note_from(original).update_content("c");
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.contains("tags:"));
        assert!(out.contains("  - a"));
    }

    #[test]
    fn extra_target_keys_appended_in_order() {
        let original = "---\naliases: [\"A\"]\n---\nbody\n";
        let note = note_from(original).set_project(&ProjectId::create("p").unwrap());
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.contains("aliases: [\"A\"]"));
        assert!(out.contains("project: \"p\""));
        let aliases = out.lines().position(|l| l.starts_with("aliases:")).unwrap();
        let project = out.lines().position(|l| l.starts_with("project:")).unwrap();
        assert!(aliases < project);
    }

    #[test]
    fn set_title_replaces_only_title_line() {
        let original = "---\ncreated: 2026-10-06\ntitle: \"Antigo\"\n---\nbody\n";
        let note = note_from(original).update_title("Novo");
        let out = MarkdownEditor::apply_note(original, &note).unwrap();
        assert!(out.contains("title: \"Novo\""));
        assert!(out.contains("created: 2026-10-06"));
    }
}
