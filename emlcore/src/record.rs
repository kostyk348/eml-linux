//! Ordered RFC 822 record: headers + raw body. Format-compatible with EML-IPC.

#[derive(Debug, Clone, Default)]
pub struct Record {
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Record {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or replace, case-insensitively) a header.
    pub fn set(&mut self, key: &str, value: impl Into<String>) -> &mut Self {
        let value = value.into();
        if let Some(slot) = self
            .headers
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            slot.1 = value;
        } else {
            self.headers.push((key.to_string(), value));
        }
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(&self.body).trim().to_string()
    }

    /// Render to RFC 822 bytes (LF line endings, trailing blank line + body).
    pub fn render(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in &self.headers {
            out.extend_from_slice(k.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(v.as_bytes());
            out.push(b'\n');
        }
        out.push(b'\n');
        out.extend_from_slice(&self.body);
        if !self.body.ends_with(b"\n") {
            out.push(b'\n');
        }
        out
    }

    /// Parse the first RFC 822 record in `data`.
    pub fn parse(data: &[u8]) -> Self {
        let (head, body) = match find_subslice(data, b"\n\n") {
            Some(i) => (&data[..i], &data[i + 2..]),
            None => (data, &data[data.len()..]),
        };
        let text = String::from_utf8_lossy(head);
        let mut headers = Vec::new();
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            if let Some(idx) = line.find(':') {
                let k = line[..idx].trim().to_string();
                let v = line[idx + 1..].trim().to_string();
                headers.push((k, v));
            }
        }
        Record {
            headers,
            body: body.to_vec(),
        }
    }
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut r = Record::new();
        r.set("From", "<a@b>");
        r.set("X-Event", "START");
        r.body = b"{\"x\":1}".to_vec();
        let bytes = r.render();
        let p = Record::parse(&bytes);
        assert_eq!(p.get("From"), Some("<a@b>"));
        assert_eq!(p.get("X-Event"), Some("START"));
        assert_eq!(p.body_str(), "{\"x\":1}");
    }

    #[test]
    fn set_replaces_case_insensitive() {
        let mut r = Record::new();
        r.set("X-A", "1");
        r.set("x-a", "2");
        assert_eq!(r.headers.len(), 1);
        assert_eq!(r.get("X-A"), Some("2"));
    }
}
