use serde::{Deserialize, Serialize};

/// What kind of memory this is. Priority order for prompt injection:
/// instruction > preference > fact > context (higher = included first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryCategory {
    Instruction,
    Preference,
    Fact,
    Context,
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Instruction => write!(f, "instruction"),
            Self::Preference => write!(f, "preference"),
            Self::Fact => write!(f, "fact"),
            Self::Context => write!(f, "context"),
        }
    }
}

impl std::str::FromStr for MemoryCategory {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "instruction" => Ok(Self::Instruction),
            "preference" => Ok(Self::Preference),
            "fact" => Ok(Self::Fact),
            "context" => Ok(Self::Context),
            other => Err(format!("unknown memory category: {other}")),
        }
    }
}

/// How the memory was acquired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// User explicitly stated this ("I'm vegetarian").
    UserSaid,
    /// AI inferred from conversation context.
    Inferred,
    /// Admin set this on behalf of the user.
    AdminSet,
}

impl std::fmt::Display for MemorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserSaid => write!(f, "user_said"),
            Self::Inferred => write!(f, "inferred"),
            Self::AdminSet => write!(f, "admin_set"),
        }
    }
}

impl std::str::FromStr for MemorySource {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user_said" => Ok(Self::UserSaid),
            "inferred" => Ok(Self::Inferred),
            "admin_set" => Ok(Self::AdminSet),
            other => Err(format!("unknown memory source: {other}")),
        }
    }
}

/// Single memory entry for a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMemory {
    pub id: i64,
    pub user_id: String,
    pub category: MemoryCategory,
    pub key: String,
    pub value: String,
    /// 0.0–1.0 confidence score. Higher confidence wins on UPSERT.
    pub confidence: f64,
    pub source: MemorySource,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Single conversation message, stored per-user with cost tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: i64,
    pub user_id: Option<String>,
    pub session_key: String,
    pub channel: String,
    pub role: String,
    pub content: String,
    pub model_used: Option<String>,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_usd: f64,
    pub created_at: String,
}

/// A knowledge base entry — operator or bot-authored fact stored with FTS5 index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub id: i64,
    pub topic: String,
    pub content: String,
    /// Comma-separated tags for loose categorisation (e.g. "ai,models,anthropic").
    pub tags: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Rendered user context ready for prompt injection.
/// Capped at ~1500 tokens. Priority: instruction > preference > fact > context.
#[derive(Debug, Clone)]
pub struct UserContext {
    pub user_id: String,
    pub rendered: String,
    pub memory_count: usize,
    pub built_at: chrono::DateTime<chrono::Utc>,
}
