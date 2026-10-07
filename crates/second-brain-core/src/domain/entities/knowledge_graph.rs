use std::collections::{HashMap, VecDeque};

/// Nó do grafo (espelha `GraphNode` do legado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub id: String,
    pub path: String,
    pub title: String,
    pub tags: Vec<String>,
    pub project_id: Option<String>,
}

/// Aresta do grafo (espelha `GraphEdge` do legado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub source_title: String,
    pub target_title: String,
}

/// Componente conexa (espelha `ConnectedComponent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedComponent {
    pub id: usize,
    pub nodes: Vec<String>,
    pub size: usize,
}

/// Métricas do grafo (espelha `GraphMetrics`).
#[derive(Debug, Clone, PartialEq)]
pub struct GraphMetrics {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub density: f64,
    pub average_degree: f64,
    pub components: Vec<ConnectedComponent>,
    pub node_centrality: HashMap<String, f64>,
    pub largest_component_size: usize,
    pub component_count: usize,
}

/// Grafo do conhecimento (espelha `KnowledgeGraph.ts`).
///
/// Métricas reproduzem as fórmulas do legado:
/// `density = e / (n·(n-1))` (n>1, **sem** fator 2), `avgDegree = 2e/n`,
/// `centrality = neighbors / (n-1)` (sem arredondamento), componentes por BFS
/// (ordem de inserção dos nós, ordenadas por tamanho decrescente).
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeGraph {
    nodes: HashMap<String, GraphNode>,
    edges: Vec<GraphEdge>,
    pub metrics: GraphMetrics,
}

impl KnowledgeGraph {
    pub fn build(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> KnowledgeGraph {
        let mut node_map = HashMap::new();
        for node in &nodes {
            node_map.insert(node.path.clone(), node.clone());
        }

        let n = nodes.len();
        let e = edges.len();
        let density = if n > 1 {
            e as f64 / (n as f64 * (n as f64 - 1.0))
        } else {
            0.0
        };
        let average_degree = if n > 0 {
            (2.0 * e as f64) / n as f64
        } else {
            0.0
        };

        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
        for node in &nodes {
            adjacency.insert(node.path.clone(), Vec::new());
        }
        for edge in &edges {
            adjacency
                .entry(edge.source.clone())
                .or_default()
                .push(edge.target.clone());
            adjacency
                .entry(edge.target.clone())
                .or_default()
                .push(edge.source.clone());
        }

        let mut centrality: HashMap<String, f64> = HashMap::new();
        for (path, neighbors) in &adjacency {
            let c = if n > 1 {
                neighbors.len() as f64 / (n as f64 - 1.0)
            } else {
                0.0
            };
            centrality.insert(path.clone(), c);
        }

        let components = find_components(&adjacency);
        let largest_component_size = components.iter().map(|c| c.size).max().unwrap_or(0);
        let component_count = components.len();

        KnowledgeGraph {
            nodes: node_map,
            edges,
            metrics: GraphMetrics {
                total_nodes: n,
                total_edges: e,
                density: round5(density),
                average_degree: round2(average_degree),
                components,
                node_centrality: centrality,
                largest_component_size,
                component_count,
            },
        }
    }

    pub fn get_node(&self, path: &str) -> Option<&GraphNode> {
        self.nodes.get(path)
    }

    pub fn edges_for_node(&self, path: &str) -> Vec<&GraphEdge> {
        self.edges
            .iter()
            .filter(|edge| edge.source == path || edge.target == path)
            .collect()
    }

    pub fn neighbors(&self, path: &str) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for edge in &self.edges {
            if edge.source == path {
                set.insert(edge.target.clone());
            }
            if edge.target == path {
                set.insert(edge.source.clone());
            }
        }
        set.into_iter().collect()
    }
}

fn round5(x: f64) -> f64 {
    (x * 100_000.0).round() / 100_000.0
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn find_components(adjacency: &HashMap<String, Vec<String>>) -> Vec<ConnectedComponent> {
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut components: Vec<ConnectedComponent> = Vec::new();
    let mut component_id = 0;

    for node in adjacency.keys() {
        if visited.contains(node) {
            continue;
        }
        let mut component: Vec<String> = Vec::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(node.clone());
        visited.insert(node.clone());

        while let Some(current) = queue.pop_front() {
            component.push(current.clone());
            for neighbor in adjacency.get(&current).unwrap_or(&Vec::new()) {
                if !visited.contains(neighbor) {
                    visited.insert(neighbor.clone());
                    queue.push_back(neighbor.clone());
                }
            }
        }

        components.push(ConnectedComponent {
            id: component_id,
            nodes: component.clone(),
            size: component.len(),
        });
        component_id += 1;
    }

    components.sort_by_key(|c| std::cmp::Reverse(c.size));
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes() -> Vec<GraphNode> {
        vec![
            GraphNode {
                id: "1".into(),
                path: "home.md".into(),
                title: "Home".into(),
                tags: vec![],
                project_id: None,
            },
            GraphNode {
                id: "2".into(),
                path: "architecture.md".into(),
                title: "Architecture".into(),
                tags: vec!["design".into()],
                project_id: None,
            },
            GraphNode {
                id: "3".into(),
                path: "database.md".into(),
                title: "Database".into(),
                tags: vec!["backend".into()],
                project_id: None,
            },
            GraphNode {
                id: "4".into(),
                path: "api.md".into(),
                title: "API Design".into(),
                tags: vec!["backend".into(), "design".into()],
                project_id: None,
            },
            GraphNode {
                id: "5".into(),
                path: "orphan.md".into(),
                title: "Orphan".into(),
                tags: vec![],
                project_id: None,
            },
        ]
    }

    fn edges() -> Vec<GraphEdge> {
        let e = |s: &str, t: &str, st: &str, tt: &str| GraphEdge {
            source: s.into(),
            target: t.into(),
            source_title: st.into(),
            target_title: tt.into(),
        };
        vec![
            e("home.md", "architecture.md", "Home", "Architecture"),
            e("architecture.md", "database.md", "Architecture", "Database"),
            e("database.md", "api.md", "Database", "API Design"),
            e("home.md", "api.md", "Home", "API Design"),
        ]
    }

    #[test]
    fn builds_graph_with_correct_counts() {
        let g = KnowledgeGraph::build(nodes(), edges());
        assert_eq!(g.metrics.total_nodes, 5);
        assert_eq!(g.metrics.total_edges, 4);
    }

    #[test]
    fn computes_density_correctly() {
        // density = edges / (nodes * (nodes-1)) = 4 / (5 * 4) = 0.2 (sem fator 2)
        let g = KnowledgeGraph::build(nodes(), edges());
        assert_approx(g.metrics.density, 0.2, 1e-9);
    }

    #[test]
    fn computes_average_degree() {
        // avg degree = 2*edges / nodes = 8/5 = 1.6
        let g = KnowledgeGraph::build(nodes(), edges());
        assert_approx(g.metrics.average_degree, 1.6, 1e-9);
    }

    #[test]
    fn identifies_connected_components() {
        let g = KnowledgeGraph::build(nodes(), edges());
        assert_eq!(g.metrics.component_count, 2);
        assert_eq!(g.metrics.largest_component_size, 4);
        let biggest = &g.metrics.components[0];
        assert_eq!(biggest.size, 4);
        assert!(biggest.nodes.contains(&"home.md".to_string()));
        assert!(biggest.nodes.contains(&"api.md".to_string()));
        let orphan = &g.metrics.components[1];
        assert_eq!(orphan.size, 1);
        assert!(orphan.nodes.contains(&"orphan.md".to_string()));
    }

    #[test]
    fn computes_node_centrality() {
        let g = KnowledgeGraph::build(nodes(), edges());
        // home.md tem 2 vizinhos; centrality = 2 / (5-1) = 0.5
        assert_approx(g.metrics.node_centrality["home.md"], 0.5, 1e-9);
        assert_eq!(g.metrics.node_centrality["orphan.md"], 0.0);
    }

    #[test]
    fn finds_neighbors_for_a_node() {
        let g = KnowledgeGraph::build(nodes(), edges());
        let neighbors = g.neighbors("architecture.md");
        assert!(neighbors.contains(&"home.md".to_string()));
        assert!(neighbors.contains(&"database.md".to_string()));
        assert!(!neighbors.contains(&"api.md".to_string()));
    }

    #[test]
    fn gets_edges_for_a_node() {
        let g = KnowledgeGraph::build(nodes(), edges());
        let node_edges = g.edges_for_node("home.md");
        assert_eq!(node_edges.len(), 2);
        assert!(node_edges
            .iter()
            .any(|e| format!("{}->{}", e.source, e.target) == "home.md->architecture.md"));
        assert!(node_edges
            .iter()
            .any(|e| format!("{}->{}", e.source, e.target) == "home.md->api.md"));
    }

    #[test]
    fn builds_graph_with_no_edges() {
        let g = KnowledgeGraph::build(nodes(), vec![]);
        assert_eq!(g.metrics.total_edges, 0);
        assert_eq!(g.metrics.density, 0.0);
        assert_eq!(g.metrics.average_degree, 0.0);
        assert_eq!(g.metrics.component_count, 5);
    }

    #[test]
    fn reports_target_metrics() {
        // Representa a projeção por nó usada no `toJSON(targetPath)` da tool graph.
        let g = KnowledgeGraph::build(nodes(), edges());
        let node = g.get_node("home.md").expect("node");
        assert_eq!(node.title, "Home");
        let connections = g.edges_for_node("home.md");
        assert_eq!(connections.len(), 2);
        assert_approx(g.metrics.node_centrality["home.md"], 0.5, 1e-9);
        assert!(connections.iter().any(|e| e.target == "architecture.md"));
    }

    #[test]
    fn loads_graph_with_single_node() {
        let g = KnowledgeGraph::build(vec![nodes()[0].clone()], vec![]);
        assert_eq!(g.metrics.total_nodes, 1);
        assert_eq!(g.metrics.density, 0.0);
        assert_eq!(g.metrics.component_count, 1);
        assert_eq!(g.metrics.largest_component_size, 1);
    }

    #[test]
    fn orphan_component_sorted_after_giant() {
        let g = KnowledgeGraph::build(nodes(), edges());
        let sizes: Vec<usize> = g.metrics.components.iter().map(|c| c.size).collect();
        assert_eq!(sizes, vec![4, 1]);
    }

    fn assert_approx(a: f64, b: f64, eps: f64) {
        assert!((a - b).abs() < eps, "expected {a} ~ {b} (±{eps})");
    }
}
