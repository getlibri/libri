//! The pieces of a call with the chat itself: which of the chat's words
//! are worth saying aloud, what to say while a tool runs, and the voice
//! that says them. The person's speech is heard by the host's
//! speech-to-text (stt.rs), whose voice detector ends each utterance.
//!
//! A call carries the person's words into the main chat as messages marked
//! "(call)", so the chat's own model, memory and tools answer, and speaks
//! the chat's reply back sentence by sentence (site.rs `call_socket`).

/// Turns the chat's reply, as it streams, into sentences to say: code,
/// tables, links and paths stay on the page, and markdown's marks go.
#[derive(Default)]
pub struct Speakable {
    pending: String,
    fenced: bool,
}

/// The longest sentence said at once; a run-on is cut at a comma or space.
const SENTENCE_MAX: usize = 280;

impl Speakable {
    /// Takes more of the reply; gives the sentences now complete.
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.pending.push_str(text);
        let mut out = Vec::new();
        while let Some(end) = self.boundary() {
            let piece: String = self.pending.drain(..end).collect();
            out.extend(self.speak(&piece));
        }
        out
    }

    /// What is left once the reply is done.
    pub fn finish(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.pending);
        let said = self.speak(&rest);
        self.fenced = false;
        said.into_iter().collect()
    }

    /// Where the first complete piece ends: after a line, or after a
    /// sentence's stop followed by a space.
    fn boundary(&self) -> Option<usize> {
        let p = &self.pending;
        let bytes = p.as_bytes();
        for (i, c) in p.char_indices() {
            if c == '\n' {
                return Some(i + 1);
            }
            // Inside a fence every line is code: only a line's end counts.
            if self.fenced {
                continue;
            }
            if matches!(c, '.' | '!' | '?') && bytes.get(i + 1) == Some(&b' ') {
                // "e.g. " and a version's "0.9." are not a sentence's end.
                let before = &p[..i];
                let word = before.rsplit(' ').next().unwrap_or("");
                if word.len() <= 1 || word.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    continue;
                }
                if matches!(
                    word.to_ascii_lowercase().as_str(),
                    "e.g" | "i.e" | "etc" | "vs" | "mr" | "mrs" | "dr"
                ) {
                    continue;
                }
                return Some(i + 2);
            }
        }
        if p.len() > SENTENCE_MAX {
            let cut = p[..SENTENCE_MAX]
                .rfind(", ")
                .map(|i| i + 2)
                .or_else(|| p[..SENTENCE_MAX].rfind(' ').map(|i| i + 1))
                .unwrap_or(SENTENCE_MAX);
            return Some(floor_char(p, cut));
        }
        None
    }

    /// A piece said aloud, or nothing when it is code, a table or a mark.
    fn speak(&mut self, piece: &str) -> Option<String> {
        let line = piece.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            self.fenced = !self.fenced;
            return None;
        }
        // An indented block is code, but an indented list item is a list.
        let indented = (piece.starts_with("    ") || piece.starts_with('\t'))
            && !line.starts_with(['-', '*', '>'])
            && !line.starts_with(|c: char| c.is_ascii_digit());
        if self.fenced || line.starts_with('|') || indented {
            return None;
        }
        let said = plain(line);
        let words = said.split_whitespace().count();
        (words > 0 && said.chars().any(char::is_alphabetic)).then_some(said)
    }
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A line of markdown as it is said: headings, bullets and emphasis go,
/// a link keeps its words, inline code its word unless it is a path or an
/// address, which is left to the page.
fn plain(line: &str) -> String {
    let line = line.trim_start_matches('#').trim_start();
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("> "))
        .unwrap_or(line);
    // A numbered list's number.
    let line = match line.split_once(". ") {
        Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => rest,
        _ => line,
    };
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find(['[', '`']) {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        if let Some(code) = after.strip_prefix('`') {
            match code.find('`') {
                Some(j) => {
                    let word = &code[..j];
                    if !looks_like_path(word) {
                        out.push_str(word);
                    }
                    rest = &code[j + 1..];
                }
                None => {
                    rest = code;
                }
            }
        } else if let Some(close) = after.find("](") {
            let words = &after[1..close];
            match after[close..].find(')') {
                Some(j) => {
                    out.push_str(words);
                    rest = &after[close + j + 1..];
                }
                None => {
                    out.push('[');
                    rest = &after[1..];
                }
            }
        } else {
            out.push('[');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    let words: Vec<&str> = out
        .split_whitespace()
        .filter(|w| !looks_like_path(w.trim_matches(|c: char| ",.;:()".contains(c))))
        .collect();
    words
        .join(" ")
        .replace("**", "")
        .replace("__", "")
        .replace(" ,", ",")
}

/// A path, an address or a hash: said aloud it is noise.
fn looks_like_path(w: &str) -> bool {
    w.starts_with("http://")
        || w.starts_with("https://")
        || w.starts_with('/')
        || w.starts_with("~/")
        || w.starts_with("./")
        || (w.contains('/') && !w.contains(' ') && w.len() > 3)
        || (w.len() >= 7
            && w.chars().all(|c| c.is_ascii_hexdigit())
            && w.chars().any(|c| c.is_ascii_digit()))
}

/// What to say while the chat runs tool `name` and has nothing to say yet.
pub fn progress(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    let has = |s: &str| n.contains(s);
    if has("mail") {
        "Checking your mail."
    } else if has("event") || has("calendar") {
        "Checking your calendar."
    } else if has("recall") || has("hippo") || has("memory") {
        "Looking back through our chat."
    } else if has("search") || has("fetch") || has("web") {
        "Looking that up."
    } else if has("camera") || has("hob") || has("weather") || has("air") {
        "Checking the house."
    } else if has("spawn") || has("tell_agent") || has("worker") {
        "Passing that to a worker."
    } else if has("bash") || has("terminal") || has("run") {
        "Working on the machine."
    } else {
        "Working on it."
    }
}

/// What speaks a call's replies. A call is offered only where speech can
/// be heard as the person's dictation choice stands (stt.rs). Speaking is
/// the host's own voice (Kokoro, tts.rs) once its model is here, else a
/// deployment's speech service with OpenAI's API, else xAI's voice.
#[derive(Clone)]
pub struct Engine {
    voice: Voice,
}

#[derive(Clone)]
enum Voice {
    Local(std::sync::Arc<crate::tts::Service>),
    OpenAi(OpenAi),
    Xai {
        state: std::path::PathBuf,
        voice: String,
    },
}

/// A speaking service with OpenAI's API: its base address, key, model
/// and voice. Hearing goes through stt.rs.
#[derive(Clone)]
pub struct OpenAi {
    pub base: String,
    pub key: Option<String>,
    pub voice_model: String,
    pub voice: String,
}

impl OpenAi {
    /// From the deployment: `LIBRI_SPEECH_API` (the base, e.g.
    /// `http://127.0.0.1:8880/v1`), `LIBRI_SPEECH_KEY_FILE`,
    /// `LIBRI_SPEECH_VOICE_MODEL` (default `tts-1`), `LIBRI_SPEECH_VOICE`
    /// (default `alloy`). None when no base is set.
    pub fn from_env() -> Option<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let base = var("LIBRI_SPEECH_API")?.trim_end_matches('/').to_owned();
        let key = var("LIBRI_SPEECH_KEY_FILE")
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|k| k.trim().to_owned());
        Some(Self {
            base,
            key,
            voice_model: var("LIBRI_SPEECH_VOICE_MODEL").unwrap_or_else(|| "tts-1".into()),
            voice: var("LIBRI_SPEECH_VOICE").unwrap_or_else(|| "alloy".into()),
        })
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(60)))
                .http_status_as_error(false)
                .build(),
        )
    }

    fn speak(&self, text: &str) -> Result<Vec<u8>, String> {
        let mut ask = Self::agent().post(&format!("{}/audio/speech", self.base));
        if let Some(k) = &self.key {
            ask = ask.header("Authorization", &format!("Bearer {k}"));
        }
        let mut resp = ask
            .send_json(serde_json::json!({
                "model": self.voice_model,
                "input": text,
                "voice": self.voice,
                "response_format": "mp3",
            }))
            .map_err(|e| format!("speech api: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(32 << 20)
            .read_to_vec()
            .map_err(|e| format!("speech api: {status}: {e}"))?;
        if status >= 400 {
            return Err(format!(
                "speech api: {status}: {}",
                String::from_utf8_lossy(&body)
            ));
        }
        Ok(body)
    }
}

impl Engine {
    /// A call's engine, or none when nothing here can hear or speak.
    pub fn pick(
        state: &std::path::Path,
        voice: Option<String>,
        tts: &std::sync::Arc<crate::tts::Service>,
    ) -> Option<Self> {
        if !crate::stt::available(state) {
            return None;
        }
        let voice = if crate::stt::LOCAL_BUILT && crate::tts::installed(state) {
            Voice::Local(tts.clone())
        } else if let Some(api) = OpenAi::from_env() {
            Voice::OpenAi(api)
        } else {
            Voice::Xai {
                state: state.to_owned(),
                voice: voice?,
            }
        };
        Some(Self { voice })
    }

    /// Readies the voice ahead of the first sentence.
    pub fn warm(&self) {
        if let Voice::Local(tts) = &self.voice
            && let Err(e) = tts.warm()
        {
            eprintln!("libri: call voice: {e}");
        }
    }

    /// A sentence spoken, as one audio file the page can decode (WAV or
    /// MP3).
    pub fn speak(&self, text: &str) -> Result<Vec<u8>, String> {
        match &self.voice {
            Voice::Local(tts) => {
                let (pcm, rate) = tts.speak(text, crate::tts::DEFAULT_VOICE, 1.0)?;
                Ok(crate::stt::wav16(&pcm, rate))
            }
            Voice::OpenAi(api) => api.speak(text),
            Voice::Xai { state, voice } => {
                let mut audio = Vec::new();
                crate::x::speak(state, text, voice, |b| audio.extend_from_slice(b))?;
                Ok(audio)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_come_as_they_complete() {
        let mut s = Speakable::default();
        assert!(s.push("Your calendar is clear").is_empty());
        assert_eq!(
            s.push(" tomorrow. The gas bill"),
            vec!["Your calendar is clear tomorrow."]
        );
        assert_eq!(s.push(" is due Sunday."), Vec::<String>::new());
        assert_eq!(s.finish(), vec!["The gas bill is due Sunday."]);
    }

    #[test]
    fn code_tables_and_paths_stay_on_the_page() {
        let mut s = Speakable::default();
        let said = s.push(
            "Here it is:\n```rust\nfn main() {}\n```\n| a | b |\nIt lives in `~/ark/libri/src/call.rs`, see [the docs](https://x.y).\n",
        );
        assert_eq!(said, vec!["Here it is:", "It lives in, see the docs."]);
    }

    #[test]
    fn markdown_marks_go_and_numbers_are_not_sentence_ends() {
        let mut s = Speakable::default();
        let said = s.push("- **Libri 0.9.74** is live, e.g. the CPU fix. Done.\n");
        assert_eq!(
            said,
            vec!["Libri 0.9.74 is live, e.g. the CPU fix.", "Done."]
        );
    }

    #[test]
    fn progress_names_what_runs() {
        assert_eq!(
            progress("mcp__fastmail__search_email"),
            "Checking your mail."
        );
        assert_eq!(
            progress("mcp__hippo2__recall"),
            "Looking back through our chat."
        );
        assert_eq!(progress("Bash"), "Working on the machine.");
    }
}
