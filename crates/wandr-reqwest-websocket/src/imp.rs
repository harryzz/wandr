//! A drop-in subset of `reqwest-websocket` (RFC6455 client) over the
//! wandr-reqwest `TlsStream`. Provides exactly what our libsignal-service-rs
//! fork's `SignalWebSocketProcess` uses: `RequestBuilderExt::upgrade`, an
//! `UpgradedRequestBuilder::send` → `UpgradeResponse::into_websocket`, and a
//! `WebSocket` with inherent async `send`/`next`/`close` + `Message`/`CloseCode`.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use base64::Engine;
use url::Url;

use wandr_reqwest::tls::TlsStream;
use wandr_reqwest::{random_bytes, RequestBuilder};

// ---- Error ----

#[derive(Debug)]
pub struct Error {
    msg: String,
}

impl Error {
    fn new(msg: impl Into<String>) -> Self {
        Error { msg: msg.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for Error {}

// ---- CloseCode / Message ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseCode {
    Normal,
    Away,
    Protocol,
    Abnormal,
    Other(u16),
}

impl CloseCode {
    fn to_u16(self) -> u16 {
        match self {
            CloseCode::Normal => 1000,
            CloseCode::Away => 1001,
            CloseCode::Protocol => 1002,
            CloseCode::Abnormal => 1006,
            CloseCode::Other(c) => c,
        }
    }
    fn from_u16(c: u16) -> CloseCode {
        match c {
            1000 => CloseCode::Normal,
            1001 => CloseCode::Away,
            1002 => CloseCode::Protocol,
            1006 => CloseCode::Abnormal,
            other => CloseCode::Other(other),
        }
    }
}

impl fmt::Display for CloseCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_u16())
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close { code: CloseCode, reason: String },
}

impl From<Vec<u8>> for Message {
    fn from(v: Vec<u8>) -> Self {
        Message::Binary(v)
    }
}

// ---- WebSocket ----

struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

pub struct WebSocket {
    stream: TlsStream,
    closed: bool,
}

impl WebSocket {
    fn new(stream: TlsStream) -> Self {
        WebSocket {
            stream,
            closed: false,
        }
    }

    pub async fn send(&mut self, msg: Message) -> Result<(), Error> {
        let (opcode, payload) = match msg {
            Message::Text(s) => (0x1u8, s.into_bytes()),
            Message::Binary(b) => (0x2, b),
            Message::Ping(b) => (0x9, b),
            Message::Pong(b) => (0xA, b),
            Message::Close { code, reason } => {
                let mut p = code.to_u16().to_be_bytes().to_vec();
                p.extend_from_slice(reason.as_bytes());
                (0x8, p)
            },
        };
        let frame = encode_client_frame(opcode, &payload);
        self.stream.write_all(&frame).await.map_err(Error::new)
    }

    pub async fn close(
        &mut self,
        code: CloseCode,
        reason: Option<&str>,
    ) -> Result<(), Error> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.send(Message::Close {
            code,
            reason: reason.unwrap_or("").to_string(),
        })
        .await
    }

    /// Boxed so the returned future is `Unpin` (the fork drives this inside
    /// `futures::select!`, which requires `Unpin`).
    pub fn next(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Option<Result<Message, Error>>> + '_>>
    {
        Box::pin(self.read_message())
    }

    async fn read_message(&mut self) -> Option<Result<Message, Error>> {
        let mut frag_opcode: Option<u8> = None;
        let mut frag_payload: Vec<u8> = Vec::new();
        loop {
            let frame = match self.read_frame().await {
                Ok(Some(f)) => f,
                Ok(None) => return None,
                Err(e) => return Some(Err(e)),
            };
            match frame.opcode {
                0x0 => {
                    if frag_opcode.is_none() {
                        return Some(Err(Error::new(
                            "unexpected continuation frame",
                        )));
                    }
                    frag_payload.extend_from_slice(&frame.payload);
                    if frame.fin {
                        let op = frag_opcode.take().unwrap();
                        let data = std::mem::take(&mut frag_payload);
                        return Some(Ok(data_message(op, data)));
                    }
                },
                0x1 | 0x2 => {
                    if frame.fin {
                        return Some(Ok(data_message(
                            frame.opcode,
                            frame.payload,
                        )));
                    }
                    frag_opcode = Some(frame.opcode);
                    frag_payload = frame.payload;
                },
                0x8 => {
                    let (code, reason) = parse_close(&frame.payload);
                    return Some(Ok(Message::Close { code, reason }));
                },
                0x9 => return Some(Ok(Message::Ping(frame.payload))),
                0xA => return Some(Ok(Message::Pong(frame.payload))),
                _ => {
                    return Some(Err(Error::new("unknown websocket opcode")))
                },
            }
        }
    }

    async fn read_frame(&mut self) -> Result<Option<Frame>, Error> {
        // First 2 header bytes; a clean EOF here means the socket closed.
        let hdr = match self.stream.read_exact(2).await {
            Ok(h) => h,
            Err(_) => return Ok(None),
        };
        let fin = hdr[0] & 0x80 != 0;
        let opcode = hdr[0] & 0x0F;
        let masked = hdr[1] & 0x80 != 0;
        let len = match hdr[1] & 0x7F {
            126 => {
                let e = self.stream.read_exact(2).await.map_err(Error::new)?;
                u16::from_be_bytes([e[0], e[1]]) as usize
            },
            127 => {
                let e = self.stream.read_exact(8).await.map_err(Error::new)?;
                u64::from_be_bytes(e[..8].try_into().unwrap()) as usize
            },
            n => n as usize,
        };
        let mask_key = if masked {
            Some(self.stream.read_exact(4).await.map_err(Error::new)?)
        } else {
            None
        };
        let mut payload = if len > 0 {
            self.stream.read_exact(len).await.map_err(Error::new)?
        } else {
            Vec::new()
        };
        if let Some(k) = mask_key {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= k[i % 4];
            }
        }
        Ok(Some(Frame {
            fin,
            opcode,
            payload,
        }))
    }
}

fn data_message(opcode: u8, data: Vec<u8>) -> Message {
    if opcode == 0x1 {
        Message::Text(String::from_utf8_lossy(&data).into_owned())
    } else {
        Message::Binary(data)
    }
}

fn parse_close(payload: &[u8]) -> (CloseCode, String) {
    if payload.len() >= 2 {
        let code = CloseCode::from_u16(u16::from_be_bytes([
            payload[0], payload[1],
        ]));
        let reason = String::from_utf8_lossy(&payload[2..]).into_owned();
        (code, reason)
    } else {
        (CloseCode::Normal, String::new())
    }
}

fn encode_client_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode); // FIN + opcode
    let len = payload.len();
    if len < 126 {
        frame.push(0x80 | len as u8);
    } else if len <= 0xFFFF {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    let key = random_bytes(4);
    frame.extend_from_slice(&key);
    for (i, b) in payload.iter().enumerate() {
        frame.push(b ^ key[i % 4]);
    }
    frame
}

// ---- Upgrade path ----

pub trait RequestBuilderExt {
    fn upgrade(self) -> UpgradedRequestBuilder;
}

impl RequestBuilderExt for RequestBuilder {
    fn upgrade(self) -> UpgradedRequestBuilder {
        let (url, headers) = self.into_ws_parts();
        UpgradedRequestBuilder { url, headers }
    }
}

pub struct UpgradedRequestBuilder {
    url: Url,
    headers: Vec<(String, String)>,
}

impl UpgradedRequestBuilder {
    pub async fn send(self) -> Result<UpgradeResponse, Error> {
        let host = self
            .url
            .host_str()
            .ok_or_else(|| Error::new("websocket url has no host"))?
            .to_string();
        let port = self.url.port_or_known_default().unwrap_or(443);
        let stream =
            TlsStream::connect(&host, port).await.map_err(Error::new)?;

        let key = base64::engine::general_purpose::STANDARD
            .encode(random_bytes(16));
        let mut target = self.url.path().to_string();
        if let Some(q) = self.url.query() {
            target.push('?');
            target.push_str(q);
        }
        let host_hdr = match self.url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.clone(),
        };

        let mut req = format!("GET {target} HTTP/1.1\r\n");
        req.push_str(&format!("Host: {host_hdr}\r\n"));
        req.push_str(
            "Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n",
        );
        req.push_str(&format!("Sec-WebSocket-Key: {key}\r\n"));
        for (k, v) in &self.headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");

        stream
            .write_all(req.as_bytes())
            .await
            .map_err(Error::new)?;
        let head =
            stream.read_until(b"\r\n\r\n").await.map_err(Error::new)?;
        let status = parse_status(&head)?;
        if status != 101 {
            return Err(Error::new(format!(
                "websocket upgrade failed: HTTP {status}"
            )));
        }
        Ok(UpgradeResponse { stream })
    }
}

pub struct UpgradeResponse {
    stream: TlsStream,
}

impl UpgradeResponse {
    pub async fn into_websocket(self) -> Result<WebSocket, Error> {
        Ok(WebSocket::new(self.stream))
    }
}

fn parse_status(head: &[u8]) -> Result<u16, Error> {
    let text = String::from_utf8_lossy(head);
    let line = text.lines().next().unwrap_or("");
    line.split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| Error::new(format!("bad status line: {line}")))
}
