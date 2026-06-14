//! The wasm32 (wasi:tls) implementation of the `reqwest` subset. On native
//! targets `lib.rs` re-exports the real `reqwest` instead of this module.

use std::fmt;
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use serde::Serialize;
use url::Url;

pub use http::header;
pub use http::{Method, StatusCode};

// ---- Error ----

#[derive(Debug)]
pub struct Error {
    msg: String,
}

impl Error {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Error { msg: msg.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::new(format!("json: {e}"))
    }
}

// ---- Certificate (host-delegated trust; bytes ignored) ----

pub struct Certificate;

impl Certificate {
    pub fn from_pem(_pem: &[u8]) -> Result<Certificate, Error> {
        Ok(Certificate)
    }
}

// ---- Client / ClientBuilder ----

#[derive(Clone)]
pub struct Client {
    user_agent: Option<String>,
}

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    pub fn request(&self, method: Method, url: Url) -> RequestBuilder {
        let mut headers = Vec::new();
        if let Some(ua) = &self.user_agent {
            headers.push(("User-Agent".to_string(), ua.clone()));
        }
        RequestBuilder {
            method,
            url,
            headers,
            body: None,
            err: None,
        }
    }

    pub fn get(&self, url: Url) -> RequestBuilder {
        self.request(Method::GET, url)
    }
}

#[derive(Default)]
pub struct ClientBuilder {
    user_agent: Option<String>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        ClientBuilder::default()
    }
    pub fn tls_built_in_root_certs(self, _enabled: bool) -> Self {
        self
    }
    pub fn add_root_certificate(self, _cert: Certificate) -> Self {
        self
    }
    pub fn connect_timeout(self, _t: Duration) -> Self {
        self
    }
    pub fn timeout(self, _t: Duration) -> Self {
        self
    }
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }
    pub fn http1_only(self) -> Self {
        self
    }
    pub fn build(self) -> Result<Client, Error> {
        Ok(Client {
            user_agent: self.user_agent,
        })
    }
}

// ---- RequestBuilder ----

pub struct RequestBuilder {
    method: Method,
    url: Url,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    err: Option<Error>,
}

impl RequestBuilder {
    pub fn header<K: fmt::Display, V: fmt::Display>(
        mut self,
        key: K,
        value: V,
    ) -> Self {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }

    pub fn basic_auth<U: fmt::Display, P: fmt::Display>(
        mut self,
        username: U,
        password: Option<P>,
    ) -> Self {
        let raw = match password {
            Some(p) => format!("{username}:{p}"),
            None => format!("{username}:"),
        };
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        self.headers
            .push(("Authorization".to_string(), format!("Basic {encoded}")));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(v) => {
                self.headers.push((
                    "Content-Type".to_string(),
                    "application/json".to_string(),
                ));
                self.body = Some(v);
            },
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    pub fn multipart(mut self, form: crate::multipart::Form) -> Self {
        let (content_type, body) = form.encode();
        self.headers.push(("Content-Type".to_string(), content_type));
        self.body = Some(body);
        self
    }

    /// Consume into the URL + headers, for the websocket-shim upgrade path.
    pub fn into_ws_parts(self) -> (Url, Vec<(String, String)>) {
        (self.url, self.headers)
    }

    pub async fn send(self) -> Result<Response, Error> {
        if let Some(e) = self.err {
            return Err(e);
        }
        let raw = crate::http1::request(
            self.method.as_str(),
            &self.url,
            &self.headers,
            self.body.as_deref(),
        )
        .await
        .map_err(Error::new)?;
        Ok(Response {
            status: StatusCode::from_u16(raw.status)
                .map_err(|_| Error::new("invalid status code"))?,
            headers: raw.headers,
            body: raw.body,
        })
    }
}

// ---- Response ----

#[derive(Debug)]
pub struct Response {
    status: StatusCode,
    headers: http::HeaderMap,
    body: Vec<u8>,
}

impl Response {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &http::HeaderMap {
        &self.headers
    }

    pub async fn json<U: serde::de::DeserializeOwned>(
        self,
    ) -> Result<U, Error> {
        serde_json::from_slice(&self.body).map_err(Into::into)
    }

    pub async fn text(self) -> Result<String, Error> {
        Ok(String::from_utf8_lossy(&self.body).into_owned())
    }

    pub async fn bytes(self) -> Result<Bytes, Error> {
        Ok(Bytes::from(self.body))
    }

    pub fn error_for_status(self) -> Result<Response, Error> {
        if self.status.is_client_error() || self.status.is_server_error() {
            Err(Error::new(format!("HTTP status {}", self.status.as_u16())))
        } else {
            Ok(self)
        }
    }

    pub fn bytes_stream(
        self,
    ) -> impl futures::Stream<Item = Result<Bytes, Error>> + Unpin {
        // Body is already buffered; yield it once. `iter` is Unpin (required by
        // the fork's `.bytes_stream().map_err(..).into_async_read()`).
        futures::stream::iter(std::iter::once(Ok(Bytes::from(self.body))))
    }
}
