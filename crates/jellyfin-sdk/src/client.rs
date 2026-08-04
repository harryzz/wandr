use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use reqwest::{
    Client, Method, Response,
    header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue},
};
use url::Url;

use crate::{Error, Result};

#[derive(Debug)]
struct Inner {
    http: Client,
    base_url: Url,
    auth: AuthContext,
    retry: RetryConfig,
}

#[derive(Debug)]
struct AuthContext {
    client_name: String,
    device_name: String,
    device_id: String,
    client_version: String,
    token: RwLock<Option<String>>,
}

/// A clonable async Jellyfin client.
#[derive(Clone, Debug)]
pub struct JellyfinClient {
    inner: Arc<Inner>,
}

/// Retry configuration for requests sent by [`JellyfinClient`].
#[derive(Clone, Debug)]
pub struct RetryConfig {
    /// How many retries to attempt after the initial request.
    pub max_retries: u32,
    /// Initial backoff delay when retrying.
    pub initial_backoff: Duration,
    /// Maximum backoff delay when retrying.
    pub max_backoff: Duration,
    /// Whether to allow retries for non-idempotent methods (e.g. POST).
    pub retry_non_idempotent: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            initial_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(5),
            retry_non_idempotent: false,
        }
    }
}

/// Builds a [`JellyfinClient`].
#[derive(Debug)]
pub struct JellyfinClientBuilder {
    base_url: Url,
    client_name: String,
    device_name: String,
    device_id: String,
    client_version: String,
    token: Option<String>,
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
    retry: RetryConfig,
}

impl JellyfinClientBuilder {
    /// Creates a new builder with a base server URL.
    ///
    /// `base_url` may include a reverse-proxy path (e.g. `https://host/jellyfin/`).
    pub fn new(base_url: impl AsRef<str>) -> Result<Self> {
        let base_url = normalize_base_url(Url::parse(base_url.as_ref())?)?;
        Ok(Self {
            base_url,
            client_name: env!("CARGO_PKG_NAME").to_owned(),
            device_name: std::env::consts::OS.to_owned(),
            device_id: uuid::Uuid::new_v4().to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
            token: None,
            timeout: Some(Duration::from_secs(30)),
            connect_timeout: Some(Duration::from_secs(10)),
            retry: RetryConfig::default(),
        })
    }

    /// Sets the Jellyfin access token (API key).
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Sets the global request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the connect timeout.
    pub fn connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = Some(connect_timeout);
        self
    }

    /// Sets retry configuration.
    pub fn retry_config(mut self, retry: RetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// Sets the client name used in the `Authorization: MediaBrowser ...` header.
    pub fn client_name(mut self, client_name: impl Into<String>) -> Self {
        self.client_name = client_name.into();
        self
    }

    /// Sets the device name used in the `Authorization: MediaBrowser ...` header.
    pub fn device_name(mut self, device_name: impl Into<String>) -> Self {
        self.device_name = device_name.into();
        self
    }

    /// Sets the device id used in the `Authorization: MediaBrowser ...` header.
    pub fn device_id(mut self, device_id: impl Into<String>) -> Self {
        self.device_id = device_id.into();
        self
    }

    /// Sets the client version used in the `Authorization: MediaBrowser ...` header.
    pub fn client_version(mut self, client_version: impl Into<String>) -> Self {
        self.client_version = client_version.into();
        self
    }

    /// Builds the client.
    pub fn build(self) -> Result<JellyfinClient> {
        if self.client_name.trim().is_empty() {
            return Err(Error::InvalidConfig("client_name must not be empty"));
        }
        if self.device_name.trim().is_empty() {
            return Err(Error::InvalidConfig("device_name must not be empty"));
        }
        if self.device_id.trim().is_empty() {
            return Err(Error::InvalidConfig("device_id must not be empty"));
        }
        if self.client_version.trim().is_empty() {
            return Err(Error::InvalidConfig("client_version must not be empty"));
        }

        let user_agent = format!("{}/{}", self.client_name, self.client_version);
        let mut http_builder = Client::builder()
            .default_headers(HeaderMap::new())
            .user_agent(user_agent);

        if let Some(timeout) = self.timeout {
            http_builder = http_builder.timeout(timeout);
        }
        if let Some(connect_timeout) = self.connect_timeout {
            http_builder = http_builder.connect_timeout(connect_timeout);
        }

        let http = http_builder.build()?;

        Ok(JellyfinClient {
            inner: Arc::new(Inner {
                http,
                base_url: self.base_url,
                auth: AuthContext {
                    client_name: self.client_name,
                    device_name: self.device_name,
                    device_id: self.device_id,
                    client_version: self.client_version,
                    token: RwLock::new(self.token),
                },
                retry: self.retry,
            }),
        })
    }
}

impl JellyfinClient {
    /// Creates a client with default configuration.
    pub fn new(base_url: impl AsRef<str>) -> Result<Self> {
        Self::builder(base_url)?.build()
    }

    /// Creates a builder for a Jellyfin client.
    pub fn builder(base_url: impl AsRef<str>) -> Result<JellyfinClientBuilder> {
        JellyfinClientBuilder::new(base_url)
    }

    /// Returns the configured base URL (always ends with `/`).
    pub fn base_url(&self) -> &Url {
        &self.inner.base_url
    }

    /// Returns the configured device id used in Jellyfin auth headers.
    pub fn device_id(&self) -> &str {
        &self.inner.auth.device_id
    }

    /// Sets the access token used for authenticated requests.
    ///
    /// This updates the shared auth context, so cloned clients will see the change.
    pub fn set_token(&self, token: impl Into<String>) {
        *self.inner.auth.token.write().expect("poisoned token lock") = Some(token.into());
    }

    /// Clears the access token.
    pub fn clear_token(&self) {
        *self.inner.auth.token.write().expect("poisoned token lock") = None;
    }

    /// Provides access to System API endpoints.
    pub fn system(&self) -> crate::api::SystemApi {
        crate::api::SystemApi::new(self.clone())
    }

    /// Provides access to User API endpoints.
    pub fn user(&self) -> crate::api::UserApi {
        crate::api::UserApi::new(self.clone())
    }

    /// Provides access to Items API endpoints.
    pub fn items(&self) -> crate::api::ItemsApi {
        crate::api::ItemsApi::new(self.clone())
    }

    /// Provides access to Search API endpoints.
    pub fn search(&self) -> crate::api::SearchApi {
        crate::api::SearchApi::new(self.clone())
    }

    /// Provides access to Filters API endpoints.
    pub fn filters(&self) -> crate::api::FiltersApi {
        crate::api::FiltersApi::new(self.clone())
    }

    /// Provides access to Movies API endpoints.
    pub fn movies(&self) -> crate::api::MoviesApi {
        crate::api::MoviesApi::new(self.clone())
    }

    /// Provides access to Videos API endpoints.
    pub fn videos(&self) -> crate::api::VideosApi {
        crate::api::VideosApi::new(self.clone())
    }

    /// Provides access to LibraryStructure API endpoints.
    pub fn library_structure(&self) -> crate::api::LibraryStructureApi {
        crate::api::LibraryStructureApi::new(self.clone())
    }

    /// Provides access to Configuration API endpoints.
    pub fn configuration(&self) -> crate::api::ConfigurationApi {
        crate::api::ConfigurationApi::new(self.clone())
    }

    /// Provides access to API key endpoints.
    pub fn api_key(&self) -> crate::api::ApiKeyApi {
        crate::api::ApiKeyApi::new(self.clone())
    }

    /// Provides access to Plugin API endpoints.
    pub fn plugins(&self) -> crate::api::PluginsApi {
        crate::api::PluginsApi::new(self.clone())
    }

    /// Provides access to Dynamic HLS endpoints.
    pub fn dynamic_hls(&self) -> crate::api::DynamicHlsApi {
        crate::api::DynamicHlsApi::new(self.clone())
    }

    /// Provides access to ScheduledTasks API endpoints.
    pub fn scheduled_tasks(&self) -> crate::api::ScheduledTasksApi {
        crate::api::ScheduledTasksApi::new(self.clone())
    }

    /// Provides access to Playstate API endpoints.
    pub fn playstate(&self) -> crate::api::PlaystateApi {
        crate::api::PlaystateApi::new(self.clone())
    }

    /// Provides access to Playlist endpoints.
    pub fn playlists(&self) -> crate::api::PlaylistsApi {
        crate::api::PlaylistsApi::new(self.clone())
    }

    /// Provides access to Image endpoints (user profile image, etc).
    pub fn images(&self) -> crate::api::ImagesApi {
        crate::api::ImagesApi::new(self.clone())
    }

    /// Provides access to Subtitle API endpoints.
    pub fn subtitles(&self) -> crate::api::SubtitlesApi {
        crate::api::SubtitlesApi::new(self.clone())
    }

    /// Provides access to Suggestions API endpoints.
    pub fn suggestions(&self) -> crate::api::SuggestionsApi {
        crate::api::SuggestionsApi::new(self.clone())
    }

    /// Provides access to Genres API endpoints.
    pub fn genres(&self) -> crate::api::GenresApi {
        crate::api::GenresApi::new(self.clone())
    }

    /// Provides access to Sessions API endpoints.
    pub fn sessions(&self) -> crate::api::SessionsApi {
        crate::api::SessionsApi::new(self.clone())
    }

    /// Provides access to UserViews API endpoints.
    pub fn user_views(&self) -> crate::api::UserViewsApi {
        crate::api::UserViewsApi::new(self.clone())
    }

    /// Provides access to UserLibrary API endpoints.
    pub fn user_library(&self) -> crate::api::UserLibraryApi {
        crate::api::UserLibraryApi::new(self.clone())
    }

    /// Starts building a request against this server.
    ///
    /// `path` is joined onto `base_url`, so it can be either absolute-like (`/System/Ping`)
    /// or relative-like (`System/Ping`).
    pub fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let url = self.endpoint_url(path)?;
        let auth = self.inner.auth.authorization_header_value()?;

        let mut builder = self
            .inner
            .http
            .request(method, url)
            .header(AUTHORIZATION, auth);

        if let Some(token) = self.inner.auth.token() {
            builder = builder.header(
                HeaderName::from_static("x-emby-token"),
                HeaderValue::from_str(&token)
                    .map_err(|_| Error::InvalidConfig("invalid x-emby-token header"))?,
            );
        }

        Ok(builder)
    }

    /// Executes a request and parses a JSON response, applying the client's retry policy.
    pub async fn send_json<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T> {
        let request = request.build()?;
        let response = self.execute_with_retry(request).await?;
        parse_json_response(response).await
    }

    /// Executes a request and returns the raw response, applying the client's retry policy.
    ///
    /// This is useful for streaming downloads via `Response::bytes_stream()`.
    pub async fn execute(&self, request: reqwest::RequestBuilder) -> Result<Response> {
        let request = request.build()?;
        self.execute_with_retry(request).await
    }

    /// Executes a request and discards the response body.
    ///
    /// This is useful for endpoints that return `204 No Content` or where the body is irrelevant.
    pub async fn send_unit(&self, request: reqwest::RequestBuilder) -> Result<()> {
        let response = self.execute(request).await?;
        let _ = response.bytes().await?;
        Ok(())
    }

    /// Executes a request and reads the response body as UTF-8 text.
    pub async fn send_text(&self, request: reqwest::RequestBuilder) -> Result<String> {
        let response = self.execute(request).await?;
        Ok(response.text().await?)
    }

    /// Convenience helper for downloading a path as bytes.
    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let response = self.execute(self.request(Method::GET, path)?).await?;
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// Downloads a request and streams the response body into a file.
    pub async fn download(
        &self,
        request: reqwest::RequestBuilder,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        // SPIKE: buffer the body then write it (wasi:tls has no bytes_stream; the guest
        // streams media via byte-range, not download-to-file). Sync std::fs is fine on
        // the single-threaded wasip2 guest.
        let response = self.execute(request).await?;
        let bytes = response.bytes().await?;
        std::fs::write(file_path, &bytes)?;
        Ok(())
    }

    /// Downloads a path and streams the response body into a file.
    pub async fn download_to_file(
        &self,
        path: &str,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        self.download(self.request(Method::GET, path)?, file_path)
            .await
    }

    fn endpoint_url(&self, path: &str) -> Result<Url> {
        let path = path.trim_start_matches('/');
        Ok(self.inner.base_url.join(path)?)
    }

    async fn execute_with_retry(&self, request: reqwest::Request) -> Result<Response> {
        let method = request.method().clone();
        let max_attempts = self.inner.retry.max_retries.saturating_add(1);

        let mut attempt = 1u32;
        let mut request = request;

        loop {
            let next_request = request.try_clone();

            let result = self.inner.http.execute(request).await;
            match result {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
                    }

                    let retry_after_seconds = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok());

                    let body = response
                        .text()
                        .await
                        .unwrap_or_else(|_| "<body unavailable>".to_owned());

                    let error = Error::Api {
                        status,
                        body,
                        retry_after_seconds,
                    };

                    if attempt < max_attempts
                        && should_retry_status(&self.inner.retry, &method, status)
                    {
                        let Some(next_request) = next_request else {
                            return Err(error);
                        };

                        let delay = retry_delay(&self.inner.retry, attempt, retry_after_seconds);
                        tracing::warn!(
                            attempt,
                            max_attempts,
                            %status,
                            delay_ms = delay.as_millis(),
                            "request failed; retrying"
                        );
                        reqwest::task::sleep(delay).await;
                        attempt += 1;
                        request = next_request;
                        continue;
                    }

                    return Err(error);
                }
                Err(err) => {
                    if attempt < max_attempts
                        && should_retry_error(&self.inner.retry, &method, &err)
                    {
                        let Some(next_request) = next_request else {
                            return Err(Error::Http(err));
                        };

                        let delay = retry_delay(&self.inner.retry, attempt, None);
                        tracing::warn!(
                            attempt,
                            max_attempts,
                            delay_ms = delay.as_millis(),
                            "http error; retrying: {err}"
                        );
                        reqwest::task::sleep(delay).await;
                        attempt += 1;
                        request = next_request;
                        continue;
                    }

                    return Err(Error::Http(err));
                }
            }
        }
    }
}

fn normalize_base_url(mut url: Url) -> Result<Url> {
    if url.cannot_be_a_base() {
        return Err(Error::InvalidConfig("base_url cannot be a base URL"));
    }
    if url.path().is_empty() {
        url.set_path("/");
    }
    if !url.path().ends_with('/') {
        let new_path = format!("{}/", url.path());
        url.set_path(&new_path);
    }
    Ok(url)
}

fn build_mediabrowser_auth_header(
    client_name: &str,
    device_name: &str,
    device_id: &str,
    client_version: &str,
    token: Option<&str>,
) -> Result<HeaderValue> {
    fn enc(input: &str) -> String {
        url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
    }

    let mut parts = vec![
        format!("Client=\"{}\"", enc(client_name)),
        format!("Device=\"{}\"", enc(device_name)),
        format!("DeviceId=\"{}\"", enc(device_id)),
        format!("Version=\"{}\"", enc(client_version)),
    ];

    if let Some(token) = token {
        parts.push(format!("Token=\"{}\"", enc(token)));
    }

    let value = format!("MediaBrowser {}", parts.join(", "));
    HeaderValue::from_str(&value).map_err(|_| Error::InvalidConfig("invalid auth header"))
}

async fn parse_json_response<T: serde::de::DeserializeOwned>(response: Response) -> Result<T> {
    let bytes = response.bytes().await?;
    Ok(serde_json::from_slice(&bytes)?)
}

impl AuthContext {
    fn token(&self) -> Option<String> {
        self.token.read().expect("poisoned token lock").clone()
    }

    fn authorization_header_value(&self) -> Result<HeaderValue> {
        build_mediabrowser_auth_header(
            &self.client_name,
            &self.device_name,
            &self.device_id,
            &self.client_version,
            self.token().as_deref(),
        )
    }
}

fn should_retry_status(config: &RetryConfig, method: &Method, status: reqwest::StatusCode) -> bool {
    if !config.retry_non_idempotent && !is_idempotent(method) {
        return false;
    }

    matches!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS
            | reqwest::StatusCode::INTERNAL_SERVER_ERROR
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    )
}

fn should_retry_error(config: &RetryConfig, method: &Method, err: &reqwest::Error) -> bool {
    if !config.retry_non_idempotent && !is_idempotent(method) {
        return false;
    }

    err.is_timeout() || err.is_connect()
}

fn is_idempotent(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn retry_delay(config: &RetryConfig, attempt: u32, retry_after_seconds: Option<u64>) -> Duration {
    if let Some(secs) = retry_after_seconds {
        return Duration::from_secs(secs).min(config.max_backoff);
    }

    let exponent = attempt.saturating_sub(1).min(30);
    let mut delay = config
        .initial_backoff
        .saturating_mul(2u32.saturating_pow(exponent));
    if delay > config.max_backoff {
        delay = config.max_backoff;
    }
    delay
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_keeps_reverse_proxy_path() {
        let client = JellyfinClient::builder("https://example.com/jellyfin")
            .unwrap()
            .build()
            .unwrap();
        let url = client.endpoint_url("/System/Info/Public").unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.com/jellyfin/System/Info/Public"
        );
    }

    #[test]
    fn auth_header_is_mediabrowser_scheme() {
        let header = build_mediabrowser_auth_header("a", "b", "c", "d", Some("t")).unwrap();
        let as_str = header.to_str().unwrap();
        assert!(as_str.starts_with("MediaBrowser "));
        assert!(as_str.contains("Token=\"t\""));
    }
}
