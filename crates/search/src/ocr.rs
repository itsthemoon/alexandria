use std::collections::HashMap;

pub const HEADER: &str = "alexandria-ocr 1";

#[derive(Debug, Clone, Default)]
pub struct Ocr {
    files: HashMap<String, String>,
}

impl Ocr {
    pub fn parse(bytes: &[u8]) -> Ocr {
        let text = String::from_utf8_lossy(bytes);
        let mut files = HashMap::new();
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(HEADER) {
            return Ocr { files };
        }
        let mut current: Option<(String, String)> = None;
        for line in lines {
            if let Some(rest) = line.strip_prefix("@@ ") {
                if let Some((path, body)) = current.take() {
                    files.insert(path, body);
                }
                let path = rest.rsplit_once(' ').map(|(p, _)| p).unwrap_or(rest);
                current = Some((path.to_string(), String::new()));
                continue;
            }
            if let Some((_, body)) = current.as_mut() {
                if let Some(escaped) = line.strip_prefix('\\') {
                    if escaped.starts_with("@@ ") {
                        body.push_str(escaped);
                        body.push('\n');
                        continue;
                    }
                }
                body.push_str(line);
                body.push('\n');
            }
        }
        if let Some((path, body)) = current {
            files.insert(path, body);
        }
        Ocr { files }
    }

    pub fn text(&self, url: &str) -> Option<&str> {
        self.files.get(url).map(|s| s.as_str()).filter(|s| s.trim().len() > 20)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn to_text(&self, paths: &[(&str, u64)], texts: &[String]) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        for ((path, size), text) in paths.iter().zip(texts) {
            out.push_str(&format!("@@ {path} {size}\n"));
            for line in text.lines() {
                if line.starts_with("@@ ") {
                    out.push('\\');
                }
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }
}
