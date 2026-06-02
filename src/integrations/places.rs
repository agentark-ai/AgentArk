//! Google Places Integration
//!
//! Provides access to the Google Places API (New) for searching places,
//! finding nearby locations, retrieving place details, and getting directions.
//! Uses API key authentication (not OAuth) for simplicity.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// Google Places API connector
pub struct GooglePlacesConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl GooglePlacesConnector {
    const API_BASE: &'static str = "https://places.googleapis.com/v1";
    const DIRECTIONS_BASE: &'static str = "https://maps.googleapis.com/maps/api/directions/json";

    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: crate::core::net::default_outgoing_http_client(),
            config_dir,
        }
    }

    pub fn new() -> Self {
        let config_dir = crate::branding::project_dirs()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        Self::new_with_config_dir(config_dir)
    }

    /// Load API key from environment variable or secure config
    fn load_token_from(config_dir: &Path) -> Option<String> {
        if let Ok(token) = std::env::var("GOOGLE_PLACES_API_KEY") {
            if !token.is_empty() {
                return Some(token);
            }
        }
        match crate::core::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager
                .get_custom_secret("google_places_api_key")
                .ok()
                .flatten(),
            Err(_) => None,
        }
    }

    /// Get the API key or return an error
    fn api_key(&self) -> Result<String> {
        Self::load_token_from(&self.config_dir).ok_or_else(|| {
            anyhow!("Google Places API key not configured. Set GOOGLE_PLACES_API_KEY or store via secure config.")
        })
    }

    /// POST /places:searchText - Search for places by text query
    async fn search(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let api_key = self.api_key()?;

        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'query' parameter"))?;

        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(20) as i32;

        let url = format!("{}:searchText", Self::API_BASE);

        let body = serde_json::json!({
            "textQuery": query,
            "maxResultCount": max_results,
        });

        let field_mask = "places.displayName,places.formattedAddress,places.rating,places.types,places.googleMapsUri";

        let response = self
            .http
            .post(&url)
            .header("X-Goog-Api-Key", api_key)
            .header("X-Goog-FieldMask", field_mask)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Google Places search failed ({}): {}", status, error_text);
            return Err(anyhow!(
                "Google Places API error ({}): {}",
                status,
                error_text
            ));
        }

        let result: serde_json::Value = response.json().await?;

        let places: Vec<serde_json::Value> = result
            .get("places")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|place| {
                serde_json::json!({
                    "name": place.get("displayName").and_then(|d| d.get("text")),
                    "address": place.get("formattedAddress"),
                    "rating": place.get("rating"),
                    "types": place.get("types"),
                    "maps_url": place.get("googleMapsUri"),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "places": places,
            "count": places.len(),
            "query": query,
        }))
    }

    /// POST /places:searchNearby - Find places near a location
    async fn nearby(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let api_key = self.api_key()?;

        let latitude = params
            .get("latitude")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow!("Missing 'latitude' parameter"))?;

        let longitude = params
            .get("longitude")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow!("Missing 'longitude' parameter"))?;

        let radius = params
            .get("radius")
            .and_then(|v| v.as_f64())
            .unwrap_or(5000.0);

        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(20) as i32;

        let url = format!("{}:searchNearby", Self::API_BASE);

        let body = serde_json::json!({
            "locationRestriction": {
                "circle": {
                    "center": {
                        "latitude": latitude,
                        "longitude": longitude,
                    },
                    "radius": radius,
                }
            },
            "maxResultCount": max_results,
        });

        let field_mask = "places.displayName,places.formattedAddress,places.rating,places.types,places.googleMapsUri,places.location";

        let response = self
            .http
            .post(&url)
            .header("X-Goog-Api-Key", api_key)
            .header("X-Goog-FieldMask", field_mask)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Google Places nearby failed ({}): {}", status, error_text);
            return Err(anyhow!(
                "Google Places API error ({}): {}",
                status,
                error_text
            ));
        }

        let result: serde_json::Value = response.json().await?;

        let places: Vec<serde_json::Value> = result
            .get("places")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|place| {
                serde_json::json!({
                    "name": place.get("displayName").and_then(|d| d.get("text")),
                    "address": place.get("formattedAddress"),
                    "rating": place.get("rating"),
                    "types": place.get("types"),
                    "maps_url": place.get("googleMapsUri"),
                    "location": place.get("location"),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "places": places,
            "count": places.len(),
            "center": {
                "latitude": latitude,
                "longitude": longitude,
            },
            "radius": radius,
        }))
    }

    /// GET /places/{place_id} - Get full details of a place
    async fn details(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let api_key = self.api_key()?;

        let place_id = params
            .get("place_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'place_id' parameter"))?;

        let url = format!("{}/places/{}", Self::API_BASE, place_id);

        let response = self
            .http
            .get(&url)
            .header("X-Goog-Api-Key", api_key)
            .header("X-Goog-FieldMask", "*")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Google Places details failed ({}): {}", status, error_text);
            return Err(anyhow!(
                "Google Places API error ({}): {}",
                status,
                error_text
            ));
        }

        let place: serde_json::Value = response.json().await?;

        Ok(serde_json::json!({
            "name": place.get("displayName").and_then(|d| d.get("text")),
            "address": place.get("formattedAddress"),
            "phone": place.get("nationalPhoneNumber"),
            "international_phone": place.get("internationalPhoneNumber"),
            "website": place.get("websiteUri"),
            "maps_url": place.get("googleMapsUri"),
            "rating": place.get("rating"),
            "user_rating_count": place.get("userRatingCount"),
            "price_level": place.get("priceLevel"),
            "types": place.get("types"),
            "location": place.get("location"),
            "opening_hours": place.get("regularOpeningHours"),
            "reviews": place.get("reviews"),
            "editorial_summary": place.get("editorialSummary"),
            "accessibility": place.get("accessibilityOptions"),
            "parking": place.get("parkingOptions"),
            "payment": place.get("paymentOptions"),
        }))
    }

    /// Directions via the legacy Directions API (uses API key auth)
    async fn directions(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let api_key = self.api_key()?;

        let origin = params
            .get("origin")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'origin' parameter"))?;

        let destination = params
            .get("destination")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'destination' parameter"))?;

        let mode = params
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("driving");

        // Validate mode
        let valid_modes = ["driving", "walking", "transit", "bicycling"];
        if !valid_modes.contains(&mode) {
            return Err(anyhow!(
                "Invalid travel mode '{}'. Must be one of: driving, walking, transit, bicycling",
                mode
            ));
        }

        let url = format!(
            "{}?origin={}&destination={}&mode={}&key={}",
            Self::DIRECTIONS_BASE,
            urlencoding::encode(origin),
            urlencoding::encode(destination),
            mode,
            api_key,
        );

        let response = self.http.get(&url).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Google Directions failed ({}): {}", status, error_text);
            return Err(anyhow!(
                "Google Directions API error ({}): {}",
                status,
                error_text
            ));
        }

        let body: serde_json::Value = response.json().await?;

        let api_status = body
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        if api_status != "OK" {
            let error_msg = body
                .get("error_message")
                .and_then(|e| e.as_str())
                .unwrap_or("No details");
            return Err(anyhow!(
                "Directions API status '{}': {}",
                api_status,
                error_msg
            ));
        }

        let routes = body
            .get("routes")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let directions: Vec<serde_json::Value> = routes
            .iter()
            .map(|route| {
                let legs = route
                    .get("legs")
                    .and_then(|l| l.as_array())
                    .cloned()
                    .unwrap_or_default();

                let leg_summaries: Vec<serde_json::Value> =
                    legs.iter()
                        .map(|leg| {
                            let steps: Vec<serde_json::Value> = leg.get("steps")
                    .and_then(|s| s.as_array())
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|step| {
                        serde_json::json!({
                            "instruction": step.get("html_instructions"),
                            "distance": step.get("distance").and_then(|d| d.get("text")),
                            "duration": step.get("duration").and_then(|d| d.get("text")),
                            "travel_mode": step.get("travel_mode"),
                        })
                    })
                    .collect();

                            serde_json::json!({
                                "start_address": leg.get("start_address"),
                                "end_address": leg.get("end_address"),
                                "distance": leg.get("distance").and_then(|d| d.get("text")),
                                "duration": leg.get("duration").and_then(|d| d.get("text")),
                                "steps": steps,
                            })
                        })
                        .collect();

                serde_json::json!({
                    "summary": route.get("summary"),
                    "legs": leg_summaries,
                    "warnings": route.get("warnings"),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "origin": origin,
            "destination": destination,
            "mode": mode,
            "routes": directions,
            "route_count": directions.len(),
        }))
    }
}

#[async_trait]
impl Integration for GooglePlacesConnector {
    fn id(&self) -> &str {
        "google_places"
    }

    fn name(&self) -> &str {
        "Google Places"
    }

    fn description(&self) -> &str {
        "Search places, find nearby locations, get details and directions via Google Places API"
    }

    fn icon(&self) -> &str {
        "📍"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        if Self::load_token_from(&self.config_dir).is_some() {
            IntegrationStatus::Connected
        } else {
            IntegrationStatus::NotConfigured
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "search" => self.search(params).await,
            "nearby" => self.nearby(params).await,
            "details" => self.details(params).await,
            "directions" => self.directions(params).await,
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for GooglePlacesConnector {
    fn default() -> Self {
        Self::new()
    }
}
