//! How long a song is and how it is encoded, read from the first and last
//! few kilobytes of its file. osu! songs are almost always MP3 or Ogg Vorbis;
//! anything else is left for the player to measure.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const PROBE: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Info {
    pub duration_secs: u64,
    pub sample_rate: u32,
    pub bitrate_kbps: u32,
}

pub fn probe(path: &Path) -> Option<Info> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut head = Vec::new();
    (&mut file).take(PROBE).read_to_end(&mut head).ok()?;
    if head.starts_with(b"OggS") {
        let start = size.saturating_sub(PROBE);
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail).ok()?;
        return ogg(&head, &tail, size);
    }
    mp3(&head, size)
}

fn mp3(head: &[u8], size: u64) -> Option<Info> {
    let start = id3_size(head);
    let at = (start..head.len().saturating_sub(4)).find(|at| frame(&head[*at..]).is_some())?;
    let frame = frame(&head[at..])?;
    let frames = xing_frames(&head[at..], &frame).or_else(|| vbri_frames(&head[at..]));
    let (duration_secs, bitrate_kbps) = match frames {
        Some(frames) if frames > 0 => {
            let seconds = frames as f64 * f64::from(frame.samples) / f64::from(frame.sample_rate);
            let audio = size.saturating_sub(start as u64) as f64;
            let kbps = if seconds > 0.0 {
                audio * 8.0 / seconds / 1000.0
            } else {
                0.0
            };
            (seconds.round() as u64, kbps.round() as u32)
        }
        _ => {
            let audio = size.saturating_sub(start as u64);
            let seconds = audio as f64 * 8.0 / (f64::from(frame.bitrate_kbps) * 1000.0);
            (seconds.round() as u64, frame.bitrate_kbps)
        }
    };
    Some(Info {
        duration_secs,
        sample_rate: frame.sample_rate,
        bitrate_kbps,
    })
}

/// Bytes of ID3v2 tag before the first frame, or none.
fn id3_size(head: &[u8]) -> usize {
    if head.len() < 10 || &head[..3] != b"ID3" {
        return 0;
    }
    let size = head[6..10]
        .iter()
        .fold(0usize, |size, byte| (size << 7) | usize::from(byte & 0x7f));
    let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

struct Frame {
    mpeg1: bool,
    mono: bool,
    samples: u32,
    sample_rate: u32,
    bitrate_kbps: u32,
}

fn frame(bytes: &[u8]) -> Option<Frame> {
    let [a, b, c, d] = *bytes.get(..4)? else {
        return None;
    };
    if a != 0xff || b & 0xe0 != 0xe0 {
        return None;
    }
    let version = (b >> 3) & 3;
    let layer = (b >> 1) & 3;
    let bitrate_index = usize::from(c >> 4);
    let rate_index = usize::from((c >> 2) & 3);
    if version == 1 || layer == 0 || bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
        return None;
    }
    let mpeg1 = version == 3;
    const V1: [[u32; 15]; 3] = [
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ],
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
        ],
        [
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
        ],
    ];
    const V2: [[u32; 15]; 2] = [
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
        ],
    ];
    // Layer bits: 1 is layer III, 2 layer II, 3 layer I.
    let bitrate_kbps = match (mpeg1, layer) {
        (true, layer) => V1[usize::from(layer - 1)][bitrate_index],
        (false, 3) => V2[1][bitrate_index],
        (false, _) => V2[0][bitrate_index],
    };
    let base = [44100, 48000, 32000][rate_index];
    let sample_rate = match version {
        3 => base,
        2 => base / 2,
        _ => base / 4,
    };
    let samples = match (layer, mpeg1) {
        (3, _) => 384,
        (2, _) | (1, true) => 1152,
        _ => 576,
    };
    Some(Frame {
        mpeg1,
        mono: d >> 6 == 3,
        samples,
        sample_rate,
        bitrate_kbps,
    })
}

/// The frame count a Xing or Info header in the first frame gives.
fn xing_frames(bytes: &[u8], frame: &Frame) -> Option<u32> {
    let side = match (frame.mpeg1, frame.mono) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let at = 4 + side;
    let tag = bytes.get(at..at + 4)?;
    if tag != b"Xing" && tag != b"Info" {
        return None;
    }
    let flags = u32::from_be_bytes(bytes.get(at + 4..at + 8)?.try_into().ok()?);
    (flags & 1 != 0).then(|| {
        bytes
            .get(at + 8..at + 12)?
            .try_into()
            .ok()
            .map(u32::from_be_bytes)
    })?
}

fn vbri_frames(bytes: &[u8]) -> Option<u32> {
    let at = 36;
    (bytes.get(at..at + 4)? == b"VBRI").then(|| {
        bytes
            .get(at + 14..at + 18)?
            .try_into()
            .ok()
            .map(u32::from_be_bytes)
    })?
}

fn ogg(head: &[u8], tail: &[u8], size: u64) -> Option<Info> {
    let (rate, granule_rate, skip) = if let Some(at) = find(head, b"\x01vorbis") {
        let rate = u32::from_le_bytes(head.get(at + 12..at + 16)?.try_into().ok()?);
        (rate, rate, 0)
    } else if let Some(at) = find(head, b"OpusHead") {
        let skip = u16::from_le_bytes(head.get(at + 10..at + 12)?.try_into().ok()?);
        let rate = u32::from_le_bytes(head.get(at + 12..at + 16)?.try_into().ok()?);
        (
            if rate == 0 { 48_000 } else { rate },
            48_000,
            u64::from(skip),
        )
    } else {
        return None;
    };
    if granule_rate == 0 {
        return None;
    }
    let last = rfind(tail, b"OggS")?;
    let granule = u64::from_le_bytes(tail.get(last + 6..last + 14)?.try_into().ok()?);
    let seconds = granule.saturating_sub(skip) as f64 / f64::from(granule_rate);
    let bitrate_kbps = if seconds > 0.0 {
        (size as f64 * 8.0 / seconds / 1000.0).round() as u32
    } else {
        0
    };
    Some(Info {
        duration_secs: seconds.round() as u64,
        sample_rate: rate,
        bitrate_kbps,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 128 kbps MPEG-1 layer III stereo frame header at 44.1 kHz.
    const FRAME: [u8; 4] = [0xff, 0xfb, 0x90, 0x00];

    #[test]
    fn a_constant_bitrate_mp3_lasts_its_size_over_its_bitrate() {
        let mut head = b"ID3\x03\x00\x00\x00\x00\x00\x0a".to_vec();
        head.extend([0; 10]);
        head.extend(FRAME);
        head.extend([0; 400]);
        let size = 20 + 128_000 / 8 * 180;
        let info = mp3(&head, size).expect("an mp3");
        assert_eq!(info.duration_secs, 180);
        assert_eq!(info.bitrate_kbps, 128);
        assert_eq!(info.sample_rate, 44_100);
    }

    #[test]
    fn a_xing_header_gives_the_frame_count() {
        let mut head = FRAME.to_vec();
        head.extend([0; 32]);
        head.extend(b"Xing");
        head.extend(1u32.to_be_bytes());
        // 7000 frames of 1152 samples at 44.1 kHz is 182.9 seconds.
        head.extend(7000u32.to_be_bytes());
        head.extend([0; 200]);
        let info = mp3(&head, 3_000_000).expect("an mp3");
        assert_eq!(info.duration_secs, 183);
    }

    #[test]
    fn a_vorbis_stream_lasts_its_last_granule_over_its_rate() {
        let mut head = b"OggS".to_vec();
        head.extend([0; 24]);
        head.extend(b"\x01vorbis");
        head.extend(0u32.to_le_bytes());
        head.push(2);
        head.extend(44_100u32.to_le_bytes());
        let mut tail = vec![0; 100];
        tail.extend(b"OggS\x00\x04");
        tail.extend((44_100u64 * 95).to_le_bytes());
        tail.extend([0; 20]);
        let info = ogg(&head, &tail, 1_000_000).expect("an ogg");
        assert_eq!(info.duration_secs, 95);
        assert_eq!(info.sample_rate, 44_100);
    }

    #[test]
    fn anything_else_is_left_unmeasured() {
        assert_eq!(mp3(b"RIFF\x00\x00\x00\x00WAVEfmt ", 1000), None);
    }
}
