//! The parts of a `.osu` beatmap file a music library needs: what plays, what
//! it is called and what is shown behind it. Everything from the timing
//! points on is skipped.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Beatmap {
    pub audio: String,
    pub title: String,
    pub title_unicode: String,
    pub artist: String,
    pub artist_unicode: String,
    pub creator: String,
    pub version: String,
    pub source: String,
    pub set_id: Option<i64>,
    pub preview_ms: Option<i64>,
    pub background: Option<String>,
}

pub fn parse(text: &str) -> Beatmap {
    let mut beatmap = Beatmap::default();
    let mut section = "";
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = match name {
                "General" | "Metadata" | "Events" => name,
                // Nothing after the events is needed, and the hit objects
                // are most of the file.
                "TimingPoints" | "HitObjects" if beatmap.background.is_some() => break,
                _ => "",
            };
            continue;
        }
        match section {
            "General" | "Metadata" => {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let value = value.trim().to_string();
                match key.trim() {
                    "AudioFilename" => beatmap.audio = value,
                    "PreviewTime" => beatmap.preview_ms = value.parse().ok().filter(|ms| *ms >= 0),
                    "Title" => beatmap.title = value,
                    "TitleUnicode" => beatmap.title_unicode = value,
                    "Artist" => beatmap.artist = value,
                    "ArtistUnicode" => beatmap.artist_unicode = value,
                    "Creator" => beatmap.creator = value,
                    "Version" => beatmap.version = value,
                    "Source" => beatmap.source = value,
                    "BeatmapSetID" => beatmap.set_id = value.parse().ok().filter(|id| *id > 0),
                    _ => {}
                }
            }
            "Events" if beatmap.background.is_none() => {
                beatmap.background = background(line);
            }
            _ => {}
        }
    }
    beatmap
}

/// The picture a background event names: `0,0,"bg.jpg",0,0`.
fn background(line: &str) -> Option<String> {
    let mut fields = line.splitn(4, ',');
    let kind = fields.next()?.trim();
    if kind != "0" && !kind.eq_ignore_ascii_case("Background") {
        return None;
    }
    fields.next()?;
    let file = fields.next()?.trim().trim_matches('"').trim();
    (!file.is_empty()).then(|| file.replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENATUS: &str = "\u{feff}osu file format v14\r\n\r\n[General]\r\nAudioFilename: 03. Renatus - Soleily 192kbps.mp3\r\nAudioLeadIn: 0\r\nPreviewTime: 22077\r\n\r\n[Editor]\r\nDistanceSpacing: 1\r\n\r\n[Metadata]\r\nTitle:Renatus\r\nTitleUnicode:Renatus\r\nArtist:Soleily\r\nArtistUnicode:Soleily\r\nCreator:Gamu\r\nVersion:Insane\r\nSource:\r\nTags:MBC7 Unisphere\r\nBeatmapID:557821\r\nBeatmapSetID:241526\r\n\r\n[Difficulty]\r\nHPDrainRate:6\r\n\r\n[Events]\r\n//Background and Video events\r\n0,0,\"machinetop_background.jpg\",0,0\r\n//Break Periods\r\n\r\n[TimingPoints]\r\n956,329.67032967033,4,2,0,60,1,0\r\n\r\n[HitObjects]\r\n256,192,956,5,0,0:0:0:0:\r\n";

    #[test]
    fn a_beatmap_names_its_song_and_background() {
        let beatmap = parse(RENATUS);
        assert_eq!(beatmap.audio, "03. Renatus - Soleily 192kbps.mp3");
        assert_eq!(beatmap.title, "Renatus");
        assert_eq!(beatmap.artist, "Soleily");
        assert_eq!(beatmap.creator, "Gamu");
        assert_eq!(beatmap.version, "Insane");
        assert_eq!(beatmap.set_id, Some(241526));
        assert_eq!(beatmap.preview_ms, Some(22077));
        assert_eq!(
            beatmap.background.as_deref(),
            Some("machinetop_background.jpg")
        );
    }

    #[test]
    fn a_video_or_a_storyboard_sprite_is_no_background() {
        let text = "[Events]\nVideo,0,\"intro.avi\"\nSprite,Foreground,Centre,\"sb/a.png\",320,240\nBackground,0,\"sub\\bg.png\"\n";
        assert_eq!(parse(text).background.as_deref(), Some("sub/bg.png"));
        assert_eq!(parse("[Events]\nVideo,0,\"intro.avi\"\n").background, None);
    }

    #[test]
    fn an_unsubmitted_map_has_no_set_id() {
        let beatmap = parse("[Metadata]\nTitle:Draft\nBeatmapSetID:-1\n");
        assert_eq!(beatmap.set_id, None);
        assert_eq!(beatmap.title, "Draft");
    }
}
