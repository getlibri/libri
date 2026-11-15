//! wiki: a Markdown vault an agent keeps as its notes, served over MCP.
//!
//! `wiki serve DIR [--read-only]` speaks MCP on stdio (newline-delimited
//! JSON-RPC, as the stdio transport has it) with four tools: read, write,
//! list and search. The vault is a plain folder Obsidian opens as it is, kept
//! by Obsidian's conventions (vault.rs); this never touches `.obsidian/`.
//! `--read-only` serves read, list and search alone, for an agent that may
//! consult the notes but not change them.
//!
//! The protocol is written out here rather than taken from an MCP library:
//! a stdio server of four tools needs initialize, ping, tools/list and
//! tools/call, and nothing of an async runtime.

mod vault;

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use vault::Vault;

/// The protocol version answered when a client names none.
const PROTOCOL: &str = "2025-06-18";

const ABOUT: &str = "The wiki is your long-term notebook: an Obsidian vault \
of Markdown notes, one per person, project, place or topic, that you keep up as \
you work and that lasts across chats, beside the chat's memory. The person \
opens and edits it in Obsidian too.";

/// How a note is written, for the tool that writes one.
const FORM: &str = "Write it as Obsidian does: link notes by name, [[Note]], \
[[Note#Heading]] or [[Note|shown text]], never by file path; properties as \
YAML frontmatter between --- lines (tags, aliases, created, sources: the \
memory's message ids a fact rests on); #tags inline as well. A note's name is \
unique across the vault, as links find notes by name. Attachments live in \
attachments/ and embed as ![[file.png]].";

struct Server {
    vault: Vault,
    read_only: bool,
}

impl Server {
    fn tools(&self) -> Vec<Value> {
        let name = json!({"type": "string", "description": "The note, by name as a link names it (Ada), or with its folder (People/Ada)."});
        let mut all = vec![
            json!({
                "name": "read",
                "description": format!("{ABOUT} Read one note whole, or one section of it as Ada#Heading."),
                "inputSchema": {"type": "object", "properties": {"note": name}, "required": ["note"]},
                "annotations": {"readOnlyHint": true},
            }),
            json!({
                "name": "list",
                "description": format!("{ABOUT} List the notes by folder and name, with each one's size and the day it last changed."),
                "inputSchema": {"type": "object", "properties": {
                    "folder": {"type": "string", "description": "Only the notes under this folder."}
                }},
                "annotations": {"readOnlyHint": true},
            }),
            json!({
                "name": "search",
                "description": format!("{ABOUT} Search every note and note name by case-insensitive regex; hits show note, line number and the line."),
                "inputSchema": {"type": "object", "properties": {
                    "regex": {"type": "string"}
                }, "required": ["regex"]},
                "annotations": {"readOnlyHint": true},
            }),
        ];
        if !self.read_only {
            all.push(json!({
                "name": "write",
                "description": format!("{ABOUT} Write one note whole, making it or replacing what it held; empty text removes it. A new note goes in the folder named, else the vault's top; an existing one stays where it is. Read a note before rewriting it, to keep what it already says. {FORM}"),
                "inputSchema": {"type": "object", "properties": {
                    "note": name,
                    "text": {"type": "string", "description": "The whole note, in Markdown."}
                }, "required": ["note", "text"]},
                "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true},
            }));
        }
        all
    }

    fn call(&self, tool: &str, args: &Value) -> Result<String, String> {
        let arg = |k: &str| args[k].as_str().ok_or_else(|| format!("{tool} needs {k}."));
        match tool {
            "read" => self.vault.read(arg("note")?),
            "list" => self.vault.list(args["folder"].as_str()),
            "search" => self.vault.search(arg("regex")?),
            "write" if self.read_only => {
                Err("The wiki is read-only here: you may read it, not change it.".into())
            }
            "write" => self.vault.write(arg("note")?, arg("text")?),
            _ => Err(format!("No tool {tool}.")),
        }
    }

    /// The answer to one message; None for a notification, which gets none.
    fn answer(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id")?.clone();
        let method = msg["method"].as_str().unwrap_or_default();
        let params = &msg["params"];
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "wiki", "version": env!("CARGO_PKG_VERSION")},
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": self.tools()})),
            "tools/call" => {
                let tool = params["name"].as_str().unwrap_or_default();
                let (text, failed) = match self.call(tool, &params["arguments"]) {
                    Ok(t) => (t, false),
                    Err(e) => (e, true),
                };
                Ok(json!({"content": [{"type": "text", "text": text}], "isError": failed}))
            }
            _ => Err(json!({"code": -32601, "message": format!("No method {method}.")})),
        };
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
        })
    }
}

fn serve(server: Server) -> Result<(), String> {
    let stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => server.answer(&msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

const USAGE: &str = "Usage: wiki serve DIR [--read-only]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, read_only) = match args.as_slice() {
        [cmd, dir] if cmd == "serve" => (dir, false),
        [cmd, dir, flag] if cmd == "serve" && flag == "--read-only" => (dir, true),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    let server = Server {
        vault: Vault::new(PathBuf::from(dir)),
        read_only,
    };
    if let Err(e) = serve(server) {
        eprintln!("wiki: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, read_only: bool) -> Server {
        let dir = std::env::temp_dir().join(format!("wiki-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Server {
            vault: Vault::new(dir),
            read_only,
        }
    }

    fn ask(s: &Server, method: &str, params: Value) -> Value {
        s.answer(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .unwrap()
    }

    fn call(s: &Server, tool: &str, args: Value) -> (String, bool) {
        let r = &ask(s, "tools/call", json!({"name": tool, "arguments": args}))["result"];
        (
            r["content"][0]["text"].as_str().unwrap().to_owned(),
            r["isError"].as_bool().unwrap(),
        )
    }

    fn names(s: &Server) -> Vec<String> {
        ask(s, "tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn it_answers_the_client_in_its_version_and_ignores_notifications() {
        let s = server("init", false);
        let r = ask(&s, "initialize", json!({"protocolVersion": "2025-03-26"}));
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "wiki");
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert_eq!(s.answer(&note), None);
        assert_eq!(ask(&s, "nope", json!({}))["error"]["code"], -32601);
    }

    #[test]
    fn the_tools_write_then_read_list_and_search() {
        let s = server("tools", false);
        assert_eq!(names(&s), ["read", "list", "search", "write"]);
        assert_eq!(
            call(&s, "write", json!({"note": "Ada", "text": "Likes [[Tea]]"})),
            ("Made Ada.".into(), false)
        );
        assert_eq!(
            call(&s, "read", json!({"note": "Ada"})).0,
            "Likes [[Tea]]\n"
        );
        assert!(call(&s, "list", json!({})).0.starts_with("Ada  "));
        assert_eq!(
            call(&s, "search", json!({"regex": "tea"})).0,
            "Ada:1: Likes [[Tea]]\n"
        );
        assert_eq!(
            call(&s, "read", json!({})),
            ("read needs note.".into(), true)
        );
    }

    #[test]
    fn read_only_offers_no_write_and_refuses_one() {
        let s = server("ro", true);
        assert_eq!(names(&s), ["read", "list", "search"]);
        let (text, failed) = call(&s, "write", json!({"note": "Ada", "text": "x"}));
        assert!(failed && text.contains("read-only"), "{text}");
        assert_eq!(
            call(&s, "read", json!({"note": "Ada"})),
            ("No note Ada.".into(), true)
        );
    }
}
