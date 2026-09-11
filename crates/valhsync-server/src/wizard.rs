//! Writing a start script for the dedicated server.
//!
//! Steam overwrites `start_headless_server.bat` on every update, so admins
//! copy it and edit the copy — by hand, which is where the two rules Valheim
//! enforces on passwords get broken and the server refuses to start. This
//! builds the copy instead, checks those rules first, and keeps the settings
//! that are easy to forget: the autosave interval and how many backups to
//! keep.
//!
//! The password ends up in plain text in the script, exactly as in the file
//! Iron Gate ships. ValhSync never reads it back, never stores it and never
//! publishes it: it is written once, here, and only to the file the admin
//! names.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// The Steam app id of the dedicated server, which the binary expects in its
/// environment.
const APP_ID: &str = "892970";

/// Scripts Steam replaces on every update; writing over one would lose the
/// admin's settings at the next verify.
const STOCK: &[&str] = &[
    "start_headless_server.bat",
    "start_server.sh",
    "start_server_bepinex.sh",
    "start_game_bepinex.sh",
];

/// What the admin filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub name: String,
    pub world: String,
    pub password: String,
    pub port: u16,
    /// Listed in the community browser, as opposed to joined by address.
    pub public: bool,
    pub crossplay: bool,
    /// Seconds between autosaves. Valheim's own default is 1800.
    pub save_interval: u32,
    pub backups: u32,
    /// Add `-logFile`, for a server with no BepInEx to write a log of its own.
    /// Unity then writes to the file *instead of* the console, so the server's
    /// window falls silent: only worth it when there is nothing else to read.
    pub log_file: bool,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            name: String::new(),
            world: String::new(),
            password: String::new(),
            port: 2456,
            public: false,
            crossplay: true,
            save_interval: 1800,
            backups: 4,
            log_file: false,
        }
    }
}

/// Where the generated script writes the server's output, relative to the
/// server folder.
pub const LOG_PATH: &str = "logs/valheim-server.log";

/// A reason the server would refuse to start, or the script would not survive
/// being written. The window turns these into its own two languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Issue {
    NameEmpty,
    WorldEmpty,
    /// Valheim requires at least five characters.
    PasswordTooShort,
    /// Valheim refuses to start when the server name contains the password.
    PasswordInName,
    /// A quote or a control character, which no shell would survive.
    BadCharacters(Field),
    /// Below 1024 the port needs privileges; the server also uses `port + 1`.
    PortOutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    World,
    Password,
}

impl Recipe {
    /// Everything wrong with it, so the window can show all of it at once
    /// rather than one problem per attempt.
    #[must_use]
    pub fn issues(&self) -> Vec<Issue> {
        let mut out = Vec::new();
        let name = self.name.trim();
        let world = self.world.trim();
        if name.is_empty() {
            out.push(Issue::NameEmpty);
        }
        if world.is_empty() {
            out.push(Issue::WorldEmpty);
        }
        if self.password.chars().count() < 5 {
            out.push(Issue::PasswordTooShort);
        } else if name.contains(self.password.as_str()) {
            out.push(Issue::PasswordInName);
        }
        for (field, value) in [
            (Field::Name, name),
            (Field::World, world),
            (Field::Password, self.password.as_str()),
        ] {
            if !is_writable(value) {
                out.push(Issue::BadCharacters(field));
            }
        }
        if self.port < 1024 || self.port > u16::MAX - 2 {
            out.push(Issue::PortOutOfRange);
        }
        out
    }

    /// The script itself, in the flavour of the platform it will run on.
    #[must_use]
    pub fn render(&self, windows: bool) -> String {
        if windows { self.batch() } else { self.shell() }
    }

    fn batch(&self) -> String {
        let name = bat(self.name.trim());
        let world = bat(self.world.trim());
        let password = bat(&self.password);
        let port = self.port;
        let public = u8::from(self.public);
        let mkdir = if self.log_file {
            "if not exist \"logs\" mkdir \"logs\"\n\n"
        } else {
            ""
        };
        let args = self.arguments("^");
        format!(
            "@echo off\n\
             title Valheim - {name}\n\
             setlocal\n\
             \n\
             REM Written by ValhSync. Steam overwrites start_headless_server.bat\n\
             REM on every update; it will never touch this copy.\n\
             REM Stop the server with Ctrl+C in this window: that is what saves\n\
             REM the world.\n\
             \n\
             set \"SERVERNAME={name}\"\n\
             set \"WORLD={world}\"\n\
             set \"PASSWORD={password}\"\n\
             set \"PORT={port}\"\n\
             set \"PUBLIC={public}\"\n\
             \n\
             set SteamAppId={APP_ID}\n\
             \n\
             echo Valheim - %SERVERNAME% - world %WORLD% - port %PORT%\n\
             echo Ctrl+C to stop; it saves the world on the way out.\n\
             echo.\n\
             \n\
             {mkdir}\
             valheim_server.exe -nographics -batchmode ^\n\
             {args}\
             \n\
             echo.\n\
             echo Server stopped.\n\
             pause\n"
        )
    }

    fn shell(&self) -> String {
        let name = sh(self.name.trim());
        let world = sh(self.world.trim());
        let password = sh(&self.password);
        let port = self.port;
        let public = u8::from(self.public);
        let mkdir = if self.log_file {
            "mkdir -p logs\n\n"
        } else {
            ""
        };
        let args = self.arguments("\\");
        format!(
            "#!/bin/sh\n\
             # Written by ValhSync. Stop the server with Ctrl+C: that is what\n\
             # saves the world.\n\
             \n\
             SERVERNAME={name}\n\
             WORLD={world}\n\
             PASSWORD={password}\n\
             PORT={port}\n\
             PUBLIC={public}\n\
             \n\
             export SteamAppId={APP_ID}\n\
             export LD_LIBRARY_PATH=\"./linux64:$LD_LIBRARY_PATH\"\n\
             \n\
             {mkdir}\
             ./valheim_server.x86_64 -nographics -batchmode \\\n\
             {args}"
        )
    }

    /// The argument list, one per line, continued with the platform's
    /// character. Variables are referenced rather than repeated so the admin
    /// can still edit the script by hand afterwards.
    fn arguments(&self, cont: &str) -> String {
        let v = |name: &str| {
            if cont == "^" {
                format!("%{name}%")
            } else {
                format!("${name}")
            }
        };
        let mut args = vec![
            format!(" -name \"{}\"", v("SERVERNAME")),
            format!(" -world \"{}\"", v("WORLD")),
            format!(" -password \"{}\"", v("PASSWORD")),
            format!(" -port {}", v("PORT")),
            format!(" -public {}", v("PUBLIC")),
        ];
        if self.crossplay {
            args.push(" -crossplay".into());
        }
        args.push(format!(" -saveinterval {}", self.save_interval));
        args.push(format!(" -backups {}", self.backups));
        if self.log_file {
            args.push(format!(" -logFile \"{LOG_PATH}\""));
        }
        let last = args.len() - 1;
        args.into_iter()
            .enumerate()
            .map(|(i, a)| {
                if i == last {
                    format!("{a}\n")
                } else {
                    format!("{a} {cont}\n")
                }
            })
            .collect()
    }
}

/// Write the script next to `valheim_server.exe`. Refuses the names Steam
/// owns, and refuses to replace an existing file unless told to.
pub fn write(root: &Path, file_name: &str, recipe: &Recipe, replace: bool) -> Result<PathBuf> {
    let issues = recipe.issues();
    if !issues.is_empty() {
        bail!("the settings are not valid yet");
    }
    let name = file_name.trim();
    if name.is_empty() || Path::new(name).file_name().is_none_or(|n| n != name) {
        bail!("{name:?} is not a file name");
    }
    let windows = name.to_lowercase().ends_with(".bat") || name.to_lowercase().ends_with(".cmd");
    if !windows && !name.to_lowercase().ends_with(".sh") {
        bail!("a start script is a .bat, a .cmd or a .sh");
    }
    if STOCK.iter().any(|s| s.eq_ignore_ascii_case(name)) {
        bail!("{name} belongs to Steam and is overwritten on every update; pick another name");
    }
    let path = root.join(name);
    if path.exists() && !replace {
        bail!("{} already exists", path.display());
    }
    std::fs::write(&path, recipe.render(windows))
        .with_context(|| format!("cannot write {}", path.display()))?;
    #[cfg(unix)]
    if !windows {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(path)
}

/// No quote, no newline, no control character: values that a shell could not
/// carry across, whichever shell it is.
fn is_writable(value: &str) -> bool {
    !value.contains(['"', '\'', '`']) && !value.contains(char::is_control)
}

/// `%` starts a variable in a batch file; doubled, it is a literal one.
fn bat(value: &str) -> String {
    value.replace('%', "%%")
}

/// Single quotes, so nothing inside is expanded. Callers have already refused
/// values holding a quote of their own.
fn sh(value: &str) -> String {
    format!("'{value}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::parse_start_script;

    fn good() -> Recipe {
        Recipe {
            name: "Midgard".into(),
            world: "Midgard".into(),
            password: "longboat".into(),
            ..Recipe::default()
        }
    }

    #[test]
    fn valheims_own_rules_are_checked_first() {
        assert!(good().issues().is_empty());

        let short = Recipe {
            password: "abc".into(),
            ..good()
        };
        assert_eq!(short.issues(), [Issue::PasswordTooShort]);

        // The rule that catches admins out: the server starts, then quits.
        let inside = Recipe {
            name: "longboat crew".into(),
            ..good()
        };
        assert_eq!(inside.issues(), [Issue::PasswordInName]);

        let quoted = Recipe {
            world: "the \"good\" one".into(),
            ..good()
        };
        assert_eq!(quoted.issues(), [Issue::BadCharacters(Field::World)]);

        let low = Recipe { port: 80, ..good() };
        assert_eq!(low.issues(), [Issue::PortOutOfRange]);
    }

    #[test]
    fn what_we_write_is_what_we_read_back() {
        let recipe = Recipe {
            port: 2466,
            public: true,
            save_interval: 900,
            backups: 6,
            log_file: true,
            ..good()
        };
        for windows in [true, false] {
            let args = parse_start_script(&recipe.render(windows));
            assert_eq!(args.name.as_deref(), Some("Midgard"), "windows={windows}");
            assert_eq!(args.world.as_deref(), Some("Midgard"));
            assert_eq!(args.port, Some(2466));
            assert_eq!(args.public, Some(true));
            assert!(args.crossplay);
            assert!(args.has_password);
            assert_eq!(args.save_interval, Some(900));
            assert_eq!(args.backups, Some(6));
            assert_eq!(args.log_file.as_deref(), Some(LOG_PATH));
        }
    }

    #[test]
    fn the_batch_file_has_the_shape_a_batch_file_needs() {
        let script = good().render(true);
        let lines: Vec<&str> = script.lines().collect();
        assert_eq!(lines[0], "@echo off");
        assert!(script.contains("set SteamAppId=892970"), "{script}");
        assert!(
            script.contains("valheim_server.exe -nographics -batchmode ^"),
            "{script}"
        );
        // Every continued line ends with the caret, and the last one does not.
        let args: Vec<&str> = lines
            .iter()
            .skip_while(|l| !l.starts_with("valheim_server.exe"))
            .skip(1)
            .take_while(|l| l.starts_with(' '))
            .copied()
            .collect();
        assert_eq!(args.len(), 8, "{args:?}");
        assert!(args[..7].iter().all(|l| l.ends_with(" ^")), "{args:?}");
        assert!(!args[7].ends_with('^'), "{args:?}");
        assert_eq!(lines.last(), Some(&"pause"));
    }

    #[test]
    fn a_percent_sign_survives_batch() {
        let recipe = Recipe {
            name: "100% Odin".into(),
            ..good()
        };
        let script = recipe.render(true);
        assert!(script.contains("set \"SERVERNAME=100%% Odin\""), "{script}");
    }

    #[test]
    fn steams_own_scripts_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let err = write(dir.path(), "start_headless_server.bat", &good(), true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("belongs to Steam"), "{err}");

        let err = write(dir.path(), "../escape.bat", &good(), true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not a file name"), "{err}");

        let err = write(dir.path(), "server.exe", &good(), true)
            .unwrap_err()
            .to_string();
        assert!(err.contains(".bat"), "{err}");
    }

    #[test]
    fn an_existing_script_is_only_replaced_on_purpose() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "start.bat", &good(), false).unwrap();
        assert!(path.is_file());

        let err = write(dir.path(), "start.bat", &good(), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already exists"), "{err}");

        let other = Recipe {
            port: 2500,
            ..good()
        };
        write(dir.path(), "start.bat", &other, true).unwrap();
        let back = parse_start_script(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(back.port, Some(2500));
    }
}
