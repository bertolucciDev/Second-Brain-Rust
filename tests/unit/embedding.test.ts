import { test, expect } from "vitest";
import { EmbeddingService } from "../../src/infrastructure/search/EmbeddingService";

test("embedding service generates vectors", async () => {
  const svc = new EmbeddingService();
  const vec = await svc.embed("hello world");
  expect(vec.length).toBeGreaterThan(0);
  expect(vec[0]).toBeTypeOf("number");
});

test("cosine similarity works", async () => {
  const svc = new EmbeddingService();
  const a = await svc.embed("cat");
  const b = await svc.embed("feline");
  const c = await svc.embed("volcano");
  const { cosineSimilarity } = await import("../../src/infrastructure/search/EmbeddingService");

  const simAB = cosineSimilarity(a, b);
  const simAC = cosineSimilarity(a, c);
  expect(simAB).toBeGreaterThan(simAC);
});

test("batch embedding returns correct count", async () => {
  const svc = new EmbeddingService();
  const vecs = await svc.embedBatch(["one", "two", "three"]);
  expect(vecs).toHaveLength(3);
  for (const v of vecs) {
    expect(v.length).toBeGreaterThan(0);
  }
});
