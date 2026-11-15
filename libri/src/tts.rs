//! Text to speech on the host: Kokoro-82M through the same sherpa-onnx as
//! stt.rs, for calls and reading aloud without sending words anywhere.
//!
//! The full-precision model, not the int8 one: on a Threadripper PRO 9965WX the int8
//! build ran slower than real time (RTF 1.16) while the fp32 one writes a
//! 3.2 s sentence in 0.4 s (RTF 0.13), whatever the thread count.
//!
//! `Service::speak(text, voice, speed)` gives 16-bit mono PCM at the
//! model's rate (24 kHz), one sentence at a time; the model loads on first
//! use and goes after ten minutes idle.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Kokoro v1.0's voices in the model's speaker order: the id is the
/// speaker number. The first letter is the accent (a American, b British,
/// e Spanish, f French, h Hindi, i Italian, j Japanese, p Portuguese,
/// z Mandarin), the second the voice (f female, m male).
pub const VOICES: &[&str] = &[
    "af_alloy",
    "af_aoede",
    "af_bella",
    "af_heart",
    "af_jessica",
    "af_kore",
    "af_nicole",
    "af_nova",
    "af_river",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_echo",
    "am_eric",
    "am_fenrir",
    "am_liam",
    "am_michael",
    "am_onyx",
    "am_puck",
    "am_santa",
    "bf_alice",
    "bf_emma",
    "bf_isabella",
    "bf_lily",
    "bm_daniel",
    "bm_fable",
    "bm_george",
    "bm_lewis",
    "ef_dora",
    "em_alex",
    "ff_siwis",
    "hf_alpha",
    "hf_beta",
    "hm_omega",
    "hm_psi",
    "if_sara",
    "im_nicola",
    "jf_alpha",
    "jf_gongitsune",
    "jf_nezumi",
    "jf_tebukuro",
    "jm_kumo",
    "pf_dora",
    "pm_alex",
    "pm_santa",
    "zf_xiaobei",
    "zf_xiaoni",
    "zf_xiaoxiao",
    "zf_xiaoyi",
    "zm_yunjian",
    "zm_yunxi",
    "zm_yunxia",
    "zm_yunyang",
];

/// Kokoro's best-rated voice, the default.
pub const DEFAULT_VOICE: &str = "af_heart";

const ARCHIVE: &str = "kokoro-multi-lang-v1_0";
const SHA256: &str = "60703707bc05ccd48d60e1eb723f38b07d6e474795a0b6a588d7220400e0f7a4";
const RELEASES: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models";

#[derive(Debug, Serialize)]
pub struct Voice {
    pub id: &'static str,
    /// The voice's name, as Kokoro gives it ("heart").
    pub name: String,
    /// Its accent, as a BCP 47 tag ("en-US").
    pub lang: &'static str,
    pub female: bool,
}

pub fn voices() -> Vec<Voice> {
    VOICES
        .iter()
        .map(|&id| {
            let lang = match &id[..1] {
                "a" => "en-US",
                "b" => "en-GB",
                "e" => "es",
                "f" => "fr",
                "h" => "hi",
                "i" => "it",
                "j" => "ja",
                "p" => "pt-BR",
                _ => "zh",
            };
            Voice {
                id,
                name: id[3..].to_owned(),
                lang,
                female: &id[1..2] == "f",
            }
        })
        .collect()
}

fn dir(state: &Path) -> PathBuf {
    state.join("tts").join(ARCHIVE)
}

pub fn installed(state: &Path) -> bool {
    dir(state).join("voices.bin").is_file()
}

/// The host's text to speech, shared by every caller.
pub struct Service {
    state: PathBuf,
    #[cfg(feature = "local-speech")]
    loaded: Mutex<Option<(sherpa_onnx::OfflineTts, std::time::Instant)>>,
    downloading: Arc<Mutex<Option<(u64, u64)>>>,
}

const IDLE: Duration = Duration::from_secs(10 * 60);

impl Service {
    pub fn new(state: PathBuf) -> Arc<Self> {
        let service = Arc::new(Self {
            state,
            #[cfg(feature = "local-speech")]
            loaded: Mutex::new(None),
            downloading: Arc::default(),
        });
        let weak = Arc::downgrade(&service);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(60));
                let Some(service) = weak.upgrade() else {
                    return;
                };
                #[cfg(feature = "local-speech")]
                {
                    let mut loaded = service.loaded.lock().unwrap();
                    if loaded
                        .as_ref()
                        .is_some_and(|(_, used)| used.elapsed() > IDLE)
                    {
                        *loaded = None;
                    }
                }
            }
        });
        service
    }

    /// Bytes so far and total while the model downloads.
    pub fn downloading(&self) -> Option<(u64, u64)> {
        *self.downloading.lock().unwrap()
    }

    /// Starts fetching the model unless it is here or on its way.
    pub fn download(&self) {
        if installed(&self.state) {
            return;
        }
        {
            let mut d = self.downloading.lock().unwrap();
            if d.is_some() {
                return;
            }
            *d = Some((0, 0));
        }
        let state = self.state.clone();
        let progress = self.downloading.clone();
        std::thread::spawn(move || {
            let parent = state.join("tts");
            let archive = parent.join(format!("{ARCHIVE}.tar.bz2"));
            let got = crate::stt::fetch_archive(
                &format!("{RELEASES}/{ARCHIVE}.tar.bz2"),
                SHA256,
                &archive,
                &parent,
                &mut |got, total| *progress.lock().unwrap() = Some((got, total)),
            );
            *progress.lock().unwrap() = None;
            if let Err(e) = got {
                eprintln!("libri: speech voice model: {e}");
            }
        });
    }

    /// Loads the model ahead of the first sentence, when it is here: a
    /// call opens with it, so the answer's first words do not wait on it.
    #[cfg(feature = "local-speech")]
    pub fn warm(&self) -> Result<(), String> {
        if !installed(&self.state) {
            return Ok(());
        }
        let mut loaded = self.loaded.lock().unwrap();
        if loaded.is_none() {
            *loaded = Some((self.load()?, std::time::Instant::now()));
        }
        Ok(())
    }

    #[cfg(not(feature = "local-speech"))]
    pub fn warm(&self) -> Result<(), String> {
        Ok(())
    }

    /// One sentence spoken: 16-bit mono PCM and its sample rate. `speed`
    /// 1.0 is Kokoro's own pace.
    #[cfg(feature = "local-speech")]
    pub fn speak(&self, text: &str, voice: &str, speed: f32) -> Result<(Vec<i16>, u32), String> {
        let sid = VOICES
            .iter()
            .position(|&v| v == voice)
            .ok_or_else(|| format!("no voice {voice}"))? as i32;
        if !installed(&self.state) {
            self.download();
            return Err("the speaking voice model is still downloading".into());
        }
        let mut loaded = self.loaded.lock().unwrap();
        if loaded.is_none() {
            *loaded = Some((self.load()?, std::time::Instant::now()));
        }
        let (tts, used) = loaded.as_mut().unwrap();
        *used = std::time::Instant::now();
        let config = sherpa_onnx::GenerationConfig {
            sid,
            speed: speed.clamp(0.5, 2.0),
            ..Default::default()
        };
        let audio = tts
            .generate_with_config(text, &config, None::<fn(&[f32], f32) -> bool>)
            .ok_or("the voice model gave no audio")?;
        let pcm = audio
            .samples()
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
        Ok((pcm, audio.sample_rate() as u32))
    }

    #[cfg(not(feature = "local-speech"))]
    pub fn speak(&self, _: &str, _: &str, _: f32) -> Result<(Vec<i16>, u32), String> {
        Err("this build of Libri has no local speech engine".into())
    }

    #[cfg(feature = "local-speech")]
    fn load(&self) -> Result<sherpa_onnx::OfflineTts, String> {
        let d = dir(&self.state);
        let file = |name: &str| Some(d.join(name).display().to_string());
        let mut config = sherpa_onnx::OfflineTtsConfig::default();
        let kokoro = &mut config.model.kokoro;
        kokoro.model = file("model.onnx");
        kokoro.voices = file("voices.bin");
        kokoro.tokens = file("tokens.txt");
        kokoro.data_dir = file("espeak-ng-data");
        kokoro.dict_dir = file("dict");
        kokoro.lexicon = Some(format!(
            "{},{}",
            d.join("lexicon-us-en.txt").display(),
            d.join("lexicon-zh.txt").display()
        ));
        config.model.num_threads = 4;
        sherpa_onnx::OfflineTts::create(&config)
            .ok_or_else(|| "the voice model would not load".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_voice_is_listed_and_voices_have_accents() {
        assert!(VOICES.contains(&DEFAULT_VOICE));
        assert_eq!(VOICES.len(), 53);
        let v = voices();
        let heart = v.iter().find(|v| v.id == "af_heart").unwrap();
        assert_eq!(heart.lang, "en-US");
        assert!(heart.female);
        assert_eq!(heart.name, "heart");
    }
}
