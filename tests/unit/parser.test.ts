import { describe, it, expect } from "vitest";
import { MarkdownParser } from "../../src/utils/markdown-parser";
import { Note } from "../../src/domain/entities/Note";
import { Tag } from "../../src/domain/value-objects/Tag";
import { WikiLink } from "../../src/domain/value-objects/WikiLink";
import { NoteId } from "../../src/domain/value-objects/NoteId";

describe("MarkdownParser", () => {
  it("extracts frontmatter from markdown", () => {
    const content = `---
title: Test Note
tags: [planning, api]
project: my-project
---
This is the body with #inline-tag and [[linked-note]].`;
    const parsed = MarkdownParser.parse(content);
    expect(parsed.frontmatter.getTitle()).toBe("Test Note");
    expect(parsed.content).toContain("This is the body");
  });

  it("extracts tags from frontmatter and inline", () => {
    const content = `---
tags: [architecture]
---
#memoryos #design`;
    const parsed = MarkdownParser.parse(content);
    expect(parsed.frontmatter.getTags()).toContain("architecture");
    const tagValues = parsed.tags.map(t => t.getValue());
    expect(tagValues).toContain("memoryos");
    expect(tagValues).toContain("design");
  });

  it("extracts wiki-links", () => {
    const content = `---
links: [home]
---
See [[ADR-001]] and [[architecture|Architecture Overview]].`;
    const parsed = MarkdownParser.parse(content);
    expect(parsed.frontmatter.getLinks()).toContain("home");
    const linkTargets = parsed.wikiLinks.map(l => l.getTarget());
    expect(linkTargets).toContain("ADR-001");
    expect(linkTargets).toContain("architecture");
    const alias = parsed.wikiLinks.find(l => l.getTarget() === "architecture")?.getAlias();
    expect(alias).toBe("Architecture Overview");
  });

  it("extracts headings", () => {
    const content = `# Title\n\n## Section One\n### Subsection\n## Section Two`;
    const parsed = MarkdownParser.parse(content);
    expect(parsed.headings).toHaveLength(4);
    expect(parsed.headings[0]!.level).toBe(1);
    expect(parsed.headings[2]!.level).toBe(3);
  });

  it("extracts code blocks", () => {
    const content = "```ts\nconst x = 1;\n```\n\n```\nplain\n```";
    const parsed = MarkdownParser.parse(content);
    expect(parsed.codeBlocks).toHaveLength(2);
    expect(parsed.codeBlocks[0]!.language).toBe("ts");
    expect(parsed.codeBlocks[0]!.content).toContain("const x = 1");
  });

  it("extracts Obsidian callouts", () => {
    const content = "> [!NOTE] Title\n> This is a note\n> with two lines\n\n> [!IMPORTANT]\n> Important callout";
    const parsed = MarkdownParser.parse(content);
    expect(parsed.callouts).toHaveLength(2);
    expect(parsed.callouts[0]!.type).toBe("note");
    expect(parsed.callouts[0]!.title).toBe("Title");
    expect(parsed.callouts[0]!.content).toContain("with two lines");
    expect(parsed.callouts[1]!.type).toBe("important");
  });
});

describe("Note", () => {
  it("creates a note with tags and links", () => {
    const note = Note.create({
      path: "Knowledge/test.md",
      title: "Test Note",
      content: "Content here",
      tags: ["test", "memoryos"],
      links: ["architecture"],
    });

    expect(note.getPath()).toBe("Knowledge/test.md");
    expect(note.getTitle()).toBe("Test Note");
    expect(note.getContent()).toBe("Content here");
    expect(note.getTags().map(t => t.getValue())).toContain("test");
    expect(note.getTags().map(t => t.getValue())).toContain("memoryos");
    expect(note.getWikiLinks().map(l => l.getTarget())).toContain("architecture");
    expect(note.getId().getValue()).toBe("Knowledge/test.md");
  });

  it("adds and removes tags", () => {
    let note = Note.create({
      path: "test.md",
      title: "Test",
      content: "Content",
      tags: ["foo"],
      links: [],
    });
    note = note.addTag(Tag.create("bar"));
    expect(note.getTags().map(t => t.getValue())).toContain("bar");
    note = note.removeTag(Tag.create("foo"));
    expect(note.getTags().map(t => t.getValue())).not.toContain("foo");
    expect(note.getTags()).toHaveLength(1);
  });

  it("generates markdown with frontmatter", () => {
    const note = Note.create({
      path: "test.md",
      title: "Export Test",
      content: "Body text",
      tags: ["api"],
      links: ["db"],
    });
    const md = note.toMarkdown();
    expect(md).toContain("---");
    expect(md).toContain('title: "Export Test"');
    expect(md).toContain('"api"');
    expect(md).toContain('"db"');
    expect(md).toContain("Body text");
  });
});

describe("Tag", () => {
  it("normalizes tags", () => {
    const tag = Tag.create("My TAG Name");
    expect(tag.getValue()).toBe("my-tag-name");
  });

  it("rejects invalid tags", () => {
    expect(() => Tag.create("")).toThrow();
    expect(() => Tag.create("with@invalid!")).toThrow();
    expect(() => Tag.create("tag with spaces")).not.toThrow();
    expect(Tag.create("tag with spaces").getValue()).toBe("tag-with-spaces");
  });
});

describe("WikiLink", () => {
  it("creates link with alias", () => {
    const link = WikiLink.fromTarget("foo", "Foo Title");
    expect(link.getTarget()).toBe("foo");
    expect(link.getAlias()).toBe("Foo Title");
    expect(link.getDisplayText()).toBe("Foo Title");
    expect(link.toString()).toBe("[[foo|Foo Title]]");
  });

  it("creates link without alias", () => {
    const link = WikiLink.fromTarget("bar");
    expect(link.getDisplayText()).toBe("bar");
    expect(link.toString()).toBe("[[bar]]");
  });
});

describe("NoteId", () => {
  it("creates and compares IDs", () => {
    const a = NoteId.create("test");
    const b = NoteId.create("test");
    const c = NoteId.create("other");
    expect(a.equals(b)).toBe(true);
    expect(a.equals(c)).toBe(false);
  });
});