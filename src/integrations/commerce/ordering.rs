//! Generic Ordering / Purchasing Integration
//!
//! Supports placing orders, searching products, and tracking order status
//! via either Shopify Admin API or a custom webhook-based backend.
//! Provider selection is driven by environment variables or encrypted config.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Which ordering backend to use
enum OrderingProvider {
    /// No provider configured
    None,
    /// Shopify Admin API
    Shopify {
        access_token: String,
        store_url: String,
    },
    /// Custom webhook endpoint
    Webhook {
        url: String,
        headers: HashMap<String, String>,
    },
}

/// Generic ordering connector
pub struct OrderingConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl OrderingConnector {
    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: crate::core::runtime::net::default_outgoing_http_client(),
            config_dir,
        }
    }

    pub fn new() -> Self {
        let config_dir = crate::branding::project_dirs()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        Self::new_with_config_dir(config_dir)
    }

    /// Determine the ordering provider from env vars and secure config
    fn load_provider(config_dir: &Path) -> OrderingProvider {
        // Check env-based provider selection first
        let provider_name = std::env::var("ORDERING_PROVIDER")
            .unwrap_or_default()
            .to_lowercase();

        match provider_name.as_str() {
            "shopify" => {
                if let Some(provider) = Self::load_shopify_from_env(config_dir) {
                    return provider;
                }
            }
            "webhook" => {
                if let Some(provider) = Self::load_webhook_from_env(config_dir) {
                    return provider;
                }
            }
            _ => {}
        }

        // Fall back to secure config JSON blob
        if let Some(provider) = Self::load_from_config_json(config_dir) {
            return provider;
        }

        // Try to auto-detect from individual env vars even without ORDERING_PROVIDER
        if let Some(provider) = Self::load_shopify_from_env(config_dir) {
            return provider;
        }
        if let Some(provider) = Self::load_webhook_from_env(config_dir) {
            return provider;
        }

        OrderingProvider::None
    }

    fn load_shopify_from_env(config_dir: &Path) -> Option<OrderingProvider> {
        let access_token =
            Self::load_token(config_dir, "SHOPIFY_ACCESS_TOKEN", "shopify_access_token")?;
        let store_url = Self::load_token(config_dir, "SHOPIFY_STORE_URL", "shopify_store_url")?;
        Some(OrderingProvider::Shopify {
            access_token,
            store_url,
        })
    }

    fn load_webhook_from_env(config_dir: &Path) -> Option<OrderingProvider> {
        let url = Self::load_token(config_dir, "ORDERING_WEBHOOK_URL", "ordering_webhook_url")?;
        Some(OrderingProvider::Webhook {
            url,
            headers: HashMap::new(),
        })
    }

    /// Load a single token: env var first, then SecureConfigManager custom secret
    fn load_token(config_dir: &Path, env_var: &str, secret_key: &str) -> Option<String> {
        if let Ok(val) = std::env::var(env_var) {
            if !val.is_empty() {
                return Some(val);
            }
        }
        match crate::core::runtime::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager.get_custom_secret(secret_key).ok().flatten(),
            Err(_) => None,
        }
    }

    /// Load provider config from the encrypted `ordering_config` custom secret (JSON)
    fn load_from_config_json(config_dir: &Path) -> Option<OrderingProvider> {
        // Support both newer and README/legacy env var names.
        let json_str = Self::load_token(config_dir, "ORDERING_CONFIG_JSON", "ordering_config")
            .or_else(|| Self::load_token(config_dir, "ORDERING_CONFIG", "ordering_config"))?;

        #[derive(Deserialize)]
        struct OrderingConfigJson {
            provider: String,
            #[serde(default)]
            access_token: Option<String>,
            #[serde(default)]
            store_url: Option<String>,
            #[serde(default)]
            webhook_url: Option<String>,
            #[serde(default)]
            webhook_headers: Option<HashMap<String, String>>,
        }

        let cfg: OrderingConfigJson = match serde_json::from_str(&json_str) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to parse ordering_config JSON: {}", e);
                return None;
            }
        };

        match cfg.provider.as_str() {
            "shopify" => {
                let access_token = cfg.access_token?;
                let store_url = cfg.store_url?;
                Some(OrderingProvider::Shopify {
                    access_token,
                    store_url,
                })
            }
            "webhook" => {
                let url = cfg.webhook_url?;
                Some(OrderingProvider::Webhook {
                    url,
                    headers: cfg.webhook_headers.unwrap_or_default(),
                })
            }
            other => {
                tracing::warn!("Unknown ordering provider in config: {}", other);
                None
            }
        }
    }

    // ── Action handlers ──────────────────────────────────────────────────

    /// Search for products
    async fn search_products(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");

        match Self::load_provider(&self.config_dir) {
            OrderingProvider::Shopify {
                access_token,
                store_url,
            } => {
                self.shopify_search_products(&store_url, &access_token, query)
                    .await
            }
            OrderingProvider::Webhook { url, headers } => {
                self.webhook_search_products(&url, &headers, query).await
            }
            OrderingProvider::None => Err(anyhow!("No ordering provider configured")),
        }
    }

    /// Create a new order
    async fn create_order(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        match Self::load_provider(&self.config_dir) {
            OrderingProvider::Shopify {
                access_token,
                store_url,
            } => {
                self.shopify_create_order(&store_url, &access_token, params)
                    .await
            }
            OrderingProvider::Webhook { url, headers } => {
                self.webhook_create_order(&url, &headers, params).await
            }
            OrderingProvider::None => Err(anyhow!("No ordering provider configured")),
        }
    }

    /// Check order status
    async fn order_status(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let order_id = params
            .get("order_id")
            .or_else(|| params.get("id"))
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            })
            .ok_or_else(|| anyhow!("Missing 'order_id' parameter"))?;

        match Self::load_provider(&self.config_dir) {
            OrderingProvider::Shopify {
                access_token,
                store_url,
            } => {
                self.shopify_order_status(&store_url, &access_token, &order_id)
                    .await
            }
            OrderingProvider::Webhook { url, headers } => {
                self.webhook_order_status(&url, &headers, &order_id).await
            }
            OrderingProvider::None => Err(anyhow!("No ordering provider configured")),
        }
    }

    /// List recent orders
    async fn list_orders(&self, _params: &serde_json::Value) -> Result<serde_json::Value> {
        match Self::load_provider(&self.config_dir) {
            OrderingProvider::Shopify {
                access_token,
                store_url,
            } => self.shopify_list_orders(&store_url, &access_token).await,
            OrderingProvider::Webhook { url, headers } => {
                self.webhook_list_orders(&url, &headers).await
            }
            OrderingProvider::None => Err(anyhow!("No ordering provider configured")),
        }
    }

    // ── Shopify implementation ───────────────────────────────────────────

    fn shopify_admin_url(store_url: &str) -> String {
        let base = store_url.trim_end_matches('/');
        // If the user already provided a full URL keep it, otherwise build one
        if base.starts_with("https://") || base.starts_with("http://") {
            format!("{}/admin/api/2024-01", base)
        } else {
            format!("https://{}.myshopify.com/admin/api/2024-01", base)
        }
    }

    async fn shopify_search_products(
        &self,
        store_url: &str,
        access_token: &str,
        query: &str,
    ) -> Result<serde_json::Value> {
        let base = Self::shopify_admin_url(store_url);
        let url = format!(
            "{}/products.json?title={}",
            base,
            urlencoding::encode(query)
        );

        let response = self
            .http
            .get(&url)
            .header("X-Shopify-Access-Token", access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Shopify product search failed: {}", error_text);
            return Err(anyhow!("Shopify product search failed: {}", error_text));
        }

        #[derive(Deserialize)]
        struct ProductsResponse {
            products: Vec<ShopifyProduct>,
        }

        #[derive(Deserialize)]
        struct ShopifyProduct {
            id: u64,
            title: String,
            body_html: Option<String>,
            #[serde(default)]
            variants: Vec<ShopifyVariant>,
            #[serde(default)]
            images: Vec<ShopifyImage>,
        }

        #[derive(Deserialize)]
        struct ShopifyVariant {
            price: Option<String>,
            #[serde(default)]
            available: Option<bool>,
        }

        #[derive(Deserialize)]
        struct ShopifyImage {
            src: String,
        }

        let result: ProductsResponse = response.json().await?;

        let products: Vec<serde_json::Value> = result
            .products
            .into_iter()
            .map(|p| {
                let first_variant = p.variants.first();
                let price = first_variant
                    .and_then(|v| v.price.as_deref())
                    .unwrap_or("0.00");
                let available = first_variant.and_then(|v| v.available).unwrap_or(false);
                let image_url = p.images.first().map(|i| i.src.as_str()).unwrap_or("");

                serde_json::json!({
                    "id": p.id,
                    "title": p.title,
                    "description": p.body_html.unwrap_or_default(),
                    "price": price,
                    "currency": "USD",
                    "image_url": image_url,
                    "available": available,
                })
            })
            .collect();

        Ok(serde_json::json!({ "products": products }))
    }

    async fn shopify_create_order(
        &self,
        store_url: &str,
        access_token: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let base = Self::shopify_admin_url(store_url);
        let url = format!("{}/orders.json", base);

        // Accept line_items directly or build from variant_id + quantity
        let line_items = if let Some(items) = params.get("line_items") {
            items.clone()
        } else {
            let variant_id = params
                .get("variant_id")
                .ok_or_else(|| anyhow!("Missing 'variant_id' or 'line_items' parameter"))?;
            let quantity = params.get("quantity").and_then(|v| v.as_u64()).unwrap_or(1);

            serde_json::json!([{
                "variant_id": variant_id,
                "quantity": quantity,
            }])
        };

        let body = serde_json::json!({
            "order": {
                "line_items": line_items,
            }
        });

        let response = self
            .http
            .post(&url)
            .header("X-Shopify-Access-Token", access_token)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Shopify create order failed: {}", error_text);
            return Err(anyhow!("Failed to create Shopify order: {}", error_text));
        }

        #[derive(Deserialize)]
        struct OrderResponse {
            order: ShopifyOrder,
        }

        #[derive(Deserialize)]
        struct ShopifyOrder {
            id: u64,
            #[serde(default)]
            financial_status: Option<String>,
            total_price: Option<String>,
        }

        let result: OrderResponse = response.json().await?;

        Ok(serde_json::json!({
            "order_id": result.order.id.to_string(),
            "status": result.order.financial_status.unwrap_or_else(|| "pending".to_string()),
            "total_price": result.order.total_price.unwrap_or_else(|| "0.00".to_string()),
        }))
    }

    async fn shopify_order_status(
        &self,
        store_url: &str,
        access_token: &str,
        order_id: &str,
    ) -> Result<serde_json::Value> {
        let base = Self::shopify_admin_url(store_url);
        let url = format!("{}/orders/{}.json", base, order_id);

        let response = self
            .http
            .get(&url)
            .header("X-Shopify-Access-Token", access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Shopify order status failed: {}", error_text);
            return Err(anyhow!("Failed to get order status: {}", error_text));
        }

        #[derive(Deserialize)]
        struct OrderResponse {
            order: ShopifyOrderDetail,
        }

        #[derive(Deserialize)]
        struct ShopifyOrderDetail {
            id: u64,
            #[serde(default)]
            financial_status: Option<String>,
            #[serde(default)]
            fulfillment_status: Option<String>,
            #[serde(default)]
            fulfillments: Vec<ShopifyFulfillment>,
        }

        #[derive(Deserialize)]
        struct ShopifyFulfillment {
            tracking_number: Option<String>,
            estimated_delivery_at: Option<String>,
        }

        let result: OrderResponse = response.json().await?;
        let order = result.order;

        let tracking_number = order
            .fulfillments
            .first()
            .and_then(|f| f.tracking_number.clone());
        let estimated_delivery = order
            .fulfillments
            .first()
            .and_then(|f| f.estimated_delivery_at.clone());

        // Combine financial + fulfillment status into a single human-readable status
        let status = match order.fulfillment_status.as_deref() {
            Some(fs) => format!(
                "{} / {}",
                order.financial_status.as_deref().unwrap_or("unknown"),
                fs
            ),
            None => order
                .financial_status
                .unwrap_or_else(|| "pending".to_string()),
        };

        Ok(serde_json::json!({
            "order_id": order.id.to_string(),
            "status": status,
            "tracking_number": tracking_number,
            "estimated_delivery": estimated_delivery,
        }))
    }

    async fn shopify_list_orders(
        &self,
        store_url: &str,
        access_token: &str,
    ) -> Result<serde_json::Value> {
        let base = Self::shopify_admin_url(store_url);
        let url = format!("{}/orders.json?status=any&limit=20", base);

        let response = self
            .http
            .get(&url)
            .header("X-Shopify-Access-Token", access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Shopify list orders failed: {}", error_text);
            return Err(anyhow!("Failed to list orders: {}", error_text));
        }

        #[derive(Deserialize)]
        struct OrdersResponse {
            orders: Vec<ShopifyOrderSummary>,
        }

        #[derive(Deserialize)]
        struct ShopifyOrderSummary {
            id: u64,
            #[serde(default)]
            financial_status: Option<String>,
            total_price: Option<String>,
            created_at: Option<String>,
        }

        let result: OrdersResponse = response.json().await?;

        let orders: Vec<serde_json::Value> = result
            .orders
            .into_iter()
            .map(|o| {
                serde_json::json!({
                    "order_id": o.id.to_string(),
                    "status": o.financial_status.unwrap_or_else(|| "unknown".to_string()),
                    "total_price": o.total_price.unwrap_or_else(|| "0.00".to_string()),
                    "created_at": o.created_at,
                })
            })
            .collect();

        Ok(serde_json::json!({ "orders": orders }))
    }

    // ── Webhook implementation ───────────────────────────────────────────

    /// Build a reqwest::RequestBuilder with the configured webhook headers
    fn webhook_request(
        &self,
        builder: reqwest::RequestBuilder,
        headers: &HashMap<String, String>,
    ) -> reqwest::RequestBuilder {
        let mut b = builder;
        for (key, value) in headers {
            b = b.header(key.as_str(), value.as_str());
        }
        b
    }

    async fn webhook_search_products(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        query: &str,
    ) -> Result<serde_json::Value> {
        let endpoint = format!("{}/search", url.trim_end_matches('/'));

        let body = serde_json::json!({ "query": query });

        let builder = self.http.post(&endpoint).json(&body);
        let response = self.webhook_request(builder, headers).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Webhook product search failed: {}", error_text);
            return Err(anyhow!("Webhook product search failed: {}", error_text));
        }

        // Expect the webhook to return {products: [...]} or a raw array
        let result: serde_json::Value = response.json().await?;

        // Normalise: if the response is already wrapped, return as-is; otherwise wrap it
        if result.get("products").is_some() {
            Ok(result)
        } else if result.is_array() {
            Ok(serde_json::json!({ "products": result }))
        } else {
            Ok(serde_json::json!({ "products": [result] }))
        }
    }

    async fn webhook_create_order(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let endpoint = format!("{}/order", url.trim_end_matches('/'));

        let builder = self.http.post(&endpoint).json(params);
        let response = self.webhook_request(builder, headers).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Webhook create order failed: {}", error_text);
            return Err(anyhow!("Webhook create order failed: {}", error_text));
        }

        let result: serde_json::Value = response.json().await?;

        // Ensure a consistent shape
        Ok(serde_json::json!({
            "order_id": result.get("order_id").or_else(|| result.get("id")).cloned().unwrap_or(serde_json::json!(null)),
            "status": result.get("status").cloned().unwrap_or(serde_json::json!("created")),
            "total_price": result.get("total_price").or_else(|| result.get("total")).cloned().unwrap_or(serde_json::json!(null)),
        }))
    }

    async fn webhook_order_status(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        order_id: &str,
    ) -> Result<serde_json::Value> {
        let endpoint = format!("{}/status/{}", url.trim_end_matches('/'), order_id);

        let builder = self.http.get(&endpoint);
        let response = self.webhook_request(builder, headers).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Webhook order status failed: {}", error_text);
            return Err(anyhow!("Webhook order status failed: {}", error_text));
        }

        let result: serde_json::Value = response.json().await?;

        Ok(serde_json::json!({
            "order_id": result.get("order_id").or_else(|| result.get("id")).cloned().unwrap_or(serde_json::json!(order_id)),
            "status": result.get("status").cloned().unwrap_or(serde_json::json!("unknown")),
            "tracking_number": result.get("tracking_number").cloned().unwrap_or(serde_json::json!(null)),
            "estimated_delivery": result.get("estimated_delivery").cloned().unwrap_or(serde_json::json!(null)),
        }))
    }

    async fn webhook_list_orders(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
    ) -> Result<serde_json::Value> {
        let endpoint = format!("{}/orders", url.trim_end_matches('/'));

        let builder = self.http.get(&endpoint);
        let response = self.webhook_request(builder, headers).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Webhook list orders failed: {}", error_text);
            return Err(anyhow!("Webhook list orders failed: {}", error_text));
        }

        let result: serde_json::Value = response.json().await?;

        if result.get("orders").is_some() {
            Ok(result)
        } else if result.is_array() {
            Ok(serde_json::json!({ "orders": result }))
        } else {
            Ok(serde_json::json!({ "orders": [result] }))
        }
    }
}

#[async_trait]
impl Integration for OrderingConnector {
    fn id(&self) -> &str {
        "ordering"
    }

    fn name(&self) -> &str {
        "Ordering & Purchasing"
    }

    fn description(&self) -> &str {
        "Search products, place orders, and track deliveries via Shopify or custom webhook"
    }

    fn icon(&self) -> &str {
        "🛒"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        match Self::load_provider(&self.config_dir) {
            OrderingProvider::None => IntegrationStatus::NotConfigured,
            OrderingProvider::Shopify { .. } => IntegrationStatus::Connected,
            OrderingProvider::Webhook { .. } => IntegrationStatus::Connected,
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "search_products" => self.search_products(params).await,
            "create_order" => self.create_order(params).await,
            "order_status" => self.order_status(params).await,
            "list_orders" => self.list_orders(params).await,
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for OrderingConnector {
    fn default() -> Self {
        Self::new()
    }
}
