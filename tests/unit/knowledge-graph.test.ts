import { describe, it, expect } from "vitest";
import { KnowledgeGraph } from "../../src/domain/entities/KnowledgeGraph";
import type { GraphNode, GraphEdge } from "../../src/domain/entities/KnowledgeGraph";

describe("KnowledgeGraph", () => {
  const nodes: GraphNode[] = [
    { id: "1", path: "home.md", title: "Home", tags: [] },
    { id: "2", path: "architecture.md", title: "Architecture", tags: ["design"] },
    { id: "3", path: "database.md", title: "Database", tags: ["backend"] },
    { id: "4", path: "api.md", title: "API Design", tags: ["backend", "design"] },
    { id: "5", path: "orphan.md", title: "Orphan", tags: [] },
  ];

  const edges: GraphEdge[] = [
    { source: "home.md", target: "architecture.md", sourceTitle: "Home", targetTitle: "Architecture" },
    { source: "architecture.md", target: "database.md", sourceTitle: "Architecture", targetTitle: "Database" },
    { source: "database.md", target: "api.md", sourceTitle: "Database", targetTitle: "API Design" },
    { source: "home.md", target: "api.md", sourceTitle: "Home", targetTitle: "API Design" },
  ];

  it("builds graph with correct counts", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    expect(g.metrics.totalNodes).toBe(5);
    expect(g.metrics.totalEdges).toBe(4);
  });

  it("computes density correctly", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    // density = edges / (nodes * (nodes-1)) = 4 / (5 * 4) = 0.2
    expect(g.metrics.density).toBe(0.2);
  });

  it("computes average degree", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    // avg degree = 2*edges / nodes = 8/5 = 1.6
    expect(g.metrics.averageDegree).toBe(1.6);
  });

  it("identifies connected components", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    // 4 connected nodes + 1 orphan = 2 components
    expect(g.metrics.componentCount).toBe(2);
    expect(g.metrics.largestComponentSize).toBe(4);
    const biggest = g.metrics.components[0]!;
    expect(biggest.size).toBe(4);
    expect(biggest.nodes).toContain("home.md");
    expect(biggest.nodes).toContain("api.md");
    const orphan = g.metrics.components[1]!;
    expect(orphan.size).toBe(1);
    expect(orphan.nodes).toContain("orphan.md");
  });

  it("computes node centrality", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    // home.md connects to architecture.md and api.md = 2 neighbors
    // centrality = 2 / (5-1) = 0.5
    expect(g.metrics.nodeCentrality["home.md"]).toBeCloseTo(0.5, 4);
    expect(g.metrics.nodeCentrality["orphan.md"]).toBe(0);
  });

  it("finds neighbors for a node", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    const neighbors = g.getNeighbors("architecture.md");
    expect(neighbors).toContain("home.md");
    expect(neighbors).toContain("database.md");
    expect(neighbors).not.toContain("api.md");
  });

  it("gets edges for a node", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    const nodeEdges = g.getEdgesForNode("home.md");
    expect(nodeEdges).toHaveLength(2);
    expect(nodeEdges.map(e => `${e.source}->${e.target}`)).toContain("home.md->architecture.md");
    expect(nodeEdges.map(e => `${e.source}->${e.target}`)).toContain("home.md->api.md");
  });

  it("builds graph with no edges", () => {
    const g = KnowledgeGraph.build(nodes, []);
    expect(g.metrics.totalEdges).toBe(0);
    expect(g.metrics.density).toBe(0);
    expect(g.metrics.averageDegree).toBe(0);
    expect(g.metrics.componentCount).toBe(5);
  });

  it("generates JSON output for specific node", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    const json = g.toJSON("home.md");
    expect(json.target).toBe("home.md");
    expect(json.targetTitle).toBe("Home");
    expect(json.targetEdges).toBe(2);
    expect(json.centrality).toBeCloseTo(0.5, 4);
    expect(Array.isArray(json.connections)).toBe(true);
  });

  it("generates JSON output without target", () => {
    const g = KnowledgeGraph.build(nodes, edges);
    const json = g.toJSON();
    expect(json.nodes).toBe(5);
    expect(json.edges).toBe(4);
    expect(json.target).toBeUndefined();
    expect(json.centrality).toBeUndefined();
  });

  it("loads graph with single node", () => {
    const g = KnowledgeGraph.build([nodes[0]!], []);
    expect(g.metrics.totalNodes).toBe(1);
    expect(g.metrics.density).toBe(0);
    expect(g.metrics.componentCount).toBe(1);
  });
});