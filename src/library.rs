//! Songs from either game, turned into the albums and tracks kopuz stores.
//! A beatmap set is an album, and each song in it a track.

use std::collections::HashSet;
use std::path::PathBuf;

use kopuz_plugin_sdk::{Album, ArtistCredit, LibrarySnapshot, Track};

use crate::audio;

pub const GENRE: &str = "osu!";

/// One song of a beatmap set, as either game describes it.
#[derive(Debug, Clone)]
pub struct Song {
    pub item_id: String,
    pub album_id: String,
    pub title: String,
    pub title_unicode: String,
    pub artist: String,
    pub artist_unicode: String,
    pub audio: PathBuf,
    pub cover: Option<PathBuf>,
    /// How long the difficulties playing it last, for a song whose file
    /// says nothing of its length.
    pub length_ms: f64,
}

#[derive(Default)]
pub struct Collected {
    albums: Vec<Album>,
    tracks: Vec<Track>,
    seen_albums: HashSet<String>,
    seen_tracks: HashSet<String>,
}

/// The romanised name osu! shows by default, or the original when a map
/// gives only that.
fn pick(romanised: &str, original: &str) -> String {
    let name = if romanised.trim().is_empty() {
        original
    } else {
        romanised
    };
    name.trim().to_string()
}

fn path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Collected {
    pub fn add(&mut self, song: Song) {
        // The same song file can sit in two sets; it is one track.
        if !self.seen_tracks.insert(song.item_id.clone()) {
            return;
        }
        let title = match pick(&song.title, &song.title_unicode) {
            title if title.is_empty() => song
                .audio
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default(),
            title => title,
        };
        let artist = pick(&song.artist, &song.artist_unicode);
        let cover = song.cover.as_deref().map(path_text);
        let info = audio::probe(&song.audio).unwrap_or_default();
        let duration_secs = match info.duration_secs {
            0 => (song.length_ms / 1000.0).ceil() as u64,
            seconds => seconds,
        };
        if self.seen_albums.insert(song.album_id.clone()) {
            self.albums.push(Album {
                id: song.album_id.clone(),
                title: title.clone(),
                artist: artist.clone(),
                genre: GENRE.to_string(),
                year: 0,
                cover: cover.clone(),
                artist_id: None,
            });
        }
        self.tracks.push(Track {
            item_id: song.item_id,
            cover,
            album_id: song.album_id,
            title: title.clone(),
            album: title,
            duration_secs,
            sample_rate: info.sample_rate,
            bitrate_kbps: info.bitrate_kbps,
            track_number: Some(1),
            artists: vec![artist.clone()],
            credits: vec![ArtistCredit::unlinked(&artist)],
            artist,
            ..Track::default()
        });
    }

    pub fn snapshot(self) -> LibrarySnapshot {
        LibrarySnapshot {
            albums: self.albums,
            tracks: self.tracks,
            artists: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}
