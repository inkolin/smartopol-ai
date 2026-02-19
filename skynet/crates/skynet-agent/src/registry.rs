//! Built-in provider registry — maps well-known provider IDs to their API
//! endpoints and default models. Used by `build_provider()` in the gateway
//! to resolve provider configuration without requiring users to look up URLs.

/// A well-known LLM provider that is OpenAI-compatible.
pub struct KnownProvider {
    /// Short identifier used in config (e.g. "groq", "deepseek").
    pub id: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
    /// Base URL without trailing slash (e.g. "https://api.groq.com/openai").
    pub base_url: &'static str,
    /// Path appended to base_url for chat completions.
    /// Almost always "/v1/chat/completions"; some providers omit the /v1.
    pub chat_path: &'static str,
    /// Recommended model for this provider.
    pub default_model: &'static str,
    /// True if the provider offers a meaningful free tier.
    pub free_tier: bool,
}

impl KnownProvider {
    /// Full chat completions endpoint URL.
    pub fn endpoint(&self) -> String {
        format!("{}{}", self.base_url, self.chat_path)
    }
}

pub const KNOWN_PROVIDERS: &[KnownProvider] = &[
    // ── Tier 1: Major commercial providers ───────────────────────────────────
    KnownProvider {
        id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai",
        chat_path: "/v1/chat/completions",
        default_model: "llama-3.3-70b-versatile",
        free_tier: true,
    },
    KnownProvider {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com",
        chat_path: "/v1/chat/completions",
        default_model: "deepseek-chat",
        free_tier: false,
    },
    KnownProvider {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api",
        chat_path: "/v1/chat/completions",
        default_model: "openai/gpt-4o",
        free_tier: true,
    },
    KnownProvider {
        id: "xai",
        name: "xAI (Grok)",
        base_url: "https://api.x.ai",
        chat_path: "/v1/chat/completions",
        default_model: "grok-2-latest",
        free_tier: false,
    },
    KnownProvider {
        id: "mistral",
        name: "Mistral AI",
        base_url: "https://api.mistral.ai",
        chat_path: "/v1/chat/completions",
        default_model: "mistral-large-latest",
        free_tier: false,
    },
    KnownProvider {
        id: "perplexity",
        name: "Perplexity",
        base_url: "https://api.perplexity.ai",
        chat_path: "/chat/completions",
        default_model: "sonar-pro",
        free_tier: false,
    },
    KnownProvider {
        id: "together",
        name: "Together AI",
        base_url: "https://api.together.xyz",
        chat_path: "/v1/chat/completions",
        default_model: "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo",
        free_tier: false,
    },
    KnownProvider {
        id: "fireworks",
        name: "Fireworks AI",
        base_url: "https://api.fireworks.ai/inference",
        chat_path: "/v1/chat/completions",
        default_model: "accounts/fireworks/models/llama-v3p3-70b-instruct",
        free_tier: false,
    },
    KnownProvider {
        id: "cerebras",
        name: "Cerebras",
        base_url: "https://api.cerebras.ai",
        chat_path: "/v1/chat/completions",
        default_model: "llama3.3-70b",
        free_tier: true,
    },
    KnownProvider {
        id: "sambanova",
        name: "SambaNova",
        base_url: "https://api.sambanova.ai",
        chat_path: "/v1/chat/completions",
        default_model: "Meta-Llama-3.1-405B-Instruct",
        free_tier: true,
    },
    KnownProvider {
        id: "hyperbolic",
        name: "Hyperbolic",
        base_url: "https://api.hyperbolic.xyz",
        chat_path: "/v1/chat/completions",
        default_model: "meta-llama/Llama-3.3-70B-Instruct",
        free_tier: false,
    },
    KnownProvider {
        id: "novita",
        name: "Novita AI",
        base_url: "https://api.novita.ai/v3/openai",
        chat_path: "/chat/completions",
        default_model: "meta-llama/llama-3.1-70b-instruct",
        free_tier: false,
    },
    KnownProvider {
        id: "lepton",
        name: "Lepton AI",
        base_url: "https://llm.lepton.ai/api",
        chat_path: "/v1/chat/completions",
        default_model: "llama3-3-70b",
        free_tier: true,
    },
    KnownProvider {
        id: "corethink",
        name: "CoreThink",
        base_url: "https://api.corethink.ai",
        chat_path: "/v1/chat/completions",
        default_model: "deepseek-r1",
        free_tier: false,
    },
    KnownProvider {
        id: "featherless",
        name: "Featherless AI",
        base_url: "https://api.featherless.ai",
        chat_path: "/v1/chat/completions",
        default_model: "meta-llama/Meta-Llama-3.1-70B-Instruct",
        free_tier: false,
    },
    KnownProvider {
        id: "requesty",
        name: "Requesty",
        base_url: "https://router.requesty.ai",
        chat_path: "/v1/chat/completions",
        default_model: "openai/gpt-4o",
        free_tier: false,
    },
    KnownProvider {
        id: "glama",
        name: "Glama",
        base_url: "https://glama.ai/api",
        chat_path: "/v1/chat/completions",
        default_model: "openai/gpt-4o",
        free_tier: true,
    },
    KnownProvider {
        id: "chutes",
        name: "Chutes AI",
        base_url: "https://llm.chutes.ai",
        chat_path: "/v1/chat/completions",
        default_model: "deepseek-ai/DeepSeek-R1",
        free_tier: true,
    },
    KnownProvider {
        id: "cohere",
        name: "Cohere",
        base_url: "https://api.cohere.com/compatibility",
        chat_path: "/v1/chat/completions",
        default_model: "command-r-plus-08-2024",
        free_tier: true,
    },
    // ── Tier 2: China region providers ───────────────────────────────────────
    KnownProvider {
        id: "moonshot",
        name: "Moonshot AI (Kimi)",
        base_url: "https://api.moonshot.cn",
        chat_path: "/v1/chat/completions",
        default_model: "moonshot-v1-8k",
        free_tier: false,
    },
    KnownProvider {
        id: "glm",
        name: "GLM (Zhipu AI)",
        base_url: "https://open.bigmodel.cn/api/paas",
        chat_path: "/v4/chat/completions",
        default_model: "glm-4-flash",
        free_tier: true,
    },
    KnownProvider {
        id: "doubao",
        name: "Doubao (ByteDance)",
        base_url: "https://ark.cn-beijing.volces.com/api",
        chat_path: "/v3/chat/completions",
        default_model: "doubao-pro-4k",
        free_tier: false,
    },
    KnownProvider {
        id: "qwen",
        name: "Qwen (Alibaba)",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode",
        chat_path: "/v1/chat/completions",
        default_model: "qwen-turbo",
        free_tier: false,
    },
    KnownProvider {
        id: "zai",
        name: "Z.AI",
        base_url: "https://api.z.ai",
        chat_path: "/v1/chat/completions",
        default_model: "z1-preview",
        free_tier: false,
    },
    KnownProvider {
        id: "yi",
        name: "01.AI (Yi)",
        base_url: "https://api.01.ai",
        chat_path: "/v1/chat/completions",
        default_model: "yi-large",
        free_tier: false,
    },
    KnownProvider {
        id: "minimax",
        name: "MiniMax",
        base_url: "https://api.minimax.chat",
        chat_path: "/v1/text/chatcompletion_v2",
        default_model: "MiniMax-Text-01",
        free_tier: false,
    },
    KnownProvider {
        id: "hunyuan",
        name: "Hunyuan (Tencent)",
        base_url: "https://api.hunyuan.cloud.tencent.com",
        chat_path: "/v1/chat/completions",
        default_model: "hunyuan-turbo",
        free_tier: false,
    },
    KnownProvider {
        id: "stepfun",
        name: "StepFun",
        base_url: "https://api.stepfun.com",
        chat_path: "/v1/chat/completions",
        default_model: "step-1-8k",
        free_tier: false,
    },
    // ── Google AI (Gemini) — OpenAI-compatible endpoint ────────────────────────
    KnownProvider {
        id: "gemini",
        name: "Google AI (Gemini)",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        chat_path: "/chat/completions",
        default_model: "gemini-2.0-flash",
        free_tier: true,
    },
    // ── Tier 3: Local / self-hosted ───────────────────────────────────────────
    KnownProvider {
        id: "lmstudio",
        name: "LM Studio (local)",
        base_url: "http://localhost:1234",
        chat_path: "/v1/chat/completions",
        default_model: "local-model",
        free_tier: true,
    },
    KnownProvider {
        id: "llamacpp",
        name: "llama.cpp server (local)",
        base_url: "http://localhost:8080",
        chat_path: "/v1/chat/completions",
        default_model: "local-model",
        free_tier: true,
    },
    KnownProvider {
        id: "localai",
        name: "LocalAI (local)",
        base_url: "http://localhost:8080",
        chat_path: "/v1/chat/completions",
        default_model: "gpt-4",
        free_tier: true,
    },
    KnownProvider {
        id: "litellm",
        name: "LiteLLM proxy",
        base_url: "http://localhost:4000",
        chat_path: "/v1/chat/completions",
        default_model: "gpt-3.5-turbo",
        free_tier: true,
    },
];

/// Look up a known provider by its ID.
pub fn lookup(id: &str) -> Option<&'static KnownProvider> {
    KNOWN_PROVIDERS.iter().find(|p| p.id == id)
}
