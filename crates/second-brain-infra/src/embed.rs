//! Embeddings via endpoint NVIDIA NIM free (P5b) — substitui o plano local
//! `ort`/all-MiniLM-L6-v2 (decisão de produto; migration-log D-P5b-1).
//!
//! Modelo default: `nvidia/nemotron-3-embed-1b` (1B, dim 2048, multilíngue).
//!
//! Segurança (migration-log D-P5b-2): o conteúdo das notas é enviado à NVIDIA;
//! a chave vem de `NVIDIA_API_KEY` (env), **nunca** de config/arquivo/repo.
//! Sem rede/key/cota → o chamador (FtsSearch) degrada para keyword.

use std::time::Duration;

use second_brain_core::app::error::{AppError, Result};
use second_brain_core::app::ports::EmbedPort;
use serde::{Deserialize, Serialize};

/// Modelo default do produto (rápido, 2048-dim, exige `input_type` passage/query).
/// O anterior `nvidia/llama-3.2-nv-embedqa-1b-v2` atingiu EOL (2026-05-18) —
/// trocado pelo sucessor `nemotron-3-embed-1b`, mesmo perfil (1B, dim 2048,
/// multilíngue).
pub const DEFAULT_NVIDIA_EMBED_MODEL: &str = "nvidia/nemotron-3-embed-1b";
/// Endpoint público free (build.nvidia.com / integrate). Trocar por um NIM
/// próprio só altera `base_url` — API OpenAI-compatible idêntica.
pub const DEFAULT_NVIDIA_BASE_URL: &str = "https://integrate.api.nvidia.com";
pub const DEFAULT_NVIDIA_EMBED_DIM: usize = 2048;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    input: &'a [String],
    model: &'a str,
    input_type: &'a str,
    encoding_format: &'a str,
    truncate: &'a str,
}

pub struct NvidiaEmbed {
    base_url: String,
    model: String,
    api_key: String,
}

impl NvidiaEmbed {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> NvidiaEmbed {
        NvidiaEmbed {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key: api_key.into(),
        }
    }

    /// Constrói a partir de `NVIDIA_API_KEY` (env). `None` se ausente/vazia —
    /// o produto então opera sem embeddings (degradado para keyword).
    pub fn from_env() -> Option<NvidiaEmbed> {
        let api_key = std::env::var("NVIDIA_API_KEY").ok()?;
        if api_key.trim().is_empty() {
            return None;
        }
        Some(NvidiaEmbed::new(
            DEFAULT_NVIDIA_BASE_URL,
            DEFAULT_NVIDIA_EMBED_MODEL,
            api_key,
        ))
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/embeddings", self.base_url)
    }
}

impl EmbedPort for NvidiaEmbed {
    fn embed_for(&mut self, texts: &[String], as_query: bool) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let body = serde_json::to_vec(&EmbedRequest {
            input: texts,
            model: &self.model,
            input_type: if as_query { "query" } else { "passage" },
            encoding_format: "float",
            truncate: "NONE",
        })
        .map_err(|e| AppError::Embed(format!("serialize request: {e}")))?;

        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build()
            .new_agent();
        let resp = agent
            .post(&self.endpoint())
            .header("Authorization", format!("Bearer {}", self.api_key).as_str())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e| AppError::Embed(format!("nvidia embed request failed: {e}")))?;

        let status = resp.status().as_u16();
        let mut response_body = resp.into_body();
        let text = response_body
            .read_to_string()
            .map_err(|e| AppError::Embed(format!("read response: {e}")))?;
        let parsed: EmbedResponse = serde_json::from_str(&text)
            .map_err(|e| AppError::Embed(format!("HTTP {status}: bad body ({e})")))?;

        let mut out: Vec<Vec<f32>> = Vec::with_capacity(parsed.data.len());
        for item in parsed.data {
            if out.len() <= item.index {
                out.resize(item.index + 1, Vec::new());
            }
            out[item.index] = item.embedding;
        }
        // Contrato: um vetor por texto, na ordem. Falta de itens ⇒ erro.
        if out.len() != texts.len() {
            return Err(AppError::Embed(format!(
                "nvidia embed returned {} vectors for {} inputs",
                out.len(),
                texts.len()
            )));
        }
        Ok(out)
    }

    fn is_available(&mut self) -> bool {
        !self.api_key.trim().is_empty() && !self.base_url.is_empty()
    }

    fn vector_size(&mut self) -> usize {
        DEFAULT_NVIDIA_EMBED_DIM
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Mini-servidor HTTP local: captura o corpo/headers e responde um payload
    /// embedding fake em formato OpenAI-compatible (sem tocar na rede).
    fn spawn_fake_server() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, format!("http://{addr}"))
    }

    /// Lê a requisição completa (headers + corpo declarado por Content-Length),
    /// sem depender de EOF — evita deadlock com keep-alive do cliente.
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        let head = String::from_utf8_lossy(&buf).to_string();
        let header_end = head.find("\r\n\r\n").map(|i| i + 4).unwrap_or(head.len());
        let content_len: usize = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                if k.trim().eq_ignore_ascii_case("content-length") {
                    v.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        while buf.len() < header_end + content_len {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    fn write_response(stream: &mut std::net::TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    }

    #[test]
    fn embed_sends_input_type_query_and_parses() {
        let (listener, url) = spawn_fake_server();
        let payload = format!(
            r#"{{"object":"list","data":[
            {{"object":"embedding","index":0,"embedding":[0.1,0.2,0.3]}},
            {{"object":"embedding","index":1,"embedding":[0.4,0.5,0.6]}}
        ],"model":"{DEFAULT_NVIDIA_EMBED_MODEL}","usage":{{"prompt_tokens":0,"total_tokens":0}}}}"#
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.contains(r#""input_type":"query""#));
            assert!(request.contains(DEFAULT_NVIDIA_EMBED_MODEL));
            assert!(request.contains(r#""truncate":"NONE""#));
            write_response(&mut stream, &payload);
        });

        let mut embed = NvidiaEmbed::new(url, DEFAULT_NVIDIA_EMBED_MODEL, "test-key");
        let out = embed
            .embed_for(&["alpha".to_string(), "beta".to_string()], true)
            .unwrap();
        handle.join().unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], vec![0.1, 0.2, 0.3]);
        assert_eq!(out[1], vec![0.4, 0.5, 0.6]);
    }

    #[test]
    fn embed_passes_passage_mode_for_indexing() {
        let (listener, url) = spawn_fake_server();
        let payload =
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.9]}]}"#;
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.contains(r#""input_type":"passage""#));
            write_response(&mut stream, payload);
        });
        let mut embed = NvidiaEmbed::new(url, DEFAULT_NVIDIA_EMBED_MODEL, "k");
        let out = embed.embed(&["doc text".to_string()]).unwrap();
        handle.join().unwrap();
        assert_eq!(out, vec![vec![0.9]]);
    }

    #[test]
    fn unavailable_without_key() {
        let mut embed = NvidiaEmbed::new(DEFAULT_NVIDIA_BASE_URL, DEFAULT_NVIDIA_EMBED_MODEL, "");
        assert!(!embed.is_available());
    }

    #[test]
    fn mismatch_count_errors() {
        // Servidor responde menos vetores que o pedido ⇒ erro (não silencioso).
        let (listener, url) = spawn_fake_server();
        let payload =
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[1.0]}]}"#;
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _request = read_request(&mut stream);
            write_response(&mut stream, payload);
        });
        let mut embed = NvidiaEmbed::new(url, DEFAULT_NVIDIA_EMBED_MODEL, "k");
        let err = embed
            .embed_for(&["a".to_string(), "b".to_string()], false)
            .unwrap_err();
        handle.join().unwrap();
        assert!(err.to_string().contains("returned 1 vectors"));
    }
}
