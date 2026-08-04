//! The wasm32 (wasi:tls) implementation of the `reqwest` subset. On native
//! targets `lib.rs` re-exports the real `reqwest` instead of this module.

use std::fmt;
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use serde::Serialize;
use url::Url;

pub use http::header;
pub use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};

// ---- Error ----

#[derive(Debug)]
pub struct Error {
    msg: String,
}

impl Error {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Error { msg: msg.into() }
    }
    /// The wasi:tls path does not classify transport errors, so these conservatively
    /// return false (an SDK using them for retry decisions simply won't retry on
    /// connect/timeout — safe, if slightly less resilient than native reqwest).
    pub fn is_connect(&self) -> bool {
        false
    }
    pub fn is_timeout(&self) -> bool {
        false
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

#[derive(Clone, Debug, Default)]
pub struct Client {
    user_agent: Option<String>,
    /// Headers merged into every request (real reqwest's `default_headers`).
    default_headers: Vec<(String, String)>,
}

impl Client {
    pub fn new() -> Self {
        Client::default()
    }
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    pub fn request(&self, method: Method, url: Url) -> RequestBuilder {
        let mut headers = Vec::new();
        if let Some(ua) = &self.user_agent {
            headers.push(("User-Agent".to_string(), ua.clone()));
        }
        headers.extend(self.default_headers.iter().cloned());
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

    pub fn post(&self, url: Url) -> RequestBuilder {
        self.request(Method::POST, url)
    }

    /// Execute a pre-built `Request` (reqwest's `Client::execute`). The request already
    /// carries the client's user-agent + default headers (baked in at build time via
    /// `request()`), so this just performs the transport.
    pub async fn execute(&self, req: Request) -> Result<Response, Error> {
        do_request(req.method.as_str(), &req.url, &req.headers, req.body.as_deref()).await
    }
}

#[derive(Default)]
pub struct ClientBuilder {
    user_agent: Option<String>,
    default_headers: Vec<(String, String)>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        ClientBuilder::default()
    }
    /// Set headers merged into every request (reqwest's `ClientBuilder::default_headers`).
    pub fn default_headers(mut self, headers: HeaderMap) -> Self {
        for (name, value) in headers.iter() {
            if let Ok(v) = value.to_str() {
                self.default_headers.push((name.as_str().to_string(), v.to_string()));
            }
        }
        self
    }
    pub fn tls_built_in_root_certs(self, _enabled: bool) -> Self {
        self
    }
    pub fn danger_accept_invalid_certs(self, _enabled: bool) -> Self {
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
            default_headers: self.default_headers,
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
    /// reqwest-compatible header signature: accepts `&str`/`String` AND typed
    /// `HeaderName`/`HeaderValue` (the SDK passes the latter). Stored as ascii strings
    /// for the http1 transport (all our headers are ascii; opaque-byte values are rejected).
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        match (HeaderName::try_from(key), HeaderValue::try_from(value)) {
            (Ok(name), Ok(val)) => match val.to_str() {
                Ok(s) => self.headers.push((name.as_str().to_string(), s.to_string())),
                Err(_) => self.err = Some(Error::new("header value not valid ascii")),
            },
            _ => self.err = Some(Error::new("invalid header name/value")),
        }
        self
    }

    /// Append query parameters (reqwest's `RequestBuilder::query`), serialized exactly
    /// like real reqwest (serde_urlencoded into the URL's query pairs — merges with any
    /// existing query).
    pub fn query<T: Serialize + ?Sized>(mut self, query: &T) -> Self {
        {
            let mut pairs = self.url.query_pairs_mut();
            let serializer = serde_urlencoded::Serializer::new(&mut pairs);
            if let Err(err) = query.serialize(serializer) {
                self.err = Some(Error::new(format!("query serialize: {err}")));
            }
        }
        if let Some("") = self.url.query() {
            self.url.set_query(None);
        }
        self
    }

    /// Build the `Request` without sending (reqwest's `RequestBuilder::build`); the
    /// retry path uses `build()` + `Client::execute()` so it can `try_clone()` a request.
    pub fn build(self) -> Result<Request, Error> {
        if let Some(e) = self.err {
            return Err(e);
        }
        Ok(Request {
            method: self.method,
            url: self.url,
            headers: self.headers,
            body: self.body,
        })
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
        do_request(self.method.as_str(), &self.url, &self.headers, self.body.as_deref()).await
    }
}

/// The single transport call shared by `RequestBuilder::send` and `Client::execute`.
async fn do_request(
    method: &str,
    url: &Url,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<Response, Error> {
    let raw = crate::http1::request(method, url, headers, body)
        .await
        .map_err(Error::new)?;
    Ok(Response {
        status: StatusCode::from_u16(raw.status).map_err(|_| Error::new("invalid status code"))?,
        headers: raw.headers,
        body: raw.body,
    })
}

// ---- Request (built, ready to execute) ----

/// A built request (reqwest's `Request`). Carries the fully-resolved method/url/headers/
/// body so the retry loop can `try_clone()` and re-`execute()` it.
#[derive(Clone)]
pub struct Request {
    method: Method,
    url: Url,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
}

impl Request {
    pub fn method(&self) -> &Method {
        &self.method
    }
    pub fn url(&self) -> &Url {
        &self.url
    }
    pub fn url_mut(&mut self) -> &mut Url {
        &mut self.url
    }
    /// The body is buffered (Vec), so a request is always cloneable.
    pub fn try_clone(&self) -> Option<Request> {
        Some(self.clone())
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
