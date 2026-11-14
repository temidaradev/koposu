//! koposu: the songs of your osu!lazer and osu!stable libraries in kopuz.
//!
//! A source is one or more folders: an osu!lazer data folder (the one with
//! `client.realm`), an osu!stable install, or its `Songs` folder. kopuz
//! mounts them read-only, and only because the marketplace lists this
//! plugin as verified. Nothing is written and nothing leaves the computer.

pub mod audio;
pub mod lazer;
pub mod library;
pub mod osu_file;
pub mod realm;
pub mod stable;

use std::path::{Path, PathBuf};

use kopuz_plugin_sdk::{
    AuthOutcome, Check, CheckOutcome, CheckStep, Connection, LibrarySnapshot, Result, Source,
    SourceError, StreamInfo, StreamQuality,
};

use library::Collected;

/// One game's library found in a source's folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Library {
    /// An osu!lazer data folder.
    Lazer(PathBuf),
    /// An osu!stable `Songs` folder.
    Stable(PathBuf),
}

impl Library {
    pub fn find(folder: &Path) -> Option<Self> {
        if lazer::is_library(folder) {
            return Some(Library::Lazer(folder.to_path_buf()));
        }
        stable::songs(folder).map(Library::Stable)
    }

    fn describe(&self) -> String {
        match self {
            Library::Lazer(root) => format!("osu!lazer at {}", root.display()),
            Library::Stable(songs) => format!("osu!stable songs at {}", songs.display()),
        }
    }

    fn scan(&self, out: &mut Collected) -> std::result::Result<(), String> {
        match self {
            Library::Lazer(root) => lazer::scan(root, out),
            Library::Stable(songs) => stable::scan(songs, out),
        }
    }

    /// The file a track id names, if it is in this library.
    fn file(&self, item_id: &str) -> Option<PathBuf> {
        match (self, item_id.split_once(':')?) {
            (Library::Lazer(root), ("lazer", hash)) => {
                lazer::file(root, hash).filter(|path| path.is_file())
            }
            (Library::Stable(songs), ("stable", item)) => stable::file(songs, item),
            _ => None,
        }
    }
}

pub struct Osu {
    folders: Vec<PathBuf>,
}

kopuz_plugin_sdk::export_source!(Osu);

impl Osu {
    pub fn new(folders: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            folders: folders.into_iter().collect(),
        }
    }

    pub fn libraries(&self) -> Vec<Library> {
        self.folders
            .iter()
            .filter_map(|folder| Library::find(folder))
            .collect()
    }

    pub fn library(&self) -> Result<LibrarySnapshot> {
        let libraries = self.libraries();
        if libraries.is_empty() {
            return Err(nothing_found());
        }
        let mut collected = Collected::default();
        let mut failures = Vec::new();
        for library in &libraries {
            if let Err(error) = library.scan(&mut collected) {
                kopuz_plugin_sdk::warn!("{}: {error}", library.describe());
                failures.push(format!("{}: {error}", library.describe()));
            }
        }
        // One unreadable library should not empty the others.
        if collected.is_empty() && !failures.is_empty() {
            return Err(SourceError::Backend(failures.join("; ")));
        }
        Ok(collected.snapshot())
    }
}

fn nothing_found() -> SourceError {
    SourceError::InvalidInput(
        "no osu! library in these folders; pick the osu!lazer data folder (with client.realm) or the osu!stable folder"
            .into(),
    )
}

impl Source for Osu {
    fn open(connection: Connection) -> Self {
        Self::new(connection.folders.into_iter().map(PathBuf::from))
    }

    fn validate(&self) -> AuthOutcome {
        if self.libraries().is_empty() {
            AuthOutcome::Unreachable
        } else {
            AuthOutcome::Valid
        }
    }

    fn diagnose(&self) -> Result<Vec<Check>> {
        let libraries = self.libraries();
        let check = if libraries.is_empty() {
            Check {
                step: CheckStep::Reach,
                outcome: CheckOutcome::Failed,
                detail: nothing_found().message(),
            }
        } else {
            Check {
                step: CheckStep::Reach,
                outcome: CheckOutcome::Passed,
                detail: libraries
                    .iter()
                    .map(Library::describe)
                    .collect::<Vec<_>>()
                    .join(", "),
            }
        };
        Ok(vec![check])
    }

    fn fetch_library(&self) -> Result<LibrarySnapshot> {
        self.library()
    }

    fn resolve_stream(&self, item_id: &str, _: StreamQuality) -> Result<StreamInfo> {
        self.libraries()
            .iter()
            .find_map(|library| library.file(item_id))
            .map(|path| StreamInfo::url(path.to_string_lossy()))
            .ok_or_else(|| SourceError::InvalidInput(format!("no osu! song {item_id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koposu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_stable_install_is_one_album_per_set_and_one_track_per_song() {
        let install = temp("stable");
        let set = install.join("Songs").join("241526 Soleily - Renatus");
        std::fs::create_dir_all(&set).unwrap();
        for version in ["Hard", "Insane"] {
            std::fs::write(
                set.join(format!("Soleily - Renatus (Gamu) [{version}].osu")),
                format!(
                    "osu file format v14\n[General]\nAudioFilename: Renatus.MP3\n[Metadata]\nTitle:Renatus\nArtist:Soleily\nVersion:{version}\nBeatmapSetID:241526\n[Events]\n0,0,\"BG.jpg\",0,0\n[HitObjects]\n"
                ),
            )
            .unwrap();
        }
        // Mapped on Windows: the files' case differs from what the map says.
        std::fs::write(
            set.join("renatus.mp3"),
            [0xff, 0xfb, 0x90, 0x00, 0, 0, 0, 0],
        )
        .unwrap();
        std::fs::write(set.join("bg.JPG"), b"picture").unwrap();

        let osu = Osu::new([install.clone()]);
        assert_eq!(osu.libraries(), [Library::Stable(install.join("Songs"))]);
        let library = osu.library().expect("a library");
        assert_eq!(library.albums.len(), 1);
        assert_eq!(library.tracks.len(), 1);
        let track = &library.tracks[0];
        assert_eq!(track.title, "Renatus");
        assert_eq!(track.artist, "Soleily");
        assert_eq!(track.album_id, "stable:241526");
        assert_eq!(track.item_id, "stable:241526 Soleily - Renatus/renatus.mp3");
        assert_eq!(
            track.cover.as_deref(),
            Some(&*set.join("bg.JPG").to_string_lossy())
        );
        let stream = osu
            .resolve_stream(&track.item_id, StreamQuality::High)
            .expect("a stream");
        assert_eq!(stream.url, set.join("renatus.mp3").to_string_lossy());

        assert!(
            osu.resolve_stream("stable:../../etc/passwd", StreamQuality::High)
                .is_err()
        );
        assert!(
            osu.resolve_stream("lazer:../x", StreamQuality::High)
                .is_err()
        );
        std::fs::remove_dir_all(&install).ok();
    }

    #[test]
    fn a_folder_with_no_osu_library_is_said_so() {
        let empty = temp("empty");
        let osu = Osu::new([empty.clone()]);
        assert_eq!(osu.validate(), AuthOutcome::Unreachable);
        assert!(matches!(osu.library(), Err(SourceError::InvalidInput(_))));
        std::fs::remove_dir_all(&empty).ok();
    }

    /// `KOPOSU_TEST_LIBRARY=~/.local/share/osu cargo test real -- --nocapture`
    /// reads a real library, lazer or stable, and checks every song plays.
    #[test]
    fn a_real_library_reads_and_every_song_resolves() {
        let Some(folder) = std::env::var_os("KOPOSU_TEST_LIBRARY") else {
            return;
        };
        let osu = Osu::new([PathBuf::from(folder)]);
        let library = osu.library().expect("a library");
        assert!(!library.tracks.is_empty());
        let measured = library
            .tracks
            .iter()
            .filter(|track| track.duration_secs > 0)
            .count();
        for track in &library.tracks {
            osu.resolve_stream(&track.item_id, StreamQuality::High)
                .unwrap_or_else(|error| panic!("{}: {error:?}", track.item_id));
        }
        let covered = library
            .albums
            .iter()
            .filter(|album| album.cover.is_some())
            .count();
        eprintln!(
            "{} albums ({covered} with a cover), {} tracks ({measured} measured)",
            library.albums.len(),
            library.tracks.len()
        );
        for track in library.tracks.iter().take(5) {
            eprintln!(
                "  {} - {} [{}s, {} Hz, {} kbps] {}",
                track.artist,
                track.title,
                track.duration_secs,
                track.sample_rate,
                track.bitrate_kbps,
                track.item_id
            );
        }
    }
}
