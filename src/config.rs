//! User settings, stored as TOML under `%APPDATA%\PetPal`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::sprites::{self, Palette};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Integer upscale of the sprite. 3 gives a 96px creature from 32px art.
    pub scale: u32,
    /// Overall window opacity, 0-255.
    pub opacity: u8,
    /// Walking speed in pixels per second.
    pub speed: f32,
    /// How restless the creature is, 0-100. It is the chance that any given
    /// decision is to move rather than settle, and it also stretches or
    /// shortens how long each choice is held: at 0 the pet mostly sits still,
    /// at 100 it is almost always on the move.
    pub roam: u8,
    pub chase_cursor: bool,
    /// Stand on the top edges of other windows, not just the desktop floor.
    pub walk_on_windows: bool,
    /// Hop up onto windows and drop back down of its own accord. With this off
    /// the creature stays on whatever surface it is currently standing on and
    /// just roams along it.
    pub jump_between_windows: bool,
    /// Perk up and look at newly opened application windows.
    pub react_to_new_apps: bool,
    /// Fall asleep after this many seconds with no keyboard or mouse input.
    pub sleep_after_idle_secs: u64,
    /// Get visibly annoyed above this whole-machine CPU load, in percent.
    pub cpu_annoy_percent: u8,
    /// Which creature to wear: a built-in id (`pal`, `vader`), the name of a
    /// folder under `<config>/sprites/`, or an absolute path to one.
    pub sprite: String,
    pub colors: Colors,
    /// Skipped when empty: TOML forbids appending `[[reminder]]` blocks to a
    /// statically declared `reminder = []`, so emitting the empty array would
    /// break the very edit the file's own header tells the user to make.
    #[serde(rename = "reminder", skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<Reminder>,

    /// Set when the file on disk failed to parse. Guards `save` so a tray
    /// toggle cannot overwrite (and destroy) a config the user is mid-edit.
    #[serde(skip)]
    unparseable: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            scale: 3,
            opacity: 255,
            speed: 46.0,
            roam: 45,
            chase_cursor: false,
            walk_on_windows: true,
            jump_between_windows: true,
            react_to_new_apps: true,
            sleep_after_idle_secs: 180,
            cpu_annoy_percent: 80,
            sprite: sprites::Kind::Pal.id().to_string(),
            colors: Colors::default(),
            reminders: Vec::new(),
            unparseable: false,
        }
    }
}

/// Per-key colour overrides.
///
/// Every field is optional because each built-in creature ships its own
/// palette — Pal is warm orange, Vader is armour black with a pale rim light.
/// Writing a concrete default here would repaint whichever creature you
/// switched to in the other one's colours.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Colors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub belly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eye: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blush: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
}

impl Colors {
    /// Layer any overrides over a creature's own palette. An unparseable entry
    /// is ignored rather than fatal, so one typo cannot leave the user with an
    /// invisible pet.
    pub fn apply(&self, base: Palette) -> Palette {
        let over = |slot: &Option<String>, fallback: u32| {
            slot.as_deref()
                .and_then(sprites::parse_hex)
                .unwrap_or(fallback)
        };
        Palette {
            body: over(&self.body, base.body),
            belly: over(&self.belly, base.belly),
            outline: over(&self.outline, base.outline),
            eye: over(&self.eye, base.eye),
            blush: over(&self.blush, base.blush),
            accent: over(&self.accent, base.accent),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Reminder {
    /// 24-hour local time, `"HH:MM"`.
    pub at: String,
    pub text: String,
    /// Weekday names (`mon`..`sun`) it applies to. Empty means every day.
    #[serde(default)]
    pub days: Vec<String>,
}

impl Config {
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join("PetPal")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    /// Drop-in folder for user sprite sheets. Each immediate subfolder holding
    /// a `sprite.toml` shows up in the tray's Sprite menu, so importing is
    /// "copy a folder in here" rather than "edit a path in a config file".
    pub fn sprites_dir() -> PathBuf {
        Self::dir().join("sprites")
    }

    /// Names of the installed sprite sheets, alphabetically.
    pub fn installed_sheets() -> Vec<String> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(Self::sprites_dir()) else {
            return out;
        };
        for e in entries.flatten() {
            if !e.path().join("sprite.toml").is_file() {
                continue;
            }
            if let Some(name) = e.file_name().to_str() {
                out.push(name.to_string());
            }
        }
        out.sort_unstable();
        out
    }

    /// Resolve `sprite` to a sheet folder, if it names one. `None` means it is
    /// a built-in creature id.
    pub fn sheet_path(&self) -> Option<PathBuf> {
        let name = self.sprite.trim();
        if name.is_empty() || sprites::Kind::from_id(name).is_some() {
            return None;
        }
        let installed = Self::sprites_dir().join(name);
        if installed.join("sprite.toml").is_file() {
            return Some(installed);
        }
        // Fall back to treating it as a path, for sheets kept elsewhere.
        Some(PathBuf::from(name))
    }

    /// Load the config, writing a commented default file on first run.
    ///
    /// A malformed file is never overwritten — we fall back to defaults, mark
    /// the config read-only and report the error so the user can fix the edit
    /// without losing the rest of it.
    pub fn load() -> (Config, Option<String>) {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(cfg) => (cfg.sanitised(), None),
                Err(e) => {
                    let cfg = Config {
                        unparseable: true,
                        ..Config::default()
                    };
                    (
                        cfg,
                        Some(format!(
                            "config.toml has an error at {} — running on defaults, \
                             and settings will not be saved until it is fixed.",
                            error_position(&e)
                        )),
                    )
                }
            },
            Err(_) => {
                let cfg = Config::default();
                let _ = cfg.save();
                write_sprite_guide();
                (cfg, None)
            }
        }
    }

    /// Clamp anything a hand-edited file could set to a value that would break
    /// rendering or peg the CPU.
    fn sanitised(mut self) -> Config {
        self.scale = self.scale.clamp(1, 8);
        self.opacity = self.opacity.max(24);
        self.speed = self.speed.clamp(4.0, 600.0);
        self.roam = self.roam.min(100);
        self.sleep_after_idle_secs = self.sleep_after_idle_secs.clamp(5, 86_400);
        self.cpu_annoy_percent = self.cpu_annoy_percent.clamp(10, 100);
        self
    }

    pub fn save(&self) -> std::io::Result<()> {
        if self.unparseable {
            return Err(std::io::Error::other(
                "config.toml has a syntax error; fix it and use Reload",
            ));
        }
        let dir = Self::dir();
        std::fs::create_dir_all(&dir)?;
        let body = toml::to_string_pretty(self)
            .unwrap_or_else(|e| format!("# could not serialise config: {e}\n"));
        std::fs::write(Self::path(), format!("{HEADER}{body}"))
    }
}

fn error_position(e: &toml::de::Error) -> String {
    e.span()
        .map(|s| format!("byte {}", s.start))
        .unwrap_or_else(|| "an unknown position".into())
}

/// Create the sprite drop-in folder and leave the authoring guide in it, so the
/// feature is discoverable without going back to the repo.
///
/// Rewritten every launch, not just the first: the guide changes when an
/// animation is added, and a user who installed six months ago should not be
/// reading a version that has never heard of `climb`. It is generated output,
/// so there is nothing of theirs to overwrite.
///
/// The guide is `include_str!`d from `docs/SPRITES.md` rather than duplicated:
/// one copy, no drift. It lands as `.txt` so a double-click opens it.
///
/// It sits loose at the top level on purpose — the folder scan only treats
/// *subfolders* containing a `sprite.toml` as installed sheets, so a stray file
/// here cannot be mistaken for a broken creature.
fn write_sprite_guide() {
    let dir = Config::sprites_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("HOW-TO-make-a-sprite.txt"), SPRITE_GUIDE);
    }
}

/// The sprite authoring guide, shared with `docs/SPRITES.md`.
const SPRITE_GUIDE: &str = include_str!("../docs/SPRITES.md");

#[cfg(test)]
mod tests {
    use super::*;

    fn serialise(cfg: &Config) -> String {
        format!("{HEADER}{}", toml::to_string_pretty(cfg).unwrap())
    }

    /// The guide is what users author sheets against, and it is shipped into
    /// their sprites folder. An animation the loader accepts but the guide never
    /// names is an animation nobody will draw — and every built-in and every
    /// creature id has to be findable there too.
    #[test]
    fn the_shipped_guide_documents_everything() {
        for a in crate::sprites::Anim::ALL {
            let key = a.key();
            assert!(
                SPRITE_GUIDE.contains(&format!("`{key}`")),
                "docs/SPRITES.md never mentions the `{key}` animation"
            );
        }
        // Quoted, and in the header specifically: that comment is the canonical
        // list of what `sprite =` accepts. A bare substring search would pass on
        // "holding it with the mouse".
        for kind in sprites::Kind::ALL {
            let id = kind.id();
            assert!(
                HEADER.contains(&format!("\"{id}\"")),
                "the config header does not list {id:?} as a sprite option"
            );
        }
    }

    /// The header tells users to append `[[reminder]]` blocks. TOML rejects
    /// appending to a statically declared array, so the emitted file must not
    /// contain `reminder = []` — otherwise following our own instructions
    /// silently resets every setting.
    #[test]
    fn appending_a_reminder_to_a_fresh_config_parses() {
        let text = serialise(&Config::default());
        assert!(
            !text.contains("reminder = []"),
            "empty reminder array must be omitted:\n{text}"
        );

        let edited = format!("{text}\n[[reminder]]\nat = \"14:30\"\ntext = \"stretch\"\n");
        let parsed: Config = toml::from_str(&edited).expect("appended reminder should parse");
        assert_eq!(parsed.reminders.len(), 1);
        assert_eq!(parsed.reminders[0].at, "14:30");
        // The rest of the settings must survive the edit.
        assert_eq!(parsed.scale, Config::default().scale);
        assert_eq!(parsed.colors.body, Config::default().colors.body);
    }

    #[test]
    fn round_trips_with_reminders() {
        let mut cfg = Config::default();
        cfg.chase_cursor = true;
        cfg.scale = 4;
        cfg.reminders.push(Reminder {
            at: "09:00".into(),
            text: "standup".into(),
            days: vec!["mon".into(), "fri".into()],
        });
        let parsed: Config = toml::from_str(&serialise(&cfg)).unwrap();
        assert!(parsed.chase_cursor);
        assert_eq!(parsed.scale, 4);
        assert_eq!(parsed.reminders.len(), 1);
        assert_eq!(parsed.reminders[0].days, vec!["mon", "fri"]);
    }

    /// A broken file must not be silently overwritten by a later tray toggle.
    #[test]
    fn broken_config_refuses_to_save() {
        let cfg = Config {
            unparseable: true,
            ..Config::default()
        };
        assert!(cfg.save().is_err());
    }

    #[test]
    fn builtin_ids_are_not_treated_as_folders() {
        for kind in sprites::Kind::ALL {
            let cfg = Config {
                sprite: kind.id().into(),
                ..Config::default()
            };
            assert!(
                cfg.sheet_path().is_none(),
                "{} should resolve to a built-in",
                kind.id()
            );
            assert_eq!(sprites::Kind::from_id(&cfg.sprite), Some(kind));
        }
        // An empty value is the default creature, not a folder called "".
        let cfg = Config {
            sprite: String::new(),
            ..Config::default()
        };
        assert!(cfg.sheet_path().is_none());
    }

    #[test]
    fn unknown_sprite_names_resolve_to_a_sheet_path() {
        let cfg = Config {
            sprite: "my-cat".into(),
            ..Config::default()
        };
        assert!(cfg.sheet_path().is_some());
        assert!(sprites::Kind::from_id(&cfg.sprite).is_none());
    }

    /// Each creature keeps its own colours unless a key is explicitly set —
    /// otherwise switching to Vader would repaint him in Pal's orange.
    #[test]
    fn colours_default_per_creature_and_override_per_key() {
        let none = Colors::default();
        for kind in sprites::Kind::ALL {
            let base = kind.palette();
            let applied = none.apply(base);
            assert_eq!(applied.body, base.body);
            assert_eq!(applied.outline, base.outline);
        }

        let one = Colors {
            body: Some("#112233".into()),
            ..Colors::default()
        };
        let vader = sprites::Kind::Vader.palette();
        let applied = one.apply(vader);
        assert_eq!(applied.body, 0xFF11_2233);
        // Untouched keys are left alone.
        assert_eq!(applied.outline, vader.outline);

        // A malformed value falls back rather than blanking the creature.
        let bad = Colors {
            body: Some("not a colour".into()),
            ..Colors::default()
        };
        assert_eq!(bad.apply(vader).body, vader.body);
    }

    /// An untouched colours table must not serialise concrete values, or the
    /// next creature switch would inherit the previous creature's palette.
    #[test]
    fn default_colours_serialise_empty() {
        // Check the emitted TOML, not `serialise`, whose header comment
        // legitimately mentions `body = ` as an example.
        let body = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(
            !body.contains("body = "),
            "colours leaked into config:\n{body}"
        );
        let parsed: Config = toml::from_str(&serialise(&Config::default())).unwrap();
        assert!(parsed.colors.body.is_none());
    }

    #[test]
    fn clamps_hostile_values() {
        let cfg = Config {
            scale: 999,
            speed: -5.0,
            sleep_after_idle_secs: 0,
            cpu_annoy_percent: 0,
            ..Config::default()
        }
        .sanitised();
        assert_eq!(cfg.scale, 8);
        assert!(cfg.speed >= 4.0);
        assert!(cfg.sleep_after_idle_secs >= 5);
        assert!(cfg.cpu_annoy_percent >= 10);
    }
}

const HEADER: &str = "\
# PetPal configuration. Edit and pick \"Reload config & sprites\" from the
# tray menu -- or just use the tray menu, which writes this file for you.
#
# sprite   : \"pal\", \"vader\", \"mouse\", \"monkey\", or a folder in sprites\\
#            next to this file. Drop a folder with a sprite.toml + PNG in there
#            and it appears in the tray's Sprite menu. Writing one is covered
#            in sprites\\HOW-TO-make-a-sprite.txt -- or skip all of it and use
#            Tray > Sprite > Make a copy to edit...
# jump_between_windows
#          : whether it moves between surfaces on its own -- hopping, climbing
#            window edges, and stepping off ledges. Off = it stays on whatever
#            it is standing on and just roams along it.
# roam     : 0-100, how restless the creature is -- how often it wanders off
#            on its own rather than settling. The tray's \"Roam around\" menu
#            sets this in steps; any value in between works here.
# speed    : base walking speed. Each built-in scales it a little (the mouse
#            skitters, Vader does not hurry).
# colors   : optional overrides, hex #rrggbb or #aarrggbb. Each creature has
#            its own palette; anything you leave out keeps the creature's own
#            colour, so you can recolour one key without repainting the rest.
#
#            [colors]
#            body = \"#5AA6F2\"
#
# reminder : repeat the [[reminder]] block as many times as you like.
#
#            [[reminder]]
#            at = \"14:30\"
#            text = \"Stand up and stretch\"
#            days = [\"mon\", \"tue\", \"wed\", \"thu\", \"fri\"]   # omit for daily

";
