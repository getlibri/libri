//! Pi through its own RPC mode, `pi --mode rpc`: the protocol omp forked
//! (`omp.rs` reads it), not the third-party pi-acp, whose ACP drops Pi's
//! thinking, partial output and usage. A chat that keeps a session goes on
//! with its own, as on omp (`omp::session_args`); any other call is one
//! ephemeral session (`--no-session`).
//!
//! Pi has no approvals: it runs what its tools allow without asking
//! (docs/security.md). Its modes here are therefore tool sets, as Pi's own
//! `--tools` gives them. Libri's MCP servers reach it through the project
//! config it reads from its working directory (`.pi/mcp.json`), trusted
//! with `--approve`; each turn's scope goes in Pi's environment, which its
//! servers inherit, as for omp.

use crate::model::{Attachment, Harness, Run};
use crate::omp::{Dialect, Process, turn, write_if_changed};
use crate::servers::{self, Scope};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The built-in tools of the mode that only reads.
pub const READING: &str = "read,grep,find,ls";

pub struct Pi {
    pub command: String,
    /// Where Pi works, and where its project config goes.
    pub cwd: PathBuf,
    /// The thinking level (LIBRI_EFFORT), unless the person picked one.
    pub effort: String,
}

impl Pi {
    pub fn from_env() -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let cwd = var("LIBRI_WORKDIR")
            .or_else(|| var("STATE_DIRECTORY"))
            .map(PathBuf::from)
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            command: var("LIBRI_PI").unwrap_or_else(|| "pi".into()),
            cwd,
            effort: var("LIBRI_EFFORT").unwrap_or_else(|| "high".into()),
        })
    }

    /// Writes Libri's MCP servers where Pi reads a project's, in `dir`.
    fn configure(dir: &Path, scope: Scope, worker: Option<&str>) -> Result<(), String> {
        let project = dir.join(".pi");
        std::fs::create_dir_all(&project).map_err(|e| format!("{}: {e}", project.display()))?;
        let tools = servers::pi(&servers::list(scope, worker)?);
        write_if_changed(&project.join("mcp.json"), &tools)
    }

    /// The models Pi reaches with the credentials it holds, as its RPC
    /// `get_available_models` gives them.
    pub fn models(&self) -> Result<Vec<Value>, String> {
        let mut cmd = Command::new(&self.command);
        cmd.args([
            "--mode",
            "rpc",
            "--no-session",
            "--no-mcp",
            "--no-extensions",
        ])
        .arg("--offline")
        .current_dir(crate::model::bare_dir()?);
        let mut p = Process::spawn(cmd, Dialect::Pi)?;
        let listed = p.request(
            "models",
            json!({"type": "get_available_models"}),
            Duration::from_secs(30),
        );
        p.end();
        Ok(listed?["models"].as_array().cloned().unwrap_or_default())
    }
}

impl Harness for Pi {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String> {
        let scope = Scope::of(&run);
        let dir = &self.cwd;
        // One file serves every turn in a directory, so it holds the
        // person's list and each turn's scope holds it to what it may reach.
        if scope.is_some() {
            Self::configure(dir, Scope::Person, None)?;
        }
        let effort = run.effort().unwrap_or_else(|| self.effort.clone());
        // Each value its own word: Pi refuses `--flag=value`.
        let mut cmd = Command::new(&self.command);
        cmd.args(["--mode", "rpc"]);
        let holds_thread = crate::omp::session_args(&mut cmd, &run, "pi")?;
        let memory = if holds_thread { "" } else { memory };
        cmd.arg("--approve")
            .args(["--thinking", &effort])
            .current_dir(dir);
        run.temp(&mut cmd);
        if scope == Some(Scope::Memoryless) {
            // The web alone: Libri's tools, which the scope holds to the
            // web, and none of Pi's own, which read the person's files.
            cmd.args(["--tools", "mcp__libri__*"]);
        } else if run.plain {
            cmd.args([
                "--no-tools",
                "--no-mcp",
                "--no-skills",
                "--no-context-files",
            ]);
        } else if run.access("pi").mode == "read" {
            cmd.args(["--tools", READING]);
        }
        if run.plain {
            cmd.args(["--system-prompt", system]);
        } else {
            // Beside Pi's own instructions, not in their place.
            cmd.args(["--append-system-prompt", system]);
        }
        if let Some(m) = run.model_for("pi") {
            cmd.args(["--model", &m]);
        }
        if let Some(scope) = scope {
            cmd.env(Scope::VAR, scope.name());
            if let Some(worker) = &run.sub {
                cmd.env(Scope::WORKER, worker);
            }
            // Pi's tool server inherits Pi's environment.
            cmd.envs(crate::servers::memory_env(&run));
        }
        let mut p = Process::spawn(cmd, Dialect::Pi)?;
        let turn = turn(&p, memory, prompt, files, &run, &[]);
        if let Some(s) = &run.steer {
            s.close();
        }
        p.end();
        turn
    }

    fn tools(&self) -> bool {
        true
    }
}
