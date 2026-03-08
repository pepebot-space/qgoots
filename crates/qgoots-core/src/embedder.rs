use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct OllamaEmbeddingRequest {
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct OllamaEmbeddingResponse {
    pub embedding: Vec<f64>,
}

pub struct Embedder {
    client: Client,
    pub base_url: String,
    pub model: String,
}

impl Embedder {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            model,
        }
    }

    pub async fn embed(&self, text: &str) -> anyhow::Result<Vec<f64>> {
        let url = format!("{}/api/embeddings", self.base_url);
        let req = OllamaEmbeddingRequest {
            model: self.model.clone(),
            prompt: text.to_string(),
        };

        let res = self.client
            .post(&url)
            .json(&req)
            .send()
            .await?
            .json::<OllamaEmbeddingResponse>()
            .await?;

        Ok(res.embedding)
    }
}
