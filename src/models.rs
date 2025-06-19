use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::RecordId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoOwner {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub id: RecordId,
    pub github_id: i64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub url: String,
    pub stars: u32,
    pub language: Option<String>,
    pub owner: RepoOwner,
    pub is_private: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub embedding: Option<Vec<f32>>,
    pub embedding_generated_at: Option<DateTime<Utc>>,
}

impl Repo {
    pub fn needs_embedding(&self) -> bool {
        self.embedding.is_none()
            || self
                .embedding_generated_at
                .map(|embed_time| self.updated_at > embed_time)
                .unwrap_or(true)
    }

    pub fn prepare_text_for_embedding(&self) -> String {
        let mut parts = vec![format!("Repository: {}", self.full_name)];

        if let Some(desc) = &self.description {
            parts.push(format!("Description: {}", desc));
        }

        if let Some(lang) = &self.language {
            parts.push(format!("Language: {}", lang));
        }

        parts.push(format!("Stars: {}", self.stars));
        parts.push(format!("Owner: {}", self.owner.login));

        parts.join("\n")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveQueryNotification {
    pub action: LiveAction,
    pub result: Repo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LiveAction {
    Create,
    Update,
    Delete,
}