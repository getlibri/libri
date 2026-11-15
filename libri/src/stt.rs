//! Speech to text: the host's one way to turn speech into words, for
//! dictation and for calls alike.
//!
//! Where it runs is the person's choice (Settings, Voice): this host, with
//! a local model (the default; no audio leaves the host), xAI on their Grok
//! subscription, or any OpenAI-compatible transcription API. Every choice
//! gives the same stream of events from 16 kHz mono PCM:
//!
//! - `SpeechStarted` the moment the voice detector hears speech begin, for
//!   a call to stop speaking over the person;
//! - `Partial`, the words so far of the utterance under way, each replacing
//!   the last (local streaming models only);
//! - `Final`, the utterance's words once it ends: a pause of the session's
//!   trailing silence, or its longest length.
//!
//! The local engine is sherpa-onnx, as omp ships it: Nemotron streaming
//! models give words while the person speaks, and the Silero voice detector
//! decides where an utterance ends. Models are downloaded on demand into the
//! state directory, checked against their recorded hashes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The sample rate every session takes.
pub const RATE: u32 = 16_000;

/// Where transcription runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// A local model on this host.
    #[default]
    Local,
    /// xAI, on the person's Grok connection.
    Xai,
    /// An OpenAI-compatible `/audio/transcriptions` endpoint.
    Openai,
}

/// The person's choice, kept in `<state>/stt.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub provider: Provider,
    /// The local model, by catalog id.
    pub model: String,
    /// The OpenAI-compatible API's base URL, e.g. `https://api.openai.com/v1`.
    pub openai_url: String,
    /// Its model name, e.g. `gpt-4o-transcribe`.
    pub openai_model: String,
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            provider: Provider::Local,
            model: DEFAULT_MODEL.to_owned(),
            openai_url: "https://api.openai.com/v1".to_owned(),
            openai_model: "gpt-4o-transcribe".to_owned(),
        }
    }
}

const CHOICE: &str = "stt.json";
/// The OpenAI-compatible API's key, apart from the choice, readable by the
/// host alone.
const OPENAI_KEY: &str = "stt-openai-key";

pub fn choice(state: &Path) -> Choice {
    std::fs::read(state.join(CHOICE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn choose(state: &Path, choice: &Choice) -> Result<(), String> {
    if model(&choice.model).is_none() {
        return Err(format!("no local model {}", choice.model));
    }
    let text = serde_json::to_string_pretty(choice).map_err(|e| e.to_string())?;
    let partial = state.join(format!("{CHOICE}.partial"));
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, state.join(CHOICE)).map_err(|e| e.to_string())
}

/// Keeps the OpenAI-compatible API's key; an empty key forgets it.
pub fn set_openai_key(state: &Path, key: &str) -> Result<(), String> {
    let path = state.join(OPENAI_KEY);
    if key.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    crate::host::write_private(&path, key.trim())
}

pub fn has_openai_key(state: &Path) -> bool {
    state.join(OPENAI_KEY).is_file()
}

/// A local model Libri knows how to fetch and run.
#[derive(Debug, Serialize)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    /// Languages it hears, ISO 639-1; `auto` where it detects them itself.
    pub languages: &'static [&'static str],
    #[serde(skip)]
    archive: &'static str,
    #[serde(skip)]
    sha256: &'static str,
    /// The archive's size in bytes.
    pub bytes: u64,
}

/// sherpa-onnx's model releases.
const RELEASES: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";

/// The default: the best English streaming model that runs live on a CPU.
/// On a 48-thread Threadripper PRO 9965WX it writes 6.6 s of speech in 0.6 s.
pub const DEFAULT_MODEL: &str = "nemotron-en";

pub const MODELS: &[Model] = &[
    Model {
        id: "nemotron-en",
        name: "Nemotron Streaming English 0.6B",
        about: "English, words as you speak, with punctuation. The default.",
        languages: &["en"],
        archive: "sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25",
        sha256: "78e2b79fcf7271553a74402a76b771b09ea40117a39566a79f52235b23db6358",
        bytes: 463_945_051,
    },
    Model {
        id: "nemotron-multilingual",
        name: "Nemotron 3.5 Streaming 0.6B",
        about: "About 40 languages, detected as you speak, words as you speak.",
        languages: &["auto"],
        archive: "sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-560ms-int8-2026-06-11",
        sha256: "c6bf5e0df765f9d5b43bc9e0536d4b4b3e7d40bdf5ecf13e45f134c51c05ae3a",
        bytes: 475_271_763,
    },
];

/// Silero's voice detector, which decides where utterances end.
const VAD_FILE: &str = "silero_vad.onnx";
const VAD_SHA256: &str = "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6";

pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

fn models_dir(state: &Path) -> PathBuf {
    state.join("stt").join("models")
}

fn model_dir(state: &Path, m: &Model) -> PathBuf {
    models_dir(state).join(m.archive)
}

pub fn installed(state: &Path, m: &Model) -> bool {
    model_dir(state, m).join("tokens.txt").is_file()
}

/// Whether this build carries the local engine; a static bundle does not.
pub const LOCAL_BUILT: bool = cfg!(feature = "local-speech");

/// Whether speech can be written down as things stand: the local engine is
/// built in, or the chosen service has its sign-in.
pub fn available(state: &Path) -> bool {
    match choice(state).provider {
        Provider::Local => LOCAL_BUILT,
        Provider::Xai => crate::x::connected(state).is_some(),
        Provider::Openai => has_openai_key(state),
    }
}

/// What a session hears.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SpeechStarted {
        utterance: u32,
    },
    Partial {
        utterance: u32,
        text: String,
    },
    Final {
        utterance: u32,
        text: String,
        /// Seconds from the session's start.
        start: f32,
        end: f32,
    },
}

/// When an utterance is over.
#[derive(Clone, Copy, Debug)]
pub struct Endpoint {
    pub trailing_silence_ms: u32,
    pub max_utterance_ms: u32,
}

impl Default for Endpoint {
    fn default() -> Self {
        Self {
            trailing_silence_ms: 800,
            max_utterance_ms: 20_000,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The language, ISO 639-1, where the model takes one; none, its own
    /// detection or its only language.
    pub lang: Option<String>,
    pub endpoint: Endpoint,
}

/// Download progress by model id: bytes so far, and the total.
type Progress = HashMap<String, (u64, u64)>;

/// The host's speech-to-text: one per daemon, shared by every session.
pub struct Service {
    state: PathBuf,
    #[cfg(feature = "local-speech")]
    loaded: Mutex<Option<local::Loaded>>,
    downloads: Arc<Mutex<Progress>>,
}

/// A loaded model goes after this long unused.
const IDLE: Duration = Duration::from_secs(10 * 60);

impl Service {
    pub fn new(state: PathBuf) -> Arc<Self> {
        let service = Arc::new(Self {
            state,
            #[cfg(feature = "local-speech")]
            loaded: Mutex::new(None),
            downloads: Arc::default(),
        });
        // Unloads an idle model, so a host that dictated once doesn't hold
        // its gigabyte for good.
        let weak = Arc::downgrade(&service);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(60));
                let Some(service) = weak.upgrade() else {
                    return;
                };
                service.unload_idle();
            }
        });
        service
    }

    pub fn state(&self) -> &Path {
        &self.state
    }

    fn unload_idle(&self) {
        #[cfg(feature = "local-speech")]
        {
            let mut loaded = self.loaded.lock().unwrap();
            if loaded.as_ref().is_some_and(|l| l.idle() > IDLE) {
                *loaded = None;
            }
        }
    }

    /// A session on the person's choice.
    pub fn open(self: &Arc<Self>, options: Options) -> Result<Session, String> {
        let choice = choice(&self.state);
        let vad = self.vad_path()?;
        match choice.provider {
            Provider::Local => self.open_local(&choice, options, &vad),
            provider => Ok(Session::Cloud(cloud::Session::new(
                self.state.clone(),
                provider,
                options,
                &vad,
            )?)),
        }
    }

    #[cfg(feature = "local-speech")]
    fn open_local(&self, choice: &Choice, options: Options, vad: &Path) -> Result<Session, String> {
        let m = model(&choice.model).ok_or("no such local model")?;
        if !installed(&self.state, m) {
            let _ = self.download(m.id);
            return Err(format!(
                "the speech model {} is still downloading{}",
                m.name,
                self.progress_note(m.id)
            ));
        }
        let mut loaded = self.loaded.lock().unwrap();
        if loaded.as_ref().is_none_or(|l| l.id != m.id) {
            *loaded = Some(local::Loaded::load(m, &model_dir(&self.state, m))?);
        }
        let recognizer = loaded.as_ref().unwrap().use_now();
        Ok(Session::Local(local::Session::new(
            recognizer, options, vad,
        )?))
    }

    #[cfg(not(feature = "local-speech"))]
    fn open_local(&self, _: &Choice, _: Options, _: &Path) -> Result<Session, String> {
        Err("this build of Libri has no local speech engine; choose xAI or an API in Settings, Voice".into())
    }

    fn progress_note(&self, id: &str) -> String {
        match self.downloads.lock().unwrap().get(id) {
            Some((got, total)) if *total > 0 => format!(" ({}%)", got * 100 / total),
            _ => String::new(),
        }
    }

    /// Bytes so far and total, by model id, of downloads under way.
    pub fn downloads(&self) -> Progress {
        self.downloads.lock().unwrap().clone()
    }

    /// Starts fetching a model unless it is here or on its way.
    pub fn download(&self, id: &str) -> Result<(), String> {
        let m = model(id).ok_or("no such local model")?;
        if installed(&self.state, m) {
            return Ok(());
        }
        {
            let mut downloads = self.downloads.lock().unwrap();
            if downloads.contains_key(id) {
                return Ok(());
            }
            downloads.insert(id.to_owned(), (0, m.bytes));
        }
        let state = self.state.clone();
        let downloads = self.downloads.clone();
        std::thread::spawn(move || {
            let got = fetch::model(&state, m, &downloads);
            downloads.lock().unwrap().remove(m.id);
            if let Err(e) = got {
                eprintln!("libri: speech model {}: {e}", m.id);
            }
        });
        Ok(())
    }

    /// The voice detector's file, fetched the first time.
    fn vad_path(&self) -> Result<PathBuf, String> {
        let path = models_dir(&self.state).join(VAD_FILE);
        if !path.is_file() {
            fetch::file(
                &format!("{RELEASES}/{VAD_FILE}"),
                VAD_SHA256,
                &path,
                &mut |_, _| {},
            )?;
        }
        Ok(path)
    }
}

/// A stream of audio being heard.
pub enum Session {
    #[cfg(feature = "local-speech")]
    Local(local::Session),
    Cloud(cloud::Session),
}

impl Session {
    /// Hears 16 kHz mono PCM, any length.
    pub fn push(&mut self, pcm: &[i16]) -> Result<Vec<Event>, String> {
        let samples: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
        match self {
            #[cfg(feature = "local-speech")]
            Session::Local(s) => Ok(s.push(&samples)),
            Session::Cloud(s) => s.push(&samples),
        }
    }

    /// Ends the audio: the utterance under way is written down.
    pub fn finish(&mut self) -> Result<Vec<Event>, String> {
        match self {
            #[cfg(feature = "local-speech")]
            Session::Local(s) => Ok(s.finish()),
            Session::Cloud(s) => s.finish(),
        }
    }

    /// Which provider and model it runs on, for the stream's `ready`.
    pub fn about(&self) -> (Provider, String) {
        match self {
            #[cfg(feature = "local-speech")]
            Session::Local(s) => (Provider::Local, s.model.clone()),
            Session::Cloud(s) => (s.provider, String::new()),
        }
    }
}

/// The words of every final event, in order.
pub fn words(events: &[Event]) -> String {
    let parts: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            Event::Final { text, .. } if !text.trim().is_empty() => Some(text.trim()),
            _ => None,
        })
        .collect();
    parts.join(" ")
}

/// A recording in whatever container the device made (WebM, Ogg, MP4)
/// as 16 kHz mono PCM, by ffmpeg.
pub fn decode(audio: &[u8]) -> Result<Vec<i16>, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i", "pipe:0"])
        .args(["-ac", "1", "-ar", "16000", "-f", "s16le", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    let audio = audio.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&audio));
    let out = child
        .wait_with_output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let _ = writer.join();
    if !out.status.success() {
        return Err(format!(
            "ffmpeg: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect())
}

/// A whole recording written down on the person's choice.
pub fn transcribe(service: &Arc<Service>, audio: &[u8], kind: &str) -> Result<String, String> {
    let choice = choice(service.state());
    match choice.provider {
        // The cloud takes the recording as it is, which keeps its quality
        // and the request small.
        Provider::Xai => {
            let name = crate::x::audio_name(kind).ok_or("unsupported audio")?;
            crate::x::transcribe(service.state(), audio, name)
        }
        Provider::Openai => {
            let name = crate::x::audio_name(kind).ok_or("unsupported audio")?;
            cloud::openai(service.state(), &choice, audio, name)
        }
        Provider::Local => {
            let pcm = decode(audio)?;
            let mut session = service.open(Options::default())?;
            let mut events = Vec::new();
            for chunk in pcm.chunks(RATE as usize / 10) {
                events.extend(session.push(chunk)?);
            }
            events.extend(session.finish()?);
            Ok(words(&events))
        }
    }
}

/// The voice detector, set for one session's endpoint.
fn vad_config(vad: &Path, endpoint: Endpoint) -> sherpa_vad::VadModelConfig {
    sherpa_vad::config(vad, endpoint)
}

#[cfg(feature = "local-speech")]
mod sherpa_vad {
    use super::Endpoint;
    pub use sherpa_onnx::VadModelConfig;
    use std::path::Path;

    pub fn config(vad: &Path, endpoint: Endpoint) -> VadModelConfig {
        let mut config = VadModelConfig::default();
        config.silero_vad.model = Some(vad.display().to_string());
        config.silero_vad.threshold = 0.5;
        config.silero_vad.min_silence_duration = endpoint.trailing_silence_ms as f32 / 1000.0;
        config.silero_vad.min_speech_duration = 0.25;
        config.silero_vad.window_size = 512;
        config.silero_vad.max_speech_duration = endpoint.max_utterance_ms as f32 / 1000.0;
        config.sample_rate = super::RATE as i32;
        config.num_threads = 1;
        config
    }
}

#[cfg(not(feature = "local-speech"))]
mod sherpa_vad {
    use super::Endpoint;
    use std::path::Path;
    pub struct VadModelConfig;
    pub fn config(_: &Path, _: Endpoint) -> VadModelConfig {
        VadModelConfig
    }
}

/// Voice detection shared by every session: speech start, and the
/// utterances' ends.
mod detector {
    use super::{Endpoint, RATE};
    use std::path::Path;

    /// What the detector saw in a run of samples.
    pub struct Heard {
        pub started: bool,
        /// Utterances it closed, as their sample spans and samples.
        pub ended: Vec<(usize, Vec<f32>)>,
    }

    #[cfg(feature = "local-speech")]
    pub struct Detector {
        vad: sherpa_onnx::VoiceActivityDetector,
        speaking: bool,
        /// Samples not yet a whole window.
        pending: Vec<f32>,
    }

    #[cfg(feature = "local-speech")]
    impl Detector {
        pub fn new(vad: &Path, endpoint: Endpoint) -> Result<Self, String> {
            let config = super::vad_config(vad, endpoint);
            let seconds = endpoint.max_utterance_ms as f32 / 1000.0 + 10.0;
            let vad = sherpa_onnx::VoiceActivityDetector::create(&config, seconds)
                .ok_or("the voice detector would not load")?;
            Ok(Self {
                vad,
                speaking: false,
                pending: Vec::new(),
            })
        }

        pub fn hear(&mut self, samples: &[f32]) -> Heard {
            self.pending.extend_from_slice(samples);
            let mut started = false;
            let whole = self.pending.len() / 512 * 512;
            for window in self.pending[..whole].chunks(512) {
                self.vad.accept_waveform(window);
                let now = self.vad.detected();
                if now && !self.speaking {
                    started = true;
                }
                self.speaking = now;
            }
            self.pending.drain(..whole);
            Heard {
                started,
                ended: self.drain(),
            }
        }

        pub fn flush(&mut self) -> Vec<(usize, Vec<f32>)> {
            if !self.pending.is_empty() {
                let mut last = std::mem::take(&mut self.pending);
                last.resize(512, 0.0);
                self.vad.accept_waveform(&last);
            }
            self.vad.flush();
            self.speaking = false;
            self.drain()
        }

        fn drain(&mut self) -> Vec<(usize, Vec<f32>)> {
            let mut ended = Vec::new();
            while let Some(segment) = self.vad.front() {
                ended.push((segment.start().max(0) as usize, segment.samples().to_vec()));
                self.vad.pop();
            }
            ended
        }
    }

    #[cfg(not(feature = "local-speech"))]
    pub struct Detector {
        samples: Vec<f32>,
        speaking: bool,
    }

    /// Without the local engine, a plain loudness gate: speech starts on the
    /// first loud window, and the whole recording is one utterance.
    #[cfg(not(feature = "local-speech"))]
    impl Detector {
        pub fn new(_: &Path, _: Endpoint) -> Result<Self, String> {
            Ok(Self {
                samples: Vec::new(),
                speaking: false,
            })
        }

        pub fn hear(&mut self, samples: &[f32]) -> Heard {
            let loud = samples.iter().any(|s| s.abs() > 0.05);
            let started = loud && !self.speaking;
            self.speaking |= loud;
            self.samples.extend_from_slice(samples);
            Heard {
                started,
                ended: Vec::new(),
            }
        }

        pub fn flush(&mut self) -> Vec<(usize, Vec<f32>)> {
            let all = std::mem::take(&mut self.samples);
            if all.is_empty() {
                Vec::new()
            } else {
                vec![(0, all)]
            }
        }
    }

    pub fn seconds(samples: usize) -> f32 {
        samples as f32 / RATE as f32
    }
}

#[cfg(feature = "local-speech")]
mod local {
    use super::detector::{Detector, seconds};
    use super::{Event, Model, Options, RATE};
    use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Silence fed after an utterance ends, so the streaming model's last
    /// chunk is decoded before its words are read.
    const TAIL: usize = RATE as usize * 6 / 10;

    pub struct Loaded {
        pub id: &'static str,
        recognizer: Arc<OnlineRecognizer>,
        used: std::sync::Mutex<Instant>,
    }

    impl Loaded {
        pub fn load(m: &Model, dir: &Path) -> Result<Self, String> {
            let file = |name: &str| Some(dir.join(name).display().to_string());
            let mut config = OnlineRecognizerConfig::default();
            config.model_config.transducer.encoder = file("encoder.int8.onnx");
            config.model_config.transducer.decoder = file("decoder.int8.onnx");
            config.model_config.transducer.joiner = file("joiner.int8.onnx");
            config.model_config.tokens = file("tokens.txt");
            // Four threads keep a 0.6B streaming model well ahead of speech
            // (RTF 0.1 on a Threadripper PRO 9965WX) without taking the host.
            config.model_config.num_threads = 4;
            config.decoding_method = Some("greedy_search".into());
            // Utterance ends are the voice detector's, per session.
            config.enable_endpoint = false;
            let recognizer = OnlineRecognizer::create(&config)
                .ok_or_else(|| format!("the speech model {} would not load", m.id))?;
            Ok(Self {
                id: m.id,
                recognizer: Arc::new(recognizer),
                used: std::sync::Mutex::new(Instant::now()),
            })
        }

        pub fn use_now(&self) -> Arc<OnlineRecognizer> {
            *self.used.lock().unwrap() = Instant::now();
            self.recognizer.clone()
        }

        pub fn idle(&self) -> Duration {
            self.used.lock().unwrap().elapsed()
        }
    }

    pub struct Session {
        pub model: String,
        recognizer: Arc<OnlineRecognizer>,
        stream: OnlineStream,
        lang: Option<String>,
        detector: Detector,
        utterance: u32,
        partial: String,
        /// Samples heard in all, and where the utterance under way began.
        heard: usize,
        began: usize,
        speaking: bool,
        /// The audio since `base` (a sample index), for decoding an ended
        /// utterance whole with the speech the detector's start trims.
        audio: Vec<f32>,
        base: usize,
    }

    /// Audio kept before an utterance's detected start: the voice detector
    /// marks speech only once it is sure, a beat after the first word began.
    const PREROLL: usize = RATE as usize * 8 / 10;

    impl Session {
        pub fn new(
            recognizer: Arc<OnlineRecognizer>,
            options: Options,
            vad: &Path,
        ) -> Result<Self, String> {
            let stream = recognizer.create_stream();
            if let Some(lang) = &options.lang {
                stream.set_option("language", lang);
            }
            Ok(Self {
                model: String::new(),
                stream,
                recognizer,
                lang: options.lang,
                detector: Detector::new(vad, options.endpoint)?,
                utterance: 0,
                partial: String::new(),
                heard: 0,
                began: 0,
                speaking: false,
                audio: Vec::new(),
                base: 0,
            })
        }

        pub fn push(&mut self, samples: &[f32]) -> Vec<Event> {
            self.audio.extend_from_slice(samples);
            let mut events = Vec::new();
            let heard = self.detector.hear(samples);
            if heard.started {
                self.speaking = true;
                self.began = self.heard;
                events.push(Event::SpeechStarted {
                    utterance: self.utterance,
                });
            }
            self.stream.accept_waveform(RATE as i32, samples);
            self.heard += samples.len();
            self.decode();
            if let Some(text) = self.text()
                && text != self.partial
            {
                self.partial = text.clone();
                events.push(Event::Partial {
                    utterance: self.utterance,
                    text,
                });
            }
            for (start, samples) in heard.ended {
                events.extend(self.close(Some((start, samples.len()))));
            }
            events
        }

        pub fn finish(&mut self) -> Vec<Event> {
            let ended = self.detector.flush();
            let mut events = Vec::new();
            for (start, samples) in &ended {
                events.extend(self.close(Some((*start, samples.len()))));
            }
            if ended.is_empty() {
                events.extend(self.close(None));
            }
            events
        }

        /// The words of one whole utterance, from a fresh stream fed all of
        /// it at once: the live stream's own text can fall behind when audio
        /// arrives in real time, and the utterance's samples are all here.
        fn whole(&self, samples: &[f32]) -> String {
            let stream = self.recognizer.create_stream();
            if let Some(lang) = &self.lang {
                stream.set_option("language", lang);
            }
            stream.accept_waveform(RATE as i32, samples);
            stream.accept_waveform(RATE as i32, &vec![0.0; TAIL]);
            stream.input_finished();
            while self.recognizer.is_ready(&stream) {
                self.recognizer.decode(&stream);
            }
            self.recognizer
                .get_result(&stream)
                .map(|r| r.text.trim().to_owned())
                .unwrap_or_default()
        }

        /// Ends the utterance under way: its words (from its whole samples
        /// when the detector gave them), then a fresh start.
        fn close(&mut self, span: Option<(usize, usize)>) -> Vec<Event> {
            let text = match span {
                Some((start, len)) => {
                    let from = start.saturating_sub(PREROLL).max(self.base) - self.base;
                    let to = (start + len + PREROLL / 2)
                        .saturating_sub(self.base)
                        .min(self.audio.len());
                    let words = self.whole(&self.audio[from.min(to)..to]);
                    // Keep only what a later utterance's preroll may need.
                    let keep_from = to.saturating_sub(PREROLL);
                    self.audio.drain(..keep_from);
                    self.base += keep_from;
                    words
                }
                None => {
                    self.stream.accept_waveform(RATE as i32, &vec![0.0; TAIL]);
                    self.decode();
                    self.text().unwrap_or_default()
                }
            };
            // Where the speech stopped, as a cloud session stamps it: the
            // detector's span ends there, before the quiet that closed it.
            let end = span.map_or(self.heard, |(start, len)| start + len);
            self.recognizer.reset(&self.stream);
            if let Some(lang) = &self.lang {
                self.stream.set_option("language", lang);
            }
            let mut events = Vec::new();
            if !text.is_empty() {
                events.push(Event::Final {
                    utterance: self.utterance,
                    text,
                    start: seconds(self.began),
                    end: seconds(end),
                });
                self.utterance += 1;
            }
            self.partial.clear();
            self.speaking = false;
            self.began = self.heard;
            events
        }

        fn decode(&self) {
            while self.recognizer.is_ready(&self.stream) {
                self.recognizer.decode(&self.stream);
            }
        }

        fn text(&self) -> Option<String> {
            let text = self.recognizer.get_result(&self.stream)?.text;
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_owned())
        }
    }
}

mod cloud {
    use super::detector::{Detector, seconds};
    use super::{Choice, Event, Options, Provider, RATE};
    use std::path::{Path, PathBuf};

    /// A session on a service that writes down whole recordings: the voice
    /// detector cuts the utterances, and each is sent once it ends, so it
    /// gives finals and no partials.
    pub struct Session {
        pub provider: Provider,
        state: PathBuf,
        detector: Detector,
        utterance: u32,
    }

    impl Session {
        pub fn new(
            state: PathBuf,
            provider: Provider,
            options: Options,
            vad: &Path,
        ) -> Result<Self, String> {
            Ok(Self {
                provider,
                state,
                detector: Detector::new(vad, options.endpoint)?,
                utterance: 0,
            })
        }

        pub fn push(&mut self, samples: &[f32]) -> Result<Vec<Event>, String> {
            let heard = self.detector.hear(samples);
            let mut events = Vec::new();
            if heard.started {
                events.push(Event::SpeechStarted {
                    utterance: self.utterance,
                });
            }
            for (start, utterance) in heard.ended {
                events.extend(self.write(start, &utterance)?);
            }
            Ok(events)
        }

        pub fn finish(&mut self) -> Result<Vec<Event>, String> {
            let mut events = Vec::new();
            for (start, utterance) in self.detector.flush() {
                events.extend(self.write(start, &utterance)?);
            }
            Ok(events)
        }

        fn write(&mut self, start: usize, samples: &[f32]) -> Result<Vec<Event>, String> {
            let wav = wav(samples);
            let text = match self.provider {
                Provider::Xai => crate::x::transcribe(&self.state, &wav, "speech.wav")?,
                Provider::Openai => {
                    openai(&self.state, &super::choice(&self.state), &wav, "speech.wav")?
                }
                Provider::Local => unreachable!("a local session is not a cloud one"),
            };
            let text = text.trim().to_owned();
            if text.is_empty() {
                return Ok(Vec::new());
            }
            let event = Event::Final {
                utterance: self.utterance,
                text,
                start: seconds(start),
                end: seconds(start + samples.len()),
            };
            self.utterance += 1;
            Ok(vec![event])
        }
    }

    /// 16-bit mono WAV of the samples, at the session rate.
    pub fn wav(samples: &[f32]) -> Vec<u8> {
        let pcm: Vec<i16> = samples
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
        super::wav16(&pcm, RATE)
    }

    /// A recording written down by an OpenAI-compatible API.
    pub fn openai(
        state: &Path,
        choice: &Choice,
        audio: &[u8],
        name: &str,
    ) -> Result<String, String> {
        let key = std::fs::read_to_string(state.join(super::OPENAI_KEY)).map_err(|_| {
            "no key for the transcription API; add one in Settings, Voice".to_owned()
        })?;
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| format!("stt: {e}"))?;
        let boundary = format!("libri{:032x}", u128::from_ne_bytes(bytes));
        let mut form = Vec::new();
        form.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{}\r\n",
                choice.openai_model
            )
            .as_bytes(),
        );
        form.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        form.extend_from_slice(audio);
        form.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let url = format!(
            "{}/audio/transcriptions",
            choice.openai_url.trim_end_matches('/')
        );
        let mut resp = ureq::post(&url)
            .header("Authorization", &format!("Bearer {}", key.trim()))
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .config()
            .http_status_as_error(false)
            .build()
            .send(&form[..])
            .map_err(|e| format!("stt api: {e}"))?;
        let status = resp.status().as_u16();
        let reply: serde_json::Value = resp
            .body_mut()
            .read_json()
            .map_err(|e| format!("stt api: {status}: {e}"))?;
        match reply["text"].as_str() {
            Some(text) if status < 400 => Ok(text.to_owned()),
            _ => Err(format!(
                "stt api: {status}: {}",
                reply["error"]["message"].as_str().unwrap_or("no text")
            )),
        }
    }
}

/// 16-bit mono WAV of `pcm` at `rate`.
pub fn wav16(pcm: &[i16], rate: u32) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Fetches an archive, checks its SHA-256, and unpacks it into `into`;
/// `progress` hears bytes so far and the total. Shared with tts.rs.
pub fn fetch_archive(
    url: &str,
    sha256: &str,
    archive: &Path,
    into: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    fetch::file(url, sha256, archive, progress)?;
    let unpacked = std::process::Command::new("tar")
        .arg("xjf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()
        .map_err(|e| format!("tar: {e}"));
    let _ = std::fs::remove_file(archive);
    if !unpacked?.success() {
        return Err("the archive did not unpack".into());
    }
    Ok(())
}

mod fetch {
    use super::{Model, Progress, RELEASES};
    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::Mutex;

    /// Fetches a model's archive, checks it, and unpacks it in place.
    pub fn model(state: &Path, m: &Model, progress: &Mutex<Progress>) -> Result<(), String> {
        let dir = super::models_dir(state);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        super::fetch_archive(
            &format!("{RELEASES}/{}.tar.bz2", m.archive),
            m.sha256,
            &dir.join(format!("{}.tar.bz2", m.archive)),
            &dir,
            &mut |got, _| {
                if let Some(entry) = progress.lock().unwrap().get_mut(m.id) {
                    entry.0 = got;
                }
            },
        )?;
        if !super::installed(state, m) {
            return Err("the archive held no model".into());
        }
        // The samples that ship beside it are not needed.
        let _ = std::fs::remove_dir_all(super::model_dir(state, m).join("test_wavs"));
        Ok(())
    }

    /// Fetches `url` to `to` by way of a partial file, checking its SHA-256.
    pub fn file(
        url: &str,
        sha256: &str,
        to: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), String> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let partial = to.with_extension("partial");
        let mut resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
        let total = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = resp.body_mut().as_reader();
        let mut out = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
        let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
        let mut buf = vec![0u8; 1 << 20];
        let mut got = 0u64;
        loop {
            let n = body.read(&mut buf).map_err(|e| format!("{url}: {e}"))?;
            if n == 0 {
                break;
            }
            digest.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            got += n as u64;
            progress(got, total);
        }
        out.sync_all().map_err(|e| e.to_string())?;
        let hex: String = digest
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if hex != sha256 {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("{url}: checksum {hex} is not the one recorded"));
        }
        std::fs::rename(&partial, to).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_is_the_default() {
        let dir = std::env::temp_dir().join(format!("stt-choice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = choice(&dir);
        assert_eq!(c.provider, Provider::Local);
        assert_eq!(c.model, DEFAULT_MODEL);
        assert!(model(DEFAULT_MODEL).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_choice_is_kept_and_an_unknown_model_refused() {
        let dir = std::env::temp_dir().join(format!("stt-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Choice::default();
        c.provider = Provider::Xai;
        choose(&dir, &c).unwrap();
        assert_eq!(choice(&dir).provider, Provider::Xai);
        c.model = "nonsense".into();
        assert!(choose(&dir, &c).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn words_join_finals_only() {
        let events = vec![
            Event::SpeechStarted { utterance: 0 },
            Event::Partial {
                utterance: 0,
                text: "hel".into(),
            },
            Event::Final {
                utterance: 0,
                text: "Hello.".into(),
                start: 0.0,
                end: 1.0,
            },
            Event::Final {
                utterance: 1,
                text: " World.".into(),
                start: 1.0,
                end: 2.0,
            },
        ];
        assert_eq!(words(&events), "Hello. World.");
    }

    #[test]
    fn a_wav_has_its_header_and_samples() {
        let w = cloud::wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 6);
    }
}
