//! `libri service install`: Libri as a systemd user service, for a machine
//! with no system package. Linger keeps it up with nobody logged in, as a
//! server is. A user unit starts with systemd's short PATH, so it keeps the
//! installer's: harnesses live where their own installers put them, and
//! that shell found them. `--instance NAME` makes it `libri-NAME.service`,
//! beside the default one, and the flags given go into its command line.

use crate::host;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// `libri.service`, or `libri-NAME.service` for a named instance.
fn unit_name() -> String {
    format!("{}.service", host::instance_dir())
}

fn unit_path() -> Result<PathBuf, String> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or("no home directory")?;
    Ok(config.join("systemd/user").join(unit_name()))
}

/// The unit's text, for the binary at `exe` with the installer's PATH.
fn unit(exe: &str, args: &[String], path: &str) -> String {
    let description = match std::env::var("LIBRI_INSTANCE") {
        Ok(name) if !name.is_empty() => format!("Libri ({name})"),
        _ => "Libri".into(),
    };
    // systemd splits ExecStart on spaces and reads quotes, as a shell does.
    let command = std::iter::once(exe.to_owned())
        .chain(args.iter().map(|a| {
            if a.contains([' ', '"', '\\']) {
                format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                a.clone()
            }
        }))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "[Unit]\n\
         Description={description}\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={command}\n\
         Environment=\"PATH={path}\"\n\
         Restart=on-failure\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// The PATH with `~/.local/bin` on it, which a first login on Debian lacks
/// until the directory exists, and where the harnesses' installers put them.
fn with_home_bin(path: &str, home: &str) -> String {
    let bin = format!("{home}/.local/bin");
    if home.is_empty() || path.split(':').any(|p| p == bin) {
        path.to_string()
    } else {
        format!("{bin}:{path}")
    }
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let done = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if done.success() {
        Ok(())
    } else {
        Err(format!("{program} {}: {done}", args.join(" ")))
    }
}

pub fn install(flags: &host::Flags) -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let path = with_home_bin(
        &env::var("PATH").unwrap_or_default(),
        &env::var("HOME").unwrap_or_default(),
    );
    let file = unit_path()?;
    fs::create_dir_all(file.parent().ok_or("no unit directory")?).map_err(|e| e.to_string())?;
    fs::write(&file, unit(&exe.to_string_lossy(), &flags.args(), &path))
        .map_err(|e| e.to_string())?;
    run("systemctl", &["--user", "daemon-reload"])?;
    let name = unit_name();
    run("systemctl", &["--user", "enable", "--now", &name])?;
    let user = env::var("USER").map_err(|_| "USER is not set")?;
    run("loginctl", &["enable-linger", &user])?;
    println!(
        "Libri runs as your user service {name} ({}), on http://{}.",
        file.display(),
        std::env::var("LIBRI_SITE").unwrap_or_default()
    );
    host::report_harnesses();
    println!("`libri pair` makes a link for a device.");
    Ok(())
}

pub fn remove() -> Result<(), String> {
    let file = unit_path()?;
    run("systemctl", &["--user", "disable", "--now", &unit_name()])?;
    fs::remove_file(&file).map_err(|e| e.to_string())?;
    run("systemctl", &["--user", "daemon-reload"])?;
    println!("Libri's service is gone; its state stays where it was.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_runs_the_binary_on_the_installers_path() {
        let text = unit("/opt/libri/bin/libri", &[], "/home/a/.local/bin:/usr/bin");
        assert!(text.contains("ExecStart=/opt/libri/bin/libri\n"));
        assert!(text.contains("Environment=\"PATH=/home/a/.local/bin:/usr/bin\"\n"));
        let flags = host::Flags {
            instance: Some("work".into()),
            port: Some(6781),
            config: Some("/home/a/my config.toml".into()),
            ..Default::default()
        };
        let named = unit("/opt/libri/bin/libri", &flags.args(), "/usr/bin");
        assert!(named.contains(
            "ExecStart=/opt/libri/bin/libri --config \"/home/a/my config.toml\" --instance work --port 6781\n"
        ));
    }

    #[test]
    fn the_units_path_reaches_the_home_bin_once() {
        assert_eq!(
            with_home_bin("/usr/bin", "/home/a"),
            "/home/a/.local/bin:/usr/bin"
        );
        assert_eq!(
            with_home_bin("/home/a/.local/bin:/usr/bin", "/home/a"),
            "/home/a/.local/bin:/usr/bin"
        );
    }
}
