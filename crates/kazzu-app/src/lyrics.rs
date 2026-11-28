//! Reads time-stamped LRC sidecars accompanying local audio files.

use std::io::Read;
use std::path::Path;

pub(crate) fn read_lrc(track: &Path) -> Vec<(i32, String)> {
    let Ok(file) = std::fs::File::open(track.with_extension("lrc")) else {
        return Vec::new();
    };
    let mut text = String::new();
    if file.take(512 * 1024).read_to_string(&mut text).is_err() {
        return Vec::new();
    }
    let offset = text
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("[offset:")?
                .strip_suffix(']')?
                .trim()
                .parse::<i32>()
                .ok()
        })
        .next()
        .unwrap_or_default();

    let mut timed = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let mut remaining = line.trim();
        let mut times = Vec::new();
        while let Some((tag, text)) = remaining.strip_prefix('[').and_then(|s| s.split_once(']')) {
            let Some(ms) = milliseconds(tag) else { break };
            times.push(ms.saturating_add(offset).max(0));
            remaining = text;
        }
        timed.extend(
            times
                .into_iter()
                .map(|ms| (ms, remaining.trim().to_owned())),
        );
    }
    timed.sort_by_key(|(ms, _)| *ms);
    timed.dedup();
    timed
}

fn milliseconds(timestamp: &str) -> Option<i32> {
    let (minutes, seconds) = timestamp.split_once(':')?;
    let minutes: u32 = minutes.parse().ok()?;
    let (seconds, fraction, digits) = match seconds.split_once('.') {
        Some((seconds, fraction)) if (1..=3).contains(&fraction.len()) => {
            (seconds, fraction.parse::<u32>().ok()?, fraction.len())
        }
        Some(_) => return None,
        None => (seconds, 0, 3),
    };
    let seconds: u32 = seconds.parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let ms = minutes
        .checked_mul(60_000)?
        .checked_add(seconds * 1000 + fraction * 10_u32.pow(3 - digits as u32))?;
    i32::try_from(ms).ok()
}
