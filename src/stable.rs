//! osu!stable keeps each beatmap set as a folder under `Songs/`, with its
//! song, its pictures and a `.osu` file per difficulty naming them.

use std::path::{Component, Path, PathBuf};

use crate::library::{Collected, Song};
use crate::osu_file;

/// The `Songs` folder of an osu!stable install, if `folder` is one or holds one.
pub fn songs(folder: &Path) -> Option<PathBuf> {
    let inside = folder.join("Songs");
    if inside.is_dir() {
        return Some(inside);
    }
    let named = folder
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Songs"));
    (named || holds_beatmap_sets(folder)).then(|| folder.to_path_buf())
}

/// A folder picked by hand may be a `Songs` folder under another name.
fn holds_beatmap_sets(folder: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .take(20)
        .any(|set| !beatmaps(&set.path()).is_empty())
}

fn beatmaps(set: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(set) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("osu"))
        })
        .collect();
    found.sort();
    found
}

/// The file in `dir` a beatmap names, whatever case it was written in: maps
/// made on Windows rarely match the case of their files.
fn named(dir: &Path, name: &str) -> Option<PathBuf> {
    let relative = relative(name)?;
    let exact = dir.join(&relative);
    if exact.is_file() {
        return Some(exact);
    }
    let mut path = dir.to_path_buf();
    for part in relative.components() {
        let wanted = part.as_os_str().to_string_lossy();
        let entry = std::fs::read_dir(&path)
            .ok()?
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&wanted)
            })?;
        path.push(entry.file_name());
    }
    path.is_file().then_some(path)
}

/// `name` as a path below a set folder, or nothing if it would leave it.
fn relative(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(name.replace('\\', "/"));
    path.components()
        .all(|part| matches!(part, Component::Normal(_)))
        .then_some(path)
}

/// The file a stable track id names under `songs`.
pub fn file(songs: &Path, item: &str) -> Option<PathBuf> {
    let path = songs.join(relative(item)?);
    path.is_file().then_some(path)
}

pub fn scan(songs: &Path, out: &mut Collected) -> Result<(), String> {
    let mut sets: Vec<PathBuf> = std::fs::read_dir(songs)
        .map_err(|error| format!("{}: {error}", songs.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    sets.sort();
    for set in sets {
        let Some(folder) = set
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let mut seen: Vec<String> = Vec::new();
        for map in beatmaps(&set) {
            let Ok(bytes) = std::fs::read(&map) else {
                continue;
            };
            let beatmap = osu_file::parse(&String::from_utf8_lossy(&bytes));
            if beatmap.audio.is_empty() || seen.contains(&beatmap.audio.to_lowercase()) {
                continue;
            }
            seen.push(beatmap.audio.to_lowercase());
            let Some(audio) = named(&set, &beatmap.audio) else {
                continue;
            };
            let Ok(within) = audio.strip_prefix(songs) else {
                continue;
            };
            let item = within
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let album = match beatmap.set_id {
                Some(id) => id.to_string(),
                None => folder.clone(),
            };
            out.add(Song {
                item_id: format!("stable:{item}"),
                album_id: format!("stable:{album}"),
                title: beatmap.title,
                title_unicode: beatmap.title_unicode,
                artist: beatmap.artist,
                artist_unicode: beatmap.artist_unicode,
                audio,
                cover: beatmap
                    .background
                    .as_deref()
                    .and_then(|name| named(&set, name)),
                length_ms: 0.0,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_cannot_leave_its_set_folder() {
        assert!(relative("audio.mp3").is_some());
        assert!(relative("sb\\bg.png").is_some());
        for outside in ["../other/audio.mp3", "/etc/passwd", "a/../../b", ""] {
            assert!(
                relative(outside).is_none_or(|path| path.as_os_str().is_empty()),
                "{outside}"
            );
        }
        assert!(file(Path::new("/songs"), "../secret").is_none());
    }
}
