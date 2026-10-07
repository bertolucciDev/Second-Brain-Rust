import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as path from "path";
import * as fs from "fs";
import { Note } from "../../src/domain/entities/Note";
import { NoteId } from "../../src/domain/value-objects/NoteId";
import { MemoryNoteRepository } from "../../src/infrastructure/persistence/MemoryNoteRepository";
import { MemorySearchIndex } from "../../src/infrastructure/search/MemorySearchIndex";
import { openSqlite } from "../../src/infrastructure/persistence/sqlite/connection";

const TEST_DB = path.join(process.cwd(), "tests", ".tmp", "test-index.db");
const TEST_VAULT = path.join(process.cwd(), "tests", ".tmp", "test-vault");

describe("MemoryNoteRepository + MemorySearchIndex", () => {
  let repo: MemoryNoteRepository;
  let index: MemorySearchIndex;

  beforeAll(async () => {
    fs.rmSync(testDir(), { recursive: true, force: true });
    fs.mkdirSync(testDir(), { recursive: true });
    fs.mkdirSync(TEST_VAULT, { recursive: true });

    const conn = await openSqlite(TEST_DB);
    repo = new MemoryNoteRepository(TEST_DB);
    Object.assign(repo, { conn, db: conn.db });
    index = new MemorySearchIndex(TEST_DB);
    Object.assign(index, { conn, db: conn.db });
  });

  afterAll(async () => {
    await repo.close();
    fs.rmSync(testDir(), { recursive: true, force: true });
  });

  function testDir(): string {
    return path.join(process.cwd(), "tests", ".tmp");
  }

  it("saves and retrieves a note", async () => {
    const note = Note.create({
      path: "test/hello.md",
      title: "Hello World",
      content: "This is a test note about #testing things.",
      tags: ["testing"],
      links: ["other"],
    });
    await repo.saveNote(note);

    const found = await repo.findById(note.getId());
    expect(found).not.toBeNull();
    expect(found!.getTitle()).toBe("Hello World");
    expect(found!.getPath()).toBe("test/hello.md");
    expect(found!.getTags().map(t => t.getValue())).toContain("testing");

    const byPath = await repo.findByPath("test/hello.md");
    expect(byPath).not.toBeNull();
  });

  it("finds notes by tag", async () => {
    const { Tag } = await import("../../src/domain/value-objects/Tag");
    const result = await repo.findByTag(Tag.create("testing"));
    expect(result.items.length).toBeGreaterThanOrEqual(1);
    expect(result.items[0]!.getTitle()).toBe("Hello World");
  });

  it("indexes and searches notes with FTS", async () => {
    const note = Note.create({
      path: "search-test.md",
      title: "Search Test",
      content: "This document contains the word ARCHITECTURE in uppercase and talks about #design.",
      tags: ["design"],
      links: [],
    });
    await repo.saveNote(note);
    await index.index(note);

    const results = await index.search({ query: "architecture", limit: 10 });
    expect(results.length).toBeGreaterThanOrEqual(1);
    expect(results[0]!.noteId.getValue()).toBe("search-test.md");

    const tagged = await index.search({ query: "document", tags: [await import("../../src/domain/value-objects/Tag").then(m => m.Tag.create("design"))], limit: 10 });
    expect(tagged.length).toBeGreaterThanOrEqual(1);
  });

  it("counts notes correctly", async () => {
    const count = await repo.count();
    expect(count).toBeGreaterThanOrEqual(2);
  });

  it("follows wiki-links (backlinks)", async () => {
    const noteB = Note.create({
      path: "link-target.md",
      title: "Target",
      content: "I am linked.",
      tags: [],
      links: [],
    });
    await repo.saveNote(noteB);

    const noteA = Note.create({
      path: "link-source.md",
      title: "Source",
      content: "Links to link-target and another.",
      tags: [],
      links: ["link-target", "another"],
    });
    await repo.saveNote(noteA);

    const backlinks = await repo.findBacklinks(noteB.getId());
    expect(backlinks.length).toBeGreaterThanOrEqual(1);
    expect(backlinks[0]!.getPath()).toBe("link-source.md");
  });

  it("deletes a note", async () => {
    await repo.delete(NoteId.create("link-source"));
    const exists = await repo.exists(NoteId.create("link-source"));
    expect(exists).toBe(false);
  });
});