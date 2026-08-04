/// Query parameters for `GET /Videos/{itemId}/stream`.
#[derive(Clone, Debug, Default)]
pub struct VideoStreamQuery {
    container: Option<String>,
    static_stream: Option<bool>,
    params: Option<String>,
    tag: Option<String>,
    play_session_id: Option<String>,
    media_source_id: Option<String>,
    device_id: Option<String>,
    extra: Vec<(String, String)>,
}

impl VideoStreamQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// The video container (e.g. `mp4`, `mkv`).
    pub fn container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    /// If true, the original file will be streamed statically without encoding.
    pub fn static_stream(mut self, value: bool) -> Self {
        self.static_stream = Some(value);
        self
    }

    /// The streaming parameters.
    pub fn params(mut self, value: impl Into<String>) -> Self {
        self.params = Some(value.into());
        self
    }

    /// The tag.
    pub fn tag(mut self, value: impl Into<String>) -> Self {
        self.tag = Some(value.into());
        self
    }

    /// The play session id (typically returned by `PlaybackInfo`).
    pub fn play_session_id(mut self, value: impl Into<String>) -> Self {
        self.play_session_id = Some(value.into());
        self
    }

    /// The media source id (for alternate versions).
    pub fn media_source_id(mut self, value: impl Into<String>) -> Self {
        self.media_source_id = Some(value.into());
        self
    }

    /// The device id of the client requesting.
    pub fn device_id(mut self, value: impl Into<String>) -> Self {
        self.device_id = Some(value.into());
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((key.into(), value.into()));
        self
    }

    pub(crate) fn has_device_id(&self) -> bool {
        self.device_id.is_some()
    }

    pub(crate) fn to_params(&self) -> Vec<(String, String)> {
        self.to_params_with_container(true)
    }

    pub(crate) fn to_params_without_container(&self) -> Vec<(String, String)> {
        self.to_params_with_container(false)
    }

    fn to_params_with_container(&self, include_container: bool) -> Vec<(String, String)> {
        let mut q = Vec::new();
        if include_container && let Some(v) = &self.container {
            q.push(("container".to_owned(), v.clone()));
        }
        if let Some(v) = self.static_stream {
            q.push(("static".to_owned(), v.to_string()));
        }
        if let Some(v) = &self.params {
            q.push(("params".to_owned(), v.clone()));
        }
        if let Some(v) = &self.tag {
            q.push(("tag".to_owned(), v.clone()));
        }
        if let Some(v) = &self.play_session_id {
            q.push(("playSessionId".to_owned(), v.clone()));
        }
        if let Some(v) = &self.media_source_id {
            q.push(("mediaSourceId".to_owned(), v.clone()));
        }
        if let Some(v) = &self.device_id {
            q.push(("deviceId".to_owned(), v.clone()));
        }

        q.extend(self.extra.clone());
        q
    }
}
