//! Signing the person in to each harness with its own sign-in, as its own
//! apps do, so every harness runs on the person's account and nothing is
//! copied from elsewhere: Codex by ChatGPT's device code (its app-server's
//! `account/login/start`), and omp by its own device code for ChatGPT (its
//! `login` command). Libri keeps no keys: OpenCode and omp read theirs
//! from their own config, and Claude Code is signed in by the service that
//! runs Libri.

use crate::rpc::{Incoming, Session};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long a device code stays good, as ChatGPT gives them.
const DEVICE: Duration = Duration::from_secs(15 * 60);

/// Whether `harness` is signed in, by its own credentials: Codex by its
/// ChatGPT sign-in, OpenCode and omp by reaching a model with the keys
/// their own config names.
pub fn signed(harness: &str) -> bool {
    match harness {
        "codex" => std::env::var_os("HOME")
            .is_some_and(|h| Path::new(&h).join(".codex/auth.json").exists()),
        h => crate::catalog::reaches(h),
    }
}

/// How the person signs `harness` in, said where it shows signed out.
pub fn remedy(harness: &str) -> &'static str {
    match harness {
        "codex" => "type /signin codex for a ChatGPT code",
        "opencode" => "run `opencode auth login` in a terminal on this host",
        "omp" => "type /signin omp for a ChatGPT code, or run `omp login` on this host",
        "pi" => "run `pi` in a terminal on this host and type /login",
        "muse" => "run `muse` in a terminal on this host and sign in",
        "claude" => "run `claude` in a terminal on this host and sign in",
        _ => "sign it in through its own config on this host",
    }
}

struct DeviceCode {
    url: String,
    code: String,
    provider: Option<String>,
}

enum Progress {
    Waiting(DeviceCode),
    Done(Option<DeviceCode>),
    Failed {
        device: Option<DeviceCode>,
        error: String,
    },
}

impl Progress {
    fn value(&self) -> Value {
        let (state, error, device) = match self {
            Self::Waiting(device) => ("waiting", None, Some(device)),
            Self::Done(device) => ("done", None, device.as_ref()),
            Self::Failed { device, error } => ("failed", Some(error), device.as_ref()),
        };
        let mut value = json!({"state": state, "error": error});
        if let Some(device) = device {
            value["url"] = json!(device.url);
            value["code"] = json!(device.code);
            value["provider"] = json!(device.provider);
        }
        value
    }
}

type Signins = HashMap<String, Progress>;
static SIGNING: LazyLock<Mutex<Signins>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn finished(all: &mut Signins, harness: &str, error: Option<String>) {
    let device = all.remove(harness).and_then(|p| match p {
        Progress::Waiting(device) => Some(device),
        Progress::Done(device) | Progress::Failed { device, .. } => device,
    });
    let progress = match error {
        None => Progress::Done(device),
        Some(error) => Progress::Failed { device, error },
    };
    all.insert(harness.to_owned(), progress);
}

/// How each device sign-in started stands, in the existing HTTP shape.
pub fn progress() -> Value {
    Value::Object(
        SIGNING
            .lock()
            .unwrap()
            .iter()
            .map(|(h, p)| (h.clone(), p.value()))
            .collect(),
    )
}

/// Starts a device sign-in to ChatGPT for `harness` (Codex or omp): the
/// page to open and the code to type there, kept with how it stands.
pub fn device(harness: &str, provider: Option<&str>) -> Result<(String, String), String> {
    let h = harness.to_owned();
    let ended = move |how: Option<String>| {
        eprintln!(
            "libri: {h} sign-in: {}",
            how.as_deref()
                .map_or("done".into(), |why| format!("failed: {why}"))
        );
        finished(&mut SIGNING.lock().unwrap(), &h, how);
    };
    let (url, code) = match harness {
        "codex" => codex_device(ended)?,
        "omp" => crate::omp::login(provider.unwrap_or("openai-codex-device"), ended)?,
        _ => return Err(format!("{harness} signs in through its own config")),
    };
    SIGNING.lock().unwrap().insert(
        harness.to_owned(),
        Progress::Waiting(DeviceCode {
            url: url.clone(),
            code: code.clone(),
            provider: provider.map(str::to_owned),
        }),
    );
    Ok((url, code))
}

/// Starts a ChatGPT device sign-in for Codex: the page to open and the
/// code to type there. A thread waits for it to be used and tells `ended`
/// how it went: `None` signed in, else why not.
fn codex_device(
    ended: impl FnOnce(Option<String>) + Send + 'static,
) -> Result<(String, String), String> {
    let codex = std::env::var("LIBRI_CODEX").unwrap_or_else(|_| "codex".into());
    let mut cmd = Command::new(codex);
    cmd.arg("app-server");
    let s = Session::spawn(cmd, false)?;
    let wait = Duration::from_secs(60);
    let mut aside = Vec::new();
    let started = (|| {
        s.call(
            "initialize",
            json!({"clientInfo": {"name": "libri", "version": env!("CARGO_PKG_VERSION")}}),
            wait,
            &mut aside,
        )?;
        s.peer.notify("initialized", json!({}))?;
        s.call(
            "account/login/start",
            json!({"type": "chatgptDeviceCode"}),
            wait,
            &mut aside,
        )
    })();
    let r = match started {
        Ok(r) => r,
        Err(e) => {
            s.end();
            return Err(e);
        }
    };
    let (Some(url), Some(code)) = (r["verificationUrl"].as_str(), r["userCode"].as_str()) else {
        s.end();
        return Err("codex gave no device code".into());
    };
    let found = (url.to_owned(), code.to_owned());
    thread::spawn(move || {
        let end = Instant::now() + DEVICE;
        let how = loop {
            if Instant::now() > end {
                break Some("The code ran out.".to_owned());
            }
            match s.incoming.recv_timeout(Duration::from_secs(1)) {
                Ok(Incoming::Notification { method, params })
                    if method == "account/login/completed" =>
                {
                    break (params["success"] != true).then(|| {
                        params["error"]
                            .as_str()
                            .unwrap_or("ChatGPT refused the sign-in")
                            .to_owned()
                    });
                }
                Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break Some("codex exited".into()),
            }
        };
        s.end();
        ended(how);
    });
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_preserves_device_fields_and_wire_shape() {
        let mut all = Signins::new();
        all.insert(
            "omp".into(),
            Progress::Waiting(DeviceCode {
                url: "https://login.example.com".into(),
                code: "ABCD".into(),
                provider: None,
            }),
        );
        let waiting = all["omp"].value();
        assert_eq!(
            waiting,
            json!({"state":"waiting", "error":null, "url":"https://login.example.com", "code":"ABCD", "provider":null})
        );
        finished(&mut all, "omp", Some("Code expired".into()));
        assert_eq!(
            all["omp"].value(),
            json!({"state":"failed", "error":"Code expired", "url":"https://login.example.com", "code":"ABCD", "provider":null})
        );
        finished(&mut all, "omp", None);
        let mut done = waiting;
        done["state"] = json!("done");
        assert_eq!(all["omp"].value(), done);
    }

    #[test]
    fn completion_before_device_code_has_no_invented_fields() {
        let mut all = Signins::new();
        finished(&mut all, "codex", None);
        assert_eq!(all["codex"].value(), json!({"state":"done", "error":null}));
    }
}
