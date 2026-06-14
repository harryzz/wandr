//! Minimal `reqwest::multipart` subset. Only used by CDN attachment upload
//! (`upload_to_cdn0`), which is out of scope for the text-only v1 — present so the
//! fork compiles. The encoder is standard multipart/form-data (untested in v1).

use crate::Error;

pub struct Part {
    body: Vec<u8>,
    mime: Option<String>,
    file_name: Option<String>,
}

impl Part {
    pub fn stream(body: impl Into<Vec<u8>>) -> Part {
        Part {
            body: body.into(),
            mime: None,
            file_name: None,
        }
    }

    pub fn mime_str(mut self, mime: &str) -> Result<Part, Error> {
        self.mime = Some(mime.to_string());
        Ok(self)
    }

    pub fn file_name(mut self, name: impl Into<String>) -> Part {
        self.file_name = Some(name.into());
        self
    }
}

#[derive(Default)]
pub struct Form {
    fields: Vec<(String, Part)>,
}

impl Form {
    pub fn new() -> Form {
        Form::default()
    }

    pub fn text(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Form {
        self.fields.push((
            name.into(),
            Part {
                body: value.into().into_bytes(),
                mime: None,
                file_name: None,
            },
        ));
        self
    }

    pub fn part(mut self, name: impl Into<String>, part: Part) -> Form {
        self.fields.push((name.into(), part));
        self
    }

    pub(crate) fn encode(&self) -> (String, Vec<u8>) {
        let boundary = "----wartBoundary7MA4YWxkTrZu0gW";
        let mut body = Vec::new();
        for (name, part) in &self.fields {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            let mut disp =
                format!("Content-Disposition: form-data; name=\"{name}\"");
            if let Some(fname) = &part.file_name {
                disp.push_str(&format!("; filename=\"{fname}\""));
            }
            disp.push_str("\r\n");
            body.extend_from_slice(disp.as_bytes());
            if let Some(mime) = &part.mime {
                body.extend_from_slice(
                    format!("Content-Type: {mime}\r\n").as_bytes(),
                );
            }
            body.extend_from_slice(b"\r\n");
            body.extend_from_slice(&part.body);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (
            format!("multipart/form-data; boundary={boundary}"),
            body,
        )
    }
}
