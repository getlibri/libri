//! JSON-RPC over a child's stdin and stdout, one message per line: the
//! wire of ACP and of Codex's app-server. Requests come both ways, so
//! what the child sends arrives on a channel the caller reads at its own
//! pace, while it also watches for a stop.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// What the child sent.
#[derive(Debug)]
pub enum Incoming {
    /// The answer to our request `id`: its result, or its error.
    Response {
        id: u64,
        result: Result<Value, Value>,
    },
    /// The child asks us something, to be answered with `respond`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

/// Our side of the conversation with one child.
pub struct Peer {
    stdin: Mutex<ChildStdin>,
    next: AtomicU64,
    /// ACP is JSON-RPC 2.0 to the letter; Codex leaves out the version.
    versioned: bool,
}

/// A child process and our peer to it, its messages on `incoming`.
pub struct Session {
    pub child: Child,
    pub peer: Arc<Peer>,
    pub incoming: Receiver<Incoming>,
}

impl Session {
    /// Starts `cmd` in a process group of its own, so a stop ends all it
    /// started too.
    pub fn spawn(mut cmd: Command, versioned: bool) -> Result<Self, String> {
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        crate::cgroup::harness(&mut cmd);
        let name = format!("{:?}", cmd.get_program());
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("{name}: {e}"))?;
        let peer = Arc::new(Peer {
            stdin: Mutex::new(child.stdin.take().unwrap()),
            next: AtomicU64::new(0),
            versioned,
        });
        let incoming = read(child.stdout.take().unwrap());
        Ok(Self {
            child,
            peer,
            incoming,
        })
    }

    /// Sends a request and waits up to `wait` for its answer, putting
    /// whatever else comes meanwhile in `aside`.
    pub fn call(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
        aside: &mut Vec<Incoming>,
    ) -> Result<Value, String> {
        let id = self.peer.request(method, params)?;
        let end = Instant::now() + wait;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.incoming.recv_timeout(left) {
                Ok(Incoming::Response { id: got, result }) if got == id => {
                    return result.map_err(|e| format!("{method}: {e}"));
                }
                Ok(other) => aside.push(other),
                Err(RecvTimeoutError::Timeout) => return Err(format!("{method}: no answer")),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("{method}: the harness exited"));
                }
            }
        }
    }

    /// Ends the child and all it started: asked first, then made to.
    pub fn end(mut self) {
        let group = self.child.id() as i32;
        // SAFETY: killpg only sends a signal, to the group this child leads.
        unsafe { libc::killpg(group, libc::SIGTERM) };
        let given = Instant::now() + Duration::from_secs(3);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < given {
            thread::sleep(Duration::from_millis(50));
        }
        // SAFETY: as above.
        unsafe { libc::killpg(group, libc::SIGKILL) };
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Peer {
    fn write(&self, mut message: Value) -> Result<(), String> {
        if self.versioned {
            message["jsonrpc"] = "2.0".into();
        }
        let mut stdin = self.stdin.lock().unwrap();
        writeln!(stdin, "{message}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("the harness's input: {e}"))
    }

    pub fn request(&self, method: &str, params: Value) -> Result<u64, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.write(json!({"id": id, "method": method, "params": params}))?;
        Ok(id)
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(json!({"method": method, "params": params}))
    }

    pub fn respond(&self, id: &Value, result: Value) -> Result<(), String> {
        self.write(json!({"id": id, "result": result}))
    }

    /// Refuses a request we don't serve.
    pub fn refuse(&self, id: &Value, method: &str) -> Result<(), String> {
        self.write(json!({"id": id, "error": {"code": -32601, "message": format!("{method} is not offered")}}))
    }
}

fn read(stdout: ChildStdout) -> Receiver<Incoming> {
    let (send, incoming) = channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(m) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if send.send(parse(m)).is_err() {
                break;
            }
        }
    });
    incoming
}

fn parse(mut m: Value) -> Incoming {
    let method = m["method"].as_str().map(str::to_owned);
    let params = m["params"].take();
    match (method, m.get("id").cloned()) {
        (Some(method), Some(id)) => Incoming::Request { id, method, params },
        (Some(method), None) => Incoming::Notification { method, params },
        (None, id) => Incoming::Response {
            id: id.and_then(|i| i.as_u64()).unwrap_or(u64::MAX),
            result: match m.get_mut("error") {
                Some(e) => Err(e.take()),
                None => Ok(m["result"].take()),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_told_apart_by_method_and_id() {
        let r = parse(json!({"jsonrpc": "2.0", "id": 3, "result": {"stopReason": "end_turn"}}));
        assert!(matches!(
            r,
            Incoming::Response {
                id: 3,
                result: Ok(_)
            }
        ));
        let e = parse(json!({"id": 4, "error": {"message": "no"}}));
        assert!(matches!(
            e,
            Incoming::Response {
                id: 4,
                result: Err(_)
            }
        ));
        let q = parse(json!({"id": 0, "method": "session/request_permission", "params": {}}));
        assert!(matches!(q, Incoming::Request { .. }));
        let n = parse(json!({"method": "session/update", "params": {"x": 1}}));
        assert!(matches!(n, Incoming::Notification { .. }));
    }
}
