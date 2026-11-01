//! Splits `<think>…</think>` sections out of streamed content.
//!
//! Many reasoning models served through OpenAI-compatible servers (and Ollama
//! without `think`) inline their chain of thought in the content stream. Tags
//! can be split across chunks, so partial tags are buffered.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Reasoning(String),
}

#[derive(Debug, Default)]
pub struct ThinkSplitter {
    in_think: bool,
    buf: String,
    seen_text: bool,
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

impl ThinkSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a chunk and returns the pieces that are certain.
    pub fn push(&mut self, chunk: &str) -> Vec<Piece> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        loop {
            let tag = if self.in_think { CLOSE } else { OPEN };
            if let Some(pos) = self.buf.find(tag) {
                let before: String = self.buf[..pos].to_string();
                self.emit(&mut out, before);
                self.buf.drain(..pos + tag.len());
                self.in_think = !self.in_think;
                if !self.in_think {
                    // Drop the blank line models put after </think>.
                    let trimmed = self.buf.trim_start_matches(['\n', '\r']).len();
                    let skip = self.buf.len() - trimmed;
                    self.buf.drain(..skip);
                }
                continue;
            }
            // Keep a possible partial tag at the end.
            let keep = partial_suffix(&self.buf, tag);
            let emit_len = self.buf.len() - keep;
            if emit_len > 0 {
                let head: String = self.buf[..emit_len].to_string();
                self.buf.drain(..emit_len);
                self.emit(&mut out, head);
            }
            break;
        }
        out
    }

    /// Flushes buffered text at the end of the stream.
    pub fn finish(&mut self) -> Vec<Piece> {
        let rest = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        self.emit(&mut out, rest);
        out
    }

    fn emit(&mut self, out: &mut Vec<Piece>, s: String) {
        if s.is_empty() {
            return;
        }
        if self.in_think {
            out.push(Piece::Reasoning(s));
        } else {
            if !self.seen_text && s.trim().is_empty() {
                return; // whitespace before the first real text
            }
            self.seen_text = true;
            out.push(Piece::Text(s));
        }
    }
}

/// Length of the longest suffix of `s` that is a proper prefix of `tag`.
fn partial_suffix(s: &str, tag: &str) -> usize {
    let max = tag.len().saturating_sub(1).min(s.len());
    for len in (1..=max).rev() {
        if s.is_char_boundary(s.len() - len) && tag.starts_with(&s[s.len() - len..]) {
            return len;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&str]) -> (String, String) {
        let mut s = ThinkSplitter::new();
        let mut text = String::new();
        let mut reasoning = String::new();
        for c in chunks {
            for p in s.push(c) {
                match p {
                    Piece::Text(t) => text.push_str(&t),
                    Piece::Reasoning(r) => reasoning.push_str(&r),
                }
            }
        }
        for p in s.finish() {
            match p {
                Piece::Text(t) => text.push_str(&t),
                Piece::Reasoning(r) => reasoning.push_str(&r),
            }
        }
        (text, reasoning)
    }

    #[test]
    fn splits_across_chunks() {
        let (t, r) = run(&["<th", "ink>Let me", " think</thi", "nk>\n\nThe answer", " is 4."]);
        assert_eq!(r, "Let me think");
        assert_eq!(t, "The answer is 4.");
    }

    #[test]
    fn plain_text_passes_through() {
        let (t, r) = run(&["Hello ", "<b>world</b> a < b"]);
        assert_eq!(t, "Hello <b>world</b> a < b");
        assert!(r.is_empty());
    }

    #[test]
    fn unicode_partial_boundary() {
        let (t, _) = run(&["日本", "語<", "x"]);
        assert_eq!(t, "日本語<x");
    }
}
