//! Voice transcription backends — converts Discord voice messages to text.
//!
//! Configured via `voice_transcription` in `[channels.discord]`:
//! - `"none"` — disabled (default)
//! - `"openai_whisper"` — OpenAI Whisper API (requires OPENAI_API_KEY)
//! - `"whisper_cpp"` — local whisper.cpp subprocess (requires `whisper` in PATH)

/// Transcription backend selection.
pub enum TranscriptionBackend {
    None,
    OpenAiWhisper,
    WhisperCpp,
}

impl TranscriptionBackend {
    /// Parse a config string into a transcription backend.
    pub fn from_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai_whisper" | "openai" | "whisper_api" => Self::OpenAiWhisper,
            "whisper_cpp" | "whisper" | "local" => Self::WhisperCpp,
            _ => Self::None,
        }
    }
}

/// Transcribe audio bytes using the configured backend.
pub async fn transcribe(
    backend: &TranscriptionBackend,
    audio_bytes: &[u8],
) -> Result<String, String> {
    match backend {
        TranscriptionBackend::None => Err(
            "Voice transcription not configured. Set voice_transcription in [channels.discord]."
                .to_string(),
        ),
        TranscriptionBackend::OpenAiWhisper => transcribe_openai(audio_bytes).await,
        TranscriptionBackend::WhisperCpp => transcribe_whisper_cpp(audio_bytes).await,
    }
}

/// Transcribe using the OpenAI Whisper API.
async fn transcribe_openai(audio_bytes: &[u8]) -> Result<String, String> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY not set for whisper transcription".to_string())?;

    let part = reqwest::multipart::Part::bytes(audio_bytes.to_vec())
        .file_name("audio.ogg")
        .mime_str("audio/ogg")
        .map_err(|e| e.to_string())?;

    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .part("file", part);

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Whisper API request failed: {}", e))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Whisper API error: {}", body));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    json["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No 'text' field in Whisper response".to_string())
}

/// Transcribe using a local whisper.cpp subprocess.
async fn transcribe_whisper_cpp(audio_bytes: &[u8]) -> Result<String, String> {
    use tokio::process::Command;

    let pid = std::process::id();
    let tmp_input = format!("/tmp/skynet_whisper_{}.ogg", pid);

    tokio::fs::write(&tmp_input, audio_bytes)
        .await
        .map_err(|e| format!("Failed to write temp audio: {}", e))?;

    let output = Command::new("whisper")
        .args([
            "--model",
            "base",
            "--output-format",
            "txt",
            "--output-dir",
            "/tmp",
            &tmp_input,
        ])
        .output()
        .await
        .map_err(|e| format!("whisper.cpp not found or failed to execute: {}", e))?;

    // Clean up input file.
    let _ = tokio::fs::remove_file(&tmp_input).await;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("whisper.cpp failed: {}", stderr));
    }

    let txt_output = format!("/tmp/skynet_whisper_{}.txt", pid);
    let text = tokio::fs::read_to_string(&txt_output)
        .await
        .map_err(|e| format!("Failed to read whisper output: {}", e))?;

    let _ = tokio::fs::remove_file(&txt_output).await;

    Ok(text.trim().to_string())
}
