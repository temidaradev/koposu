//! osu!lazer keeps every file once, named by its SHA-256 under `files/`, and
//! what each beatmap set calls its files in the Realm database next to them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::library::{Collected, Song};
use crate::realm::{Column, Obj, Realm, Table};

pub const DATABASE: &str = "client.realm";

pub fn is_library(folder: &Path) -> bool {
    folder.join(DATABASE).is_file() && folder.join("files").is_dir()
}

/// Where the file with `hash` is kept under `root`.
pub fn file(root: &Path, hash: &str) -> Option<PathBuf> {
    let valid = hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
    valid.then(|| {
        root.join("files")
            .join(&hash[..1])
            .join(&hash[..2])
            .join(hash)
    })
}

pub fn scan(root: &Path, out: &mut Collected) -> Result<(), String> {
    let bytes = std::fs::read(root.join(DATABASE))
        .map_err(|error| format!("{}: {error}", root.join(DATABASE).display()))?;
    let realm = Realm::open(&bytes).map_err(|error| error.to_string())?;
    let sets = realm.table("BeatmapSet").map_err(|e| e.to_string())?;
    let beatmaps = realm.table("Beatmap").map_err(|e| e.to_string())?;
    let metadata = realm.table("BeatmapMetadata").map_err(|e| e.to_string())?;
    let usages = realm
        .table("RealmNamedFileUsage")
        .map_err(|e| e.to_string())?;
    let files = realm.table("File").map_err(|e| e.to_string())?;
    read(
        root,
        &Tables {
            sets,
            beatmaps,
            metadata,
            usages,
            files,
        },
        out,
    )
    .map_err(|error| error.to_string())
}

struct Tables<'r, 'a> {
    sets: Table<'r, 'a>,
    beatmaps: Table<'r, 'a>,
    metadata: Table<'r, 'a>,
    usages: Table<'r, 'a>,
    files: Table<'r, 'a>,
}

/// A string, empty when it is null or unreadable.
fn text<'a>(table: &Table<'_, 'a>, obj: &Obj<'a>, column: &Column) -> String {
    table.string(obj, column).ok().flatten().unwrap_or_default()
}

/// What a set's difficulties say about one of its songs.
struct Heard {
    metadata: i64,
    length_ms: f64,
}

fn read(root: &Path, t: &Tables<'_, '_>, out: &mut Collected) -> crate::realm::Result<()> {
    let set_online = t.sets.column("OnlineID")?.clone();
    let set_deleted = t.sets.column("DeletePending")?.clone();
    let set_hash = t.sets.column("Hash")?.clone();
    let set_beatmaps = t.sets.column("Beatmaps")?.clone();
    let set_files = t.sets.column("Files")?.clone();
    let beatmap_metadata = t.beatmaps.column("Metadata")?.clone();
    let beatmap_length = t.beatmaps.column("Length")?.clone();
    let title = t.metadata.column("Title")?.clone();
    let title_unicode = t.metadata.column("TitleUnicode")?.clone();
    let artist = t.metadata.column("Artist")?.clone();
    let artist_unicode = t.metadata.column("ArtistUnicode")?.clone();
    let audio_file = t.metadata.column("AudioFile")?.clone();
    let background_file = t.metadata.column("BackgroundFile")?.clone();
    let usage_file = t.usages.column("File")?.clone();
    let usage_name = t.usages.column("Filename")?.clone();
    let file_hash = t.files.column("Hash")?.clone();

    let beatmaps = t.beatmaps.by_key()?;
    let metadata = t.metadata.by_key()?;
    let usages = t.usages.by_key()?;
    let files = t.files.by_key()?;

    for set in t.sets.objects()? {
        if t.sets.int(&set, &set_deleted)?.unwrap_or(0) != 0 {
            continue;
        }
        // A set calls its files by name; the store keeps them by hash.
        let mut named: HashMap<String, String> = HashMap::new();
        for key in t.sets.links(&set, &set_files)? {
            let Some(usage) = usages.get(&key) else {
                continue;
            };
            let name = text(&t.usages, usage, &usage_name);
            let Some(file) = t
                .usages
                .link(usage, &usage_file)?
                .and_then(|key| files.get(&key))
            else {
                continue;
            };
            named.insert(name.to_lowercase(), text(&t.files, file, &file_hash));
        }

        // One song per audio file, however many difficulties play it.
        let mut songs: Vec<(String, Heard)> = Vec::new();
        for key in t.sets.links(&set, &set_beatmaps)? {
            let Some(beatmap) = beatmaps.get(&key) else {
                continue;
            };
            let Some(meta_key) = t.beatmaps.link(beatmap, &beatmap_metadata)? else {
                continue;
            };
            let Some(meta) = metadata.get(&meta_key) else {
                continue;
            };
            let audio = text(&t.metadata, meta, &audio_file).to_lowercase();
            if audio.is_empty() {
                continue;
            }
            let length_ms = t.beatmaps.double(beatmap, &beatmap_length)?.unwrap_or(0.0);
            match songs.iter_mut().find(|(name, _)| *name == audio) {
                Some((_, heard)) => heard.length_ms = heard.length_ms.max(length_ms),
                None => songs.push((
                    audio,
                    Heard {
                        metadata: meta_key,
                        length_ms,
                    },
                )),
            }
        }

        let online = t.sets.int(&set, &set_online)?.filter(|id| *id > 0);
        let set_id = match online {
            Some(id) => id.to_string(),
            None => format!("local-{}", text(&t.sets, &set, &set_hash)),
        };
        for (audio, heard) in songs {
            let Some(meta) = metadata.get(&heard.metadata) else {
                continue;
            };
            let Some(path) = named
                .get(&audio)
                .and_then(|hash| file(root, hash).map(|path| (hash, path)))
            else {
                continue;
            };
            let (hash, path) = path;
            if !path.is_file() {
                continue;
            }
            let cover = named
                .get(&text(&t.metadata, meta, &background_file).to_lowercase())
                .and_then(|hash| file(root, hash))
                .filter(|cover| cover.is_file());
            out.add(Song {
                item_id: format!("lazer:{hash}"),
                album_id: format!("lazer:{set_id}"),
                title: text(&t.metadata, meta, &title),
                title_unicode: text(&t.metadata, meta, &title_unicode),
                artist: text(&t.metadata, meta, &artist),
                artist_unicode: text(&t.metadata, meta, &artist_unicode),
                audio: path,
                cover,
                length_ms: heard.length_ms,
            });
        }
    }
    Ok(())
}
