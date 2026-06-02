//! AI Media Generation Integration
//!
//! Supports image and video generation via multiple providers:
//! - Replicate (Flux, SDXL, Stable Video Diffusion)
//! - Stability AI (Stable Diffusion, Stable Video)
//! - FAL.ai (Fast inference)
//! - Together.ai (Open source models)
//! - OpenAI (DALL-E 3)
//! - Runway ML (Video generation)
//! - Luma AI (Dream Machine video)

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use zeroize::Zeroizing;

/// Supported media generation providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaProvider {
    /// Replicate - hosts Flux, SDXL, video models
    Replicate,
    /// Stability AI - Stable Diffusion, Stable Video
    StabilityAi,
    /// FAL.ai - Fast inference
    Fal,
    /// Together.ai - Open source models
    Together,
    /// OpenAI DALL-E 3
    OpenAiDalle,
    /// OpenAI Sora - Video generation
    OpenAiSora,
    /// Google Gemini / Nano Banana - Image generation via Gemini 2.5 Flash
    GoogleGemini,
    /// Google Veo - Video generation
    GoogleVeo,
    /// Runway ML - Gen-2, Gen-3 video
    Runway,
    /// Luma AI - Dream Machine
    Luma,
}

impl MediaProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Replicate => "replicate",
            Self::StabilityAi => "stability_ai",
            Self::Fal => "fal",
            Self::Together => "together",
            Self::OpenAiDalle => "openai_dalle",
            Self::OpenAiSora => "openai_sora",
            Self::GoogleGemini => "google_gemini",
            Self::GoogleVeo => "google_veo",
            Self::Runway => "runway",
            Self::Luma => "luma",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Replicate => "Replicate",
            Self::StabilityAi => "Stability AI",
            Self::Fal => "FAL.ai",
            Self::Together => "Together.ai",
            Self::OpenAiDalle => "OpenAI DALL-E",
            Self::OpenAiSora => "OpenAI Sora",
            Self::GoogleGemini => "Google Gemini (Nano Banana)",
            Self::GoogleVeo => "Google Veo",
            Self::Runway => "Runway ML",
            Self::Luma => "Luma AI",
        }
    }

    pub fn default_base_url(&self) -> &'static str {
        match self {
            MediaProvider::Replicate => "https://api.replicate.com/v1",
            MediaProvider::StabilityAi => "https://api.stability.ai/v1",
            MediaProvider::Fal => "https://fal.run",
            MediaProvider::Together => "https://api.together.xyz/v1",
            MediaProvider::OpenAiDalle => "https://api.openai.com/v1",
            MediaProvider::OpenAiSora => "https://api.openai.com/v1",
            MediaProvider::GoogleGemini => "https://generativelanguage.googleapis.com/v1beta",
            MediaProvider::GoogleVeo => "https://generativelanguage.googleapis.com/v1beta",
            MediaProvider::Runway => "https://api.runwayml.com/v1",
            MediaProvider::Luma => "https://api.lumalabs.ai/dream-machine/v1",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        match normalized.as_str() {
            "replicate" => Some(Self::Replicate),
            "stability_ai" | "stability" => Some(Self::StabilityAi),
            "fal" | "fal_ai" => Some(Self::Fal),
            "together" | "together_ai" => Some(Self::Together),
            "openai" | "openai_dalle" | "open_ai_dalle" | "dalle" | "dall_e" => {
                Some(Self::OpenAiDalle)
            }
            "openai_sora" | "open_ai_sora" | "sora" => Some(Self::OpenAiSora),
            "google" | "gemini" | "google_gemini" => Some(Self::GoogleGemini),
            "google_veo" | "veo" => Some(Self::GoogleVeo),
            "runway" | "runway_ml" => Some(Self::Runway),
            "luma" | "luma_ai" => Some(Self::Luma),
            _ => None,
        }
    }

    pub fn supports_video(&self) -> bool {
        matches!(
            self,
            Self::Replicate
                | Self::StabilityAi
                | Self::Runway
                | Self::Luma
                | Self::Fal
                | Self::OpenAiSora
                | Self::GoogleVeo
        )
    }

    pub fn supports_image(&self) -> bool {
        matches!(
            self,
            Self::Replicate
                | Self::StabilityAi
                | Self::Fal
                | Self::Together
                | Self::OpenAiDalle
                | Self::GoogleGemini
        )
    }
}

/// Image generation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenRequest {
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_steps")]
    pub steps: u32,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
}

fn default_width() -> u32 {
    1024
}
fn default_height() -> u32 {
    1024
}
fn default_steps() -> u32 {
    30
}

/// Video generation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenRequest {
    pub prompt: String,
    #[serde(default)]
    pub image_url: Option<String>, // For image-to-video
    #[serde(default = "default_duration")]
    pub duration_seconds: u32,
    #[serde(default)]
    pub aspect_ratio: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

fn default_duration() -> u32 {
    4
}

/// Generation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaGenResult {
    pub url: String,
    pub media_type: String, // "image" or "video"
    pub provider: String,
    pub model: String,
    pub seed: Option<i64>,
    pub generation_time_ms: Option<u64>,
}

/// Provider configuration
#[derive(Clone)]
struct ProviderConfig {
    api_key: Zeroizing<String>,
    base_url: String,
}

/// Media Generation connector
pub struct MediaGenConnector {
    providers: Arc<RwLock<std::collections::HashMap<MediaProvider, ProviderConfig>>>,
    http: reqwest::Client,
    default_image_provider: Arc<RwLock<Option<MediaProvider>>>,
    default_video_provider: Arc<RwLock<Option<MediaProvider>>>,
}

impl MediaGenConnector {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            http: crate::core::net::default_outgoing_http_client(),
            default_image_provider: Arc::new(RwLock::new(None)),
            default_video_provider: Arc::new(RwLock::new(None)),
        }
    }

    /// Configure a provider with an optional compatible endpoint override.
    pub async fn configure_provider_with_base_url(
        &self,
        provider: MediaProvider,
        api_key: String,
        base_url: Option<String>,
    ) {
        let base_url = base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_end_matches('/').to_string())
            .unwrap_or_else(|| provider.default_base_url().to_string());

        let mut providers = self.providers.write().await;
        providers.insert(
            provider,
            ProviderConfig {
                api_key: Zeroizing::new(api_key),
                base_url,
            },
        );

        // Set as default if first of its type
        if provider.supports_image() {
            let mut default = self.default_image_provider.write().await;
            if default.is_none() {
                *default = Some(provider);
            }
        }
        if provider.supports_video() {
            let mut default = self.default_video_provider.write().await;
            if default.is_none() {
                *default = Some(provider);
            }
        }
    }

    /// Set default provider for image generation
    pub async fn set_default_image_provider(&self, provider: MediaProvider) {
        *self.default_image_provider.write().await = Some(provider);
    }

    /// Set default provider for video generation
    pub async fn set_default_video_provider(&self, provider: MediaProvider) {
        *self.default_video_provider.write().await = Some(provider);
    }

    /// Generate an image
    pub async fn generate_image(
        &self,
        request: ImageGenRequest,
        provider: Option<MediaProvider>,
    ) -> Result<MediaGenResult> {
        let provider = provider
            .or(*self.default_image_provider.read().await)
            .ok_or_else(|| anyhow!("No image provider configured"))?;

        let providers = self.providers.read().await;
        let config = providers
            .get(&provider)
            .ok_or_else(|| anyhow!("Provider {:?} not configured", provider))?;

        let start = std::time::Instant::now();

        match provider {
            MediaProvider::Replicate => self.generate_image_replicate(&request, config).await,
            MediaProvider::StabilityAi => self.generate_image_stability(&request, config).await,
            MediaProvider::Fal => self.generate_image_fal(&request, config).await,
            MediaProvider::Together => self.generate_image_together(&request, config).await,
            MediaProvider::OpenAiDalle => self.generate_image_dalle(&request, config).await,
            MediaProvider::GoogleGemini => self.generate_image_gemini(&request, config).await,
            _ => Err(anyhow!(
                "Provider {:?} does not support image generation",
                provider
            )),
        }
        .map(|mut r| {
            r.generation_time_ms = Some(start.elapsed().as_millis() as u64);
            r
        })
    }

    /// Generate a video
    pub async fn generate_video(
        &self,
        request: VideoGenRequest,
        provider: Option<MediaProvider>,
    ) -> Result<MediaGenResult> {
        let provider = provider
            .or(*self.default_video_provider.read().await)
            .ok_or_else(|| anyhow!("No video provider configured"))?;

        let providers = self.providers.read().await;
        let config = providers
            .get(&provider)
            .ok_or_else(|| anyhow!("Provider {:?} not configured", provider))?;

        let start = std::time::Instant::now();

        match provider {
            MediaProvider::Replicate => self.generate_video_replicate(&request, config).await,
            MediaProvider::StabilityAi => self.generate_video_stability(&request, config).await,
            MediaProvider::Runway => self.generate_video_runway(&request, config).await,
            MediaProvider::Luma => self.generate_video_luma(&request, config).await,
            MediaProvider::Fal => self.generate_video_fal(&request, config).await,
            MediaProvider::OpenAiSora => self.generate_video_sora(&request, config).await,
            MediaProvider::GoogleVeo => self.generate_video_veo(&request, config).await,
            _ => Err(anyhow!(
                "Provider {:?} does not support video generation",
                provider
            )),
        }
        .map(|mut r| {
            r.generation_time_ms = Some(start.elapsed().as_millis() as u64);
            r
        })
    }

    // === Replicate Implementation ===

    async fn generate_image_replicate(
        &self,
        request: &ImageGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        // Default to Flux Schnell for fast generation
        let model = request
            .model
            .as_deref()
            .unwrap_or("black-forest-labs/flux-schnell");

        let input = serde_json::json!({
            "prompt": request.prompt,
            "width": request.width,
            "height": request.height,
            "num_inference_steps": request.steps,
            "seed": request.seed,
        });

        let body = serde_json::json!({
            "version": model,
            "input": input
        });

        let response = self
            .http
            .post(format!("{}/predictions", config.base_url))
            .header(
                "Authorization",
                format!("Token {}", config.api_key.as_str()),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Replicate error: {}", error));
        }

        #[derive(Deserialize)]
        struct PredictionResponse {
            #[serde(rename = "id")]
            _id: String,
            urls: PredictionUrls,
        }

        #[derive(Deserialize)]
        struct PredictionUrls {
            get: String,
        }

        let prediction: PredictionResponse = response.json().await?;

        // Poll for completion
        let result = self.poll_replicate(&prediction.urls.get, config).await?;

        Ok(MediaGenResult {
            url: result,
            media_type: "image".to_string(),
            provider: "Replicate".to_string(),
            model: model.to_string(),
            seed: request.seed,
            generation_time_ms: None,
        })
    }

    async fn generate_video_replicate(
        &self,
        request: &VideoGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        // Default to Stable Video Diffusion
        let model = request
            .model
            .as_deref()
            .unwrap_or("stability-ai/stable-video-diffusion");

        let mut input = serde_json::json!({
            "prompt": request.prompt,
        });

        if let Some(ref img) = request.image_url {
            input["image"] = serde_json::json!(img);
        }

        let body = serde_json::json!({
            "version": model,
            "input": input
        });

        let response = self
            .http
            .post(format!("{}/predictions", config.base_url))
            .header(
                "Authorization",
                format!("Token {}", config.api_key.as_str()),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Replicate error: {}", error));
        }

        #[derive(Deserialize)]
        struct PredictionResponse {
            urls: PredictionUrls,
        }

        #[derive(Deserialize)]
        struct PredictionUrls {
            get: String,
        }

        let prediction: PredictionResponse = response.json().await?;
        let result = self.poll_replicate(&prediction.urls.get, config).await?;

        Ok(MediaGenResult {
            url: result,
            media_type: "video".to_string(),
            provider: "Replicate".to_string(),
            model: model.to_string(),
            seed: None,
            generation_time_ms: None,
        })
    }

    async fn poll_replicate(&self, url: &str, config: &ProviderConfig) -> Result<String> {
        for _ in 0..120 {
            // Max 2 minutes
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            let response = self
                .http
                .get(url)
                .header(
                    "Authorization",
                    format!("Token {}", config.api_key.as_str()),
                )
                .send()
                .await?;

            #[derive(Deserialize)]
            struct PollResponse {
                status: String,
                output: Option<serde_json::Value>,
                error: Option<String>,
            }

            let poll: PollResponse = response.json().await?;

            match poll.status.as_str() {
                "succeeded" => {
                    if let Some(output) = poll.output {
                        // Output can be string or array
                        if let Some(url) = output.as_str() {
                            return Ok(url.to_string());
                        } else if let Some(arr) = output.as_array() {
                            if let Some(first) = arr.first().and_then(|v| v.as_str()) {
                                return Ok(first.to_string());
                            }
                        }
                    }
                    return Err(anyhow!("No output URL in response"));
                }
                "failed" => {
                    return Err(anyhow!(
                        "Generation failed: {}",
                        poll.error.unwrap_or_default()
                    ));
                }
                "canceled" => {
                    return Err(anyhow!("Generation was canceled"));
                }
                _ => continue, // starting, processing
            }
        }

        Err(anyhow!("Generation timed out"))
    }

    // === Stability AI Implementation ===

    async fn generate_image_stability(
        &self,
        request: &ImageGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        let engine = request
            .model
            .as_deref()
            .unwrap_or("stable-diffusion-xl-1024-v1-0");

        let body = serde_json::json!({
            "text_prompts": [{
                "text": request.prompt,
                "weight": 1.0
            }],
            "cfg_scale": 7,
            "height": request.height,
            "width": request.width,
            "steps": request.steps,
            "seed": request.seed.unwrap_or(0),
        });

        let response = self
            .http
            .post(format!(
                "{}/generation/{}/text-to-image",
                config.base_url, engine
            ))
            .header(
                "Authorization",
                format!("Bearer {}", config.api_key.as_str()),
            )
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Stability AI error: {}", error));
        }

        #[derive(Deserialize)]
        struct StabilityResponse {
            artifacts: Vec<Artifact>,
        }

        #[derive(Deserialize)]
        struct Artifact {
            base64: String,
            seed: u64,
        }

        let result: StabilityResponse = response.json().await?;
        let artifact = result
            .artifacts
            .first()
            .ok_or_else(|| anyhow!("No image generated"))?;

        // Return as data URL
        let url = format!("data:image/png;base64,{}", artifact.base64);

        Ok(MediaGenResult {
            url,
            media_type: "image".to_string(),
            provider: "Stability AI".to_string(),
            model: engine.to_string(),
            seed: Some(artifact.seed as i64),
            generation_time_ms: None,
        })
    }

    async fn generate_video_stability(
        &self,
        request: &VideoGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        // Stable Video Diffusion - requires an image input
        let image_url = request
            .image_url
            .as_ref()
            .ok_or_else(|| anyhow!("Stability video requires an input image"))?;

        let body = serde_json::json!({
            "image": image_url,
            "seed": 0,
            "cfg_scale": 2.5,
            "motion_bucket_id": 127,
        });

        let response = self
            .http
            .post(format!("{}/generation/image-to-video", config.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", config.api_key.as_str()),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Stability AI error: {}", error));
        }

        #[derive(Deserialize)]
        struct VideoResponse {
            id: String,
        }

        let result: VideoResponse = response.json().await?;

        // Poll for completion
        let video_url = self.poll_stability_video(&result.id, config).await?;

        Ok(MediaGenResult {
            url: video_url,
            media_type: "video".to_string(),
            provider: "Stability AI".to_string(),
            model: "stable-video-diffusion".to_string(),
            seed: None,
            generation_time_ms: None,
        })
    }

    async fn poll_stability_video(&self, id: &str, config: &ProviderConfig) -> Result<String> {
        for _ in 0..180 {
            // Max 3 minutes
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            let response = self
                .http
                .get(format!(
                    "{}/generation/image-to-video/result/{}",
                    config.base_url, id
                ))
                .header(
                    "Authorization",
                    format!("Bearer {}", config.api_key.as_str()),
                )
                .header("Accept", "application/json")
                .send()
                .await?;

            if response.status().as_u16() == 202 {
                continue; // Still processing
            }

            if response.status().is_success() {
                #[derive(Deserialize)]
                struct VideoResult {
                    video: String, // base64
                }

                let result: VideoResult = response.json().await?;
                return Ok(format!("data:video/mp4;base64,{}", result.video));
            }

            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Video generation failed: {}", error));
        }

        Err(anyhow!("Video generation timed out"))
    }

    // === FAL.ai Implementation ===

    async fn generate_image_fal(
        &self,
        request: &ImageGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        let model = request.model.as_deref().unwrap_or("fal-ai/flux/schnell");

        let body = serde_json::json!({
            "prompt": request.prompt,
            "image_size": {
                "width": request.width,
                "height": request.height
            },
            "num_inference_steps": request.steps,
            "seed": request.seed,
        });

        let response = self
            .http
            .post(format!("{}/{}", config.base_url, model))
            .header("Authorization", format!("Key {}", config.api_key.as_str()))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("FAL.ai error: {}", error));
        }

        #[derive(Deserialize)]
        struct FalResponse {
            images: Vec<FalImage>,
            seed: Option<i64>,
        }

        #[derive(Deserialize)]
        struct FalImage {
            url: String,
        }

        let result: FalResponse = response.json().await?;
        let image = result
            .images
            .first()
            .ok_or_else(|| anyhow!("No image generated"))?;

        Ok(MediaGenResult {
            url: image.url.clone(),
            media_type: "image".to_string(),
            provider: "FAL.ai".to_string(),
            model: model.to_string(),
            seed: result.seed,
            generation_time_ms: None,
        })
    }

    async fn generate_video_fal(
        &self,
        request: &VideoGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        let model = request.model.as_deref().unwrap_or("fal-ai/fast-svd-lcm");

        let mut body = serde_json::json!({
            "prompt": request.prompt,
        });

        if let Some(ref img) = request.image_url {
            body["image_url"] = serde_json::json!(img);
        }

        let response = self
            .http
            .post(format!("{}/{}", config.base_url, model))
            .header("Authorization", format!("Key {}", config.api_key.as_str()))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("FAL.ai error: {}", error));
        }

        #[derive(Deserialize)]
        struct FalVideoResponse {
            video: FalVideo,
        }

        #[derive(Deserialize)]
        struct FalVideo {
            url: String,
        }

        let result: FalVideoResponse = response.json().await?;

        Ok(MediaGenResult {
            url: result.video.url,
            media_type: "video".to_string(),
            provider: "FAL.ai".to_string(),
            model: model.to_string(),
            seed: None,
            generation_time_ms: None,
        })
    }

    // === Together.ai Implementation ===

    async fn generate_image_together(
        &self,
        request: &ImageGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        let model = request
            .model
            .as_deref()
            .unwrap_or("stabilityai/stable-diffusion-xl-base-1.0");

        let body = serde_json::json!({
            "model": model,
            "prompt": request.prompt,
            "negative_prompt": request.negative_prompt,
            "width": request.width,
            "height": request.height,
            "steps": request.steps,
            "seed": request.seed,
            "n": 1
        });

        let response = self
            .http
            .post(format!("{}/images/generations", config.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", config.api_key.as_str()),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Together.ai error: {}", error));
        }

        #[derive(Deserialize)]
        struct TogetherResponse {
            data: Vec<TogetherImage>,
        }

        #[derive(Deserialize)]
        struct TogetherImage {
            url: Option<String>,
            b64_json: Option<String>,
        }

        let result: TogetherResponse = response.json().await?;
        let image = result
            .data
            .first()
            .ok_or_else(|| anyhow!("No image generated"))?;

        let url = if let Some(ref u) = image.url {
            u.clone()
        } else if let Some(ref b64) = image.b64_json {
            format!("data:image/png;base64,{}", b64)
        } else {
            return Err(anyhow!("No image URL in response"));
        };

        Ok(MediaGenResult {
            url,
            media_type: "image".to_string(),
            provider: "Together.ai".to_string(),
            model: model.to_string(),
            seed: request.seed,
            generation_time_ms: None,
        })
    }

    // === OpenAI DALL-E Implementation ===

    async fn generate_image_dalle(
        &self,
        request: &ImageGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        let model = request.model.as_deref().unwrap_or("dall-e-3");

        // DALL-E 3 only supports specific sizes
        let size = match (request.width, request.height) {
            (w, h) if w >= 1792 || h >= 1792 => "1792x1024",
            (w, h) if w == h => "1024x1024",
            _ => "1024x1024",
        };

        let body = serde_json::json!({
            "model": model,
            "prompt": request.prompt,
            "size": size,
            "quality": "standard",
            "n": 1
        });

        let response = self
            .http
            .post(format!("{}/images/generations", config.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", config.api_key.as_str()),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("OpenAI error: {}", error));
        }

        #[derive(Deserialize)]
        struct DalleResponse {
            data: Vec<DalleImage>,
        }

        #[derive(Deserialize)]
        struct DalleImage {
            url: Option<String>,
            b64_json: Option<String>,
            #[serde(rename = "revised_prompt")]
            _revised_prompt: Option<String>,
        }

        let result: DalleResponse = response.json().await?;
        let image = result
            .data
            .first()
            .ok_or_else(|| anyhow!("No image generated"))?;

        let url = image
            .url
            .clone()
            .or_else(|| {
                image
                    .b64_json
                    .as_ref()
                    .map(|data| format!("data:image/png;base64,{}", data))
            })
            .unwrap_or_default();

        Ok(MediaGenResult {
            url,
            media_type: "image".to_string(),
            provider: "OpenAI".to_string(),
            model: model.to_string(),
            seed: None,
            generation_time_ms: None,
        })
    }

    // === Runway ML Implementation ===

    async fn generate_video_runway(
        &self,
        request: &VideoGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        let model = request.model.as_deref().unwrap_or("gen3a_turbo");

        let mut body = serde_json::json!({
            "promptText": request.prompt,
            "model": model,
            "duration": request.duration_seconds.min(10), // Runway max 10s
        });

        if let Some(ref img) = request.image_url {
            body["promptImage"] = serde_json::json!(img);
        }

        if let Some(ref ratio) = request.aspect_ratio {
            body["ratio"] = serde_json::json!(ratio);
        }

        let response = self
            .http
            .post(format!("{}/v1/image_to_video", config.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", config.api_key.as_str()),
            )
            .header("X-Runway-Version", "2024-11-06")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Runway error: {}", error));
        }

        #[derive(Deserialize)]
        struct RunwayResponse {
            id: String,
        }

        let result: RunwayResponse = response.json().await?;

        // Poll for completion
        let video_url = self.poll_runway(&result.id, config).await?;

        Ok(MediaGenResult {
            url: video_url,
            media_type: "video".to_string(),
            provider: "Runway ML".to_string(),
            model: model.to_string(),
            seed: None,
            generation_time_ms: None,
        })
    }

    async fn poll_runway(&self, id: &str, config: &ProviderConfig) -> Result<String> {
        for _ in 0..300 {
            // Max 5 minutes
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            let response = self
                .http
                .get(format!("{}/v1/tasks/{}", config.base_url, id))
                .header(
                    "Authorization",
                    format!("Bearer {}", config.api_key.as_str()),
                )
                .header("X-Runway-Version", "2024-11-06")
                .send()
                .await?;

            #[derive(Deserialize)]
            struct TaskResponse {
                status: String,
                output: Option<Vec<String>>,
                failure: Option<String>,
            }

            let task: TaskResponse = response.json().await?;

            match task.status.as_str() {
                "SUCCEEDED" => {
                    if let Some(outputs) = task.output {
                        if let Some(url) = outputs.first() {
                            return Ok(url.clone());
                        }
                    }
                    return Err(anyhow!("No output URL"));
                }
                "FAILED" => {
                    return Err(anyhow!(
                        "Generation failed: {}",
                        task.failure.unwrap_or_default()
                    ));
                }
                _ => continue,
            }
        }

        Err(anyhow!("Generation timed out"))
    }

    // === Luma AI Implementation ===

    async fn generate_video_luma(
        &self,
        request: &VideoGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        let mut body = serde_json::json!({
            "prompt": request.prompt,
        });

        if let Some(ref img) = request.image_url {
            body["keyframes"] = serde_json::json!({
                "frame0": {
                    "type": "image",
                    "url": img
                }
            });
        }

        if let Some(ref ratio) = request.aspect_ratio {
            body["aspect_ratio"] = serde_json::json!(ratio);
        }

        let response = self
            .http
            .post(format!("{}/generations", config.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", config.api_key.as_str()),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Luma AI error: {}", error));
        }

        #[derive(Deserialize)]
        struct LumaResponse {
            id: String,
        }

        let result: LumaResponse = response.json().await?;

        // Poll for completion
        let video_url = self.poll_luma(&result.id, config).await?;

        Ok(MediaGenResult {
            url: video_url,
            media_type: "video".to_string(),
            provider: "Luma AI".to_string(),
            model: "dream-machine".to_string(),
            seed: None,
            generation_time_ms: None,
        })
    }

    async fn poll_luma(&self, id: &str, config: &ProviderConfig) -> Result<String> {
        for _ in 0..300 {
            // Max 5 minutes
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            let response = self
                .http
                .get(format!("{}/generations/{}", config.base_url, id))
                .header(
                    "Authorization",
                    format!("Bearer {}", config.api_key.as_str()),
                )
                .send()
                .await?;

            #[derive(Deserialize)]
            struct GenerationResponse {
                state: String,
                assets: Option<Assets>,
                failure_reason: Option<String>,
            }

            #[derive(Deserialize)]
            struct Assets {
                video: Option<String>,
            }

            let generation: GenerationResponse = response.json().await?;

            match generation.state.as_str() {
                "completed" => {
                    if let Some(assets) = generation.assets {
                        if let Some(video) = assets.video {
                            return Ok(video);
                        }
                    }
                    return Err(anyhow!("No video URL in response"));
                }
                "failed" => {
                    return Err(anyhow!(
                        "Generation failed: {}",
                        generation.failure_reason.unwrap_or_default()
                    ));
                }
                _ => continue,
            }
        }

        Err(anyhow!("Generation timed out"))
    }

    // === OpenAI Sora Implementation ===

    async fn generate_video_sora(
        &self,
        request: &VideoGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        // Sora API (when available) - using OpenAI's video generation endpoint
        let model = request.model.as_deref().unwrap_or("sora");

        let mut body = serde_json::json!({
            "model": model,
            "prompt": request.prompt,
            "duration": request.duration_seconds.min(20), // Sora max ~20s
        });

        if let Some(ref ratio) = request.aspect_ratio {
            body["aspect_ratio"] = serde_json::json!(ratio);
        }

        // Note: Sora API is currently in limited access
        // This implementation follows the expected API pattern
        let response = self
            .http
            .post(format!("{}/videos/generations", config.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", config.api_key.as_str()),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("OpenAI Sora error: {}", error));
        }

        #[derive(Deserialize)]
        struct SoraResponse {
            id: String,
        }

        let result: SoraResponse = response.json().await?;

        // Poll for completion
        let video_url = self.poll_sora(&result.id, config).await?;

        Ok(MediaGenResult {
            url: video_url,
            media_type: "video".to_string(),
            provider: "OpenAI Sora".to_string(),
            model: model.to_string(),
            seed: None,
            generation_time_ms: None,
        })
    }

    async fn poll_sora(&self, id: &str, config: &ProviderConfig) -> Result<String> {
        for _ in 0..600 {
            // Max 10 minutes (Sora can take a while)
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            let response = self
                .http
                .get(format!("{}/videos/generations/{}", config.base_url, id))
                .header(
                    "Authorization",
                    format!("Bearer {}", config.api_key.as_str()),
                )
                .send()
                .await?;

            #[derive(Deserialize)]
            struct StatusResponse {
                status: String,
                video_url: Option<String>,
                error: Option<String>,
            }

            let status: StatusResponse = response.json().await?;

            match status.status.as_str() {
                "completed" | "succeeded" => {
                    if let Some(url) = status.video_url {
                        return Ok(url);
                    }
                    return Err(anyhow!("No video URL in response"));
                }
                "failed" => {
                    return Err(anyhow!(
                        "Sora generation failed: {}",
                        status.error.unwrap_or_default()
                    ));
                }
                _ => continue, // processing, pending
            }
        }

        Err(anyhow!("Sora generation timed out"))
    }

    // === Google Gemini (Nano Banana) Implementation ===

    async fn generate_image_gemini(
        &self,
        request: &ImageGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        // Google Gemini native image generation (Nano Banana)
        let model = request
            .model
            .as_deref()
            .unwrap_or("gemini-2.0-flash-preview-image-generation");

        let body = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": request.prompt
                }]
            }],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"]
            }
        });

        let response = self
            .http
            .post(format!(
                "{}/models/{}:generateContent?key={}",
                config.base_url,
                model,
                config.api_key.as_str()
            ))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Google Gemini error: {}", error));
        }

        #[derive(Deserialize)]
        struct GeminiResponse {
            candidates: Vec<Candidate>,
        }

        #[derive(Deserialize)]
        struct Candidate {
            content: Content,
        }

        #[derive(Deserialize)]
        struct Content {
            parts: Vec<Part>,
        }

        #[derive(Deserialize)]
        struct Part {
            #[serde(rename = "inlineData")]
            inline_data: Option<InlineData>,
        }

        #[derive(Deserialize)]
        struct InlineData {
            #[serde(rename = "mimeType")]
            mime_type: String,
            data: String,
        }

        let result: GeminiResponse = response.json().await?;

        // Find the image part in the response
        for candidate in result.candidates {
            for part in candidate.content.parts {
                if let Some(inline_data) = part.inline_data {
                    let url = format!("data:{};base64,{}", inline_data.mime_type, inline_data.data);
                    return Ok(MediaGenResult {
                        url,
                        media_type: "image".to_string(),
                        provider: "Google Gemini".to_string(),
                        model: model.to_string(),
                        seed: None,
                        generation_time_ms: None,
                    });
                }
            }
        }

        Err(anyhow!("No image in Gemini response"))
    }

    // === Google Veo Implementation ===

    async fn generate_video_veo(
        &self,
        request: &VideoGenRequest,
        config: &ProviderConfig,
    ) -> Result<MediaGenResult> {
        // Google Veo video generation
        let model = request.model.as_deref().unwrap_or("veo-001");

        let mut body = serde_json::json!({
            "prompt": request.prompt,
            "duration_seconds": request.duration_seconds.min(8), // Veo typically max 8s
        });

        if let Some(ref ratio) = request.aspect_ratio {
            body["aspect_ratio"] = serde_json::json!(ratio);
        }

        if let Some(ref img) = request.image_url {
            body["image"] = serde_json::json!(img);
        }

        let response = self
            .http
            .post(format!(
                "{}/models/{}:generateVideo?key={}",
                config.base_url,
                model,
                config.api_key.as_str()
            ))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(anyhow!("Google Veo error: {}", error));
        }

        #[derive(Deserialize)]
        struct VeoResponse {
            name: String, // Operation name for polling
        }

        let result: VeoResponse = response.json().await?;

        // Poll for completion
        let video_url = self.poll_veo(&result.name, config).await?;

        Ok(MediaGenResult {
            url: video_url,
            media_type: "video".to_string(),
            provider: "Google Veo".to_string(),
            model: model.to_string(),
            seed: None,
            generation_time_ms: None,
        })
    }

    async fn poll_veo(&self, operation_name: &str, config: &ProviderConfig) -> Result<String> {
        for _ in 0..300 {
            // Max 5 minutes
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            let response = self
                .http
                .get(format!(
                    "{}/{}?key={}",
                    config.base_url,
                    operation_name,
                    config.api_key.as_str()
                ))
                .send()
                .await?;

            #[derive(Deserialize)]
            struct OperationResponse {
                done: Option<bool>,
                response: Option<VideoResult>,
                error: Option<serde_json::Value>,
            }

            #[derive(Deserialize)]
            struct VideoResult {
                #[serde(rename = "generatedVideos")]
                generated_videos: Option<Vec<GeneratedVideo>>,
            }

            #[derive(Deserialize)]
            struct GeneratedVideo {
                video: Option<VideoData>,
            }

            #[derive(Deserialize)]
            struct VideoData {
                uri: Option<String>,
            }

            let op: OperationResponse = response.json().await?;

            if op.done.unwrap_or(false) {
                if let Some(err) = op.error {
                    return Err(anyhow!("Veo generation failed: {:?}", err));
                }
                if let Some(resp) = op.response {
                    if let Some(videos) = resp.generated_videos {
                        if let Some(video) = videos.first() {
                            if let Some(ref data) = video.video {
                                if let Some(ref uri) = data.uri {
                                    return Ok(uri.clone());
                                }
                            }
                        }
                    }
                }
                return Err(anyhow!("No video URL in response"));
            }
        }

        Err(anyhow!("Veo generation timed out"))
    }

    /// List configured providers
    pub async fn list_providers(&self) -> Vec<(MediaProvider, bool, bool)> {
        let providers = self.providers.read().await;
        let mut result = vec![];

        for provider in [
            MediaProvider::Replicate,
            MediaProvider::StabilityAi,
            MediaProvider::Fal,
            MediaProvider::Together,
            MediaProvider::OpenAiDalle,
            MediaProvider::OpenAiSora,
            MediaProvider::GoogleGemini,
            MediaProvider::GoogleVeo,
            MediaProvider::Runway,
            MediaProvider::Luma,
        ] {
            let configured = providers.contains_key(&provider);
            result.push((provider, configured, provider.supports_image()));
        }

        result
    }
}

#[async_trait]
impl Integration for MediaGenConnector {
    fn id(&self) -> &str {
        "media_gen"
    }

    fn name(&self) -> &str {
        "AI Media Generation"
    }

    fn description(&self) -> &str {
        "Generate images and videos with AI (Flux, SDXL, Runway, Luma, DALL-E)"
    }

    fn icon(&self) -> &str {
        "🎨"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Write]
    }

    async fn status(&self) -> IntegrationStatus {
        let providers = self.providers.read().await;
        if providers.is_empty() {
            IntegrationStatus::NotConfigured
        } else {
            IntegrationStatus::Connected
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "generate_image" => {
                let request: ImageGenRequest = serde_json::from_value(params.clone())?;
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .and_then(MediaProvider::parse);

                let result = self.generate_image(request, provider).await?;
                Ok(serde_json::to_value(result)?)
            }
            "generate_video" => {
                let request: VideoGenRequest = serde_json::from_value(params.clone())?;
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .and_then(MediaProvider::parse);

                let result = self.generate_video(request, provider).await?;
                Ok(serde_json::to_value(result)?)
            }
            "configure_provider" => {
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .and_then(MediaProvider::parse)
                    .ok_or_else(|| anyhow!("Unknown media provider"))?;
                let api_key = params
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing api_key"))?;
                let base_url = params
                    .get("base_url")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());

                self.configure_provider_with_base_url(provider, api_key.to_string(), base_url)
                    .await;
                Ok(serde_json::json!({"status": "configured"}))
            }
            "list_providers" => {
                let providers = self.list_providers().await;
                let list: Vec<_> = providers
                    .iter()
                    .map(|(p, configured, supports_image)| {
                        serde_json::json!({
                            "provider": p.id(),
                            "name": p.name(),
                            "configured": configured,
                            "supports_image": supports_image,
                            "supports_video": p.supports_video(),
                        })
                    })
                    .collect();
                Ok(serde_json::json!({"providers": list}))
            }
            "set_default_image_provider" => {
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .and_then(MediaProvider::parse)
                    .ok_or_else(|| anyhow!("Unknown media provider"))?;
                self.set_default_image_provider(provider).await;
                Ok(serde_json::json!({"status": "ok"}))
            }
            "set_default_video_provider" => {
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .and_then(MediaProvider::parse)
                    .ok_or_else(|| anyhow!("Unknown media provider"))?;
                self.set_default_video_provider(provider).await;
                Ok(serde_json::json!({"status": "ok"}))
            }
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for MediaGenConnector {
    fn default() -> Self {
        Self::new()
    }
}
