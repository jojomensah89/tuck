//! Pure LRCLIB matching, with no I/O or shared state. Implements the algorithm in section 2.2
//! of `docs/research/resources-lyrics-models.md`; see that table for where the scores come from.

use crate::lyrics::{parse_lrc, LyricLine};
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub const ACCEPT_SCORE: i32 = 75;
/// A missing duration can never reach "accept" through search, only "low confidence".
const UNKNOWN_DURATION_CAP: i32 = ACCEPT_SCORE - 1;
const FEATURE_MARKERS: [&str; 5] = ["feat", "feat.", "ft", "ft.", "featuring"];
const ARTIST_JOINERS: [&str; 9] = [
    "&",
    "and",
    "x",
    "with",
    "feat",
    "feat.",
    "ft",
    "ft.",
    "featuring",
];
const WRONG_SONG_MARKERS: [&str; 4] = [
    "karaoke",
    "tribute",
    "made famous by",
    "originally performed by",
];

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LrclibRecord {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
    #[serde(default)]
    pub has_word_sync: bool,
}

/// A track reduced to the parts that decide whether two metadata records are the same song.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    core: String,
    qualifiers: BTreeSet<String>,
    full_artist: String,
    primary: String,
    /// Every comparable artist: the unsplit string, its parts and anyone featured in the title.
    artists: Vec<String>,
    search_title: String,
    search_artist: String,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub record: LrclibRecord,
    pub lines: Vec<LyricLine>,
    pub score: i32,
    pub approximate: bool,
    duration_delta: Option<f64>,
}

enum Tag {
    /// Changes what is being sung; the candidate must carry the same set.
    Identity(Vec<String>),
    Noise,
    Featuring(String),
}

/// Comparison form: letters and digits of every script, with case, Latin-style accents and
/// punctuation folded. Marks that carry meaning in other scripts are kept, so `が` is not `か`.
pub fn normalize_text(value: &str) -> String {
    let value = value.replace('&', " and ");
    let mut normalized = String::with_capacity(value.len());
    let mut pending_space = false;
    for ch in value
        .nfkd()
        .filter(|ch| !('\u{300}'..='\u{36f}').contains(ch))
    {
        // Inside a word, so "Don't" equals "Dont" and "R.E.M." equals "REM".
        if matches!(ch, '\'' | '\u{2018}' | '\u{2019}' | '\u{2bc}' | '`' | '.') {
            continue;
        }
        for lower in ch.to_lowercase() {
            if lower.is_alphanumeric() || is_combining_mark(lower) {
                if pending_space && !normalized.is_empty() {
                    normalized.push(' ');
                }
                normalized.push(lower);
                pending_space = false;
            } else {
                pending_space = true;
            }
        }
    }
    normalized
}

fn is_marker(word: &str, markers: &[&str]) -> bool {
    markers
        .iter()
        .any(|marker| word.eq_ignore_ascii_case(marker))
}

/// Classifies the text of a bracket group or a ` - ` suffix. `None` means it belongs to the title.
fn classify(text: &str) -> Option<Tag> {
    let text = text.trim();
    let mut split = text.splitn(2, char::is_whitespace);
    if let (Some(first), Some(rest)) = (split.next(), split.next()) {
        if is_marker(first, &FEATURE_MARKERS) || first.eq_ignore_ascii_case("with") {
            return Some(Tag::Featuring(rest.to_string()));
        }
    }

    let normalized = normalize_text(text);
    let words: Vec<&str> = normalized.split_whitespace().collect();
    let has = |word: &str| words.contains(&word);
    let mut identity: Vec<String> = [
        "live",
        "acoustic",
        "instrumental",
        "karaoke",
        "cover",
        "demo",
        "slowed",
        "reverb",
    ]
    .into_iter()
    .filter(|word| has(word))
    .map(String::from)
    .collect();
    if normalized.contains("sped up") {
        identity.push("sped up".into());
    }
    if has("remix") {
        // "X Remix" is a different recording from "Y Remix".
        identity.push(if normalized.ends_with("remix") {
            normalized.clone()
        } else {
            "remix".into()
        });
    }
    if let ["part" | "pt", number] = words.as_slice() {
        if number.chars().all(|ch| ch.is_ascii_digit()) {
            identity.push(format!("part {number}"));
        }
    }
    if !identity.is_empty() {
        return Some(Tag::Identity(identity));
    }

    let noise = [
        "remaster",
        "remastered",
        "mono",
        "stereo",
        "explicit",
        "clean",
        "version",
        "deluxe",
    ]
    .into_iter()
    .any(has)
        || normalized.contains("radio edit")
        || normalized.contains("bonus track")
        || (words.first() == Some(&"from") && text.contains(['"', '\u{201c}', '\u{2018}', '\'']));
    noise.then_some(Tag::Noise)
}

/// Splits a title into its core text, identity qualifiers and featured artists.
fn split_title(title: &str) -> (String, BTreeSet<String>, Vec<String>) {
    let mut qualifiers = BTreeSet::new();
    let mut featured = Vec::new();
    let mut apply = |tag: Tag| match tag {
        Tag::Identity(tags) => qualifiers.extend(tags),
        Tag::Noise => {}
        Tag::Featuring(names) => featured.push(names),
    };

    let mut core = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let closer = if rest[open..].starts_with('(') {
            ')'
        } else {
            ']'
        };
        let Some(length) = rest[open + 1..].find(closer) else {
            break;
        };
        let close = open + 1 + length;
        match classify(&rest[open + 1..close]) {
            Some(tag) => {
                apply(tag);
                core.push_str(&rest[..open]);
                core.push(' ');
            }
            None => core.push_str(&rest[..=close]),
        }
        rest = &rest[close + 1..];
    }
    core.push_str(rest);

    // Only an explicit marker counts outside brackets: "With or Without You" keeps its "With".
    let marker = core
        .split_whitespace()
        .skip(1)
        .find(|word| is_marker(word, &FEATURE_MARKERS))
        .and_then(|word| {
            let at = word.as_ptr() as usize - core.as_ptr() as usize;
            Some((at, core.get(at + word.len()..)?.trim().to_string()))
        });
    if let Some((at, names)) = marker {
        apply(Tag::Featuring(names));
        core.truncate(at);
    }

    loop {
        let split = core
            .rsplit_once(" - ")
            .or_else(|| core.rsplit_once(" \u{2013} "));
        let Some((head, suffix)) = split else {
            break;
        };
        let Some(tag) = classify(suffix) else {
            break;
        };
        apply(tag);
        core = head.to_string();
    }
    (core, qualifiers, featured)
}

/// Splits an artist credit on the spec's separators. The unsplit form is kept by the caller,
/// because "Simon & Garfunkel" and "Tyler, The Creator" contain them.
fn split_artists(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for segment in value.split([',', ';', '/']) {
        let mut current: Vec<&str> = Vec::new();
        for word in segment.split_whitespace() {
            if is_marker(word, &ARTIST_JOINERS) {
                parts.push(current.join(" "));
                current.clear();
            } else {
                current.push(word);
            }
        }
        parts.push(current.join(" "));
    }
    parts.retain(|part| !normalize_text(part).is_empty());
    parts
}

impl Track {
    pub fn parse(title: &str, artist: &str) -> Self {
        let (mut raw_core, mut qualifiers, featured) = split_title(title);
        let mut core = normalize_text(&raw_core);
        if core.is_empty() {
            // Nothing but tags, or nothing but punctuation: the whole title is the name.
            raw_core = title.to_string();
            qualifiers.clear();
            core = normalize_text(title);
            if core.is_empty() {
                core = title.trim().to_lowercase();
            }
        }

        let parts = split_artists(artist);
        let full_artist = normalize_text(artist);
        let primary = parts
            .first()
            .map_or_else(|| full_artist.clone(), |part| normalize_text(part));
        let mut artists = vec![full_artist.clone()];
        let featured_parts = featured.iter().flat_map(|names| split_artists(names));
        for name in parts.iter().cloned().chain(featured_parts) {
            let name = normalize_text(&name);
            if !artists.contains(&name) {
                artists.push(name);
            }
        }

        Track {
            core,
            qualifiers,
            full_artist,
            primary,
            artists,
            search_title: raw_core.split_whitespace().collect::<Vec<_>>().join(" "),
            search_artist: parts
                .first()
                .map_or_else(|| artist.trim().to_string(), |part| part.trim().to_string()),
        }
    }

    /// Identifies the song, not the exact seconds. Qualifiers are part of it, so a live track
    /// never picks up the studio lyrics from the cache.
    pub fn cache_key(&self) -> String {
        let qualifiers: Vec<&str> = self.qualifiers.iter().map(String::as_str).collect();
        format!("{}\n{}\n{}", self.primary, self.core, qualifiers.join(","))
    }

    /// Readable text for LRCLIB's full-text search; the folded comparison form is not suitable,
    /// because decomposing Hangul, say, would stop it matching anything.
    pub fn search_query(&self) -> [(&'static str, String); 2] {
        [
            ("track_name", self.search_title.clone()),
            ("artist_name", self.search_artist.clone()),
        ]
    }

    fn has_marker(&self, marker: &str) -> bool {
        self.core.contains(marker) || self.qualifiers.iter().any(|tag| tag.contains(marker))
    }

    fn is_short_title(&self) -> bool {
        let words: Vec<&str> = self.core.split_whitespace().collect();
        words.len() <= 2 && words.iter().all(|word| word.chars().count() <= 5)
    }
}

fn token_similarity(left: &str, right: &str) -> f64 {
    let left: HashSet<&str> = left.split_whitespace().collect();
    let right: HashSet<&str> = right.split_whitespace().collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    left.intersection(&right).count() as f64 / left.union(&right).count() as f64
}

fn title_score(wanted: &Track, candidate: &Track) -> Option<i32> {
    if wanted.core == candidate.core {
        Some(40)
    } else if token_similarity(&wanted.core, &candidate.core) >= 0.9 {
        Some(30)
    } else {
        None
    }
}

fn artist_score(wanted: &Track, candidate: &Track) -> Option<i32> {
    if candidate.artists.contains(&wanted.primary)
        || candidate.artists.contains(&wanted.full_artist)
    {
        return Some(30);
    }
    // Whole words of a multi-word name only: "Drake" must not match "Drake Bell", nor the
    // "Tyler" of "Tyler, The Creator" match "Tyler Childers".
    let padded = format!(" {} ", candidate.full_artist);
    if wanted.primary.contains(' ') && padded.contains(&format!(" {} ", wanted.primary)) {
        return Some(28);
    }
    wanted
        .artists
        .iter()
        .any(|artist| candidate.artists.contains(artist))
        .then_some(10)
}

/// Scores one record against the wanted track. `None` means a wrong-song guard rejected it.
pub fn score_candidate(
    wanted: &Track,
    record: LrclibRecord,
    duration_s: Option<f64>,
) -> Option<Candidate> {
    let candidate = Track::parse(&record.track_name, &record.artist_name);
    if wanted.qualifiers != candidate.qualifiers {
        return None;
    }
    let title_name = normalize_text(&record.track_name);
    if WRONG_SONG_MARKERS
        .iter()
        .any(|marker| title_name.contains(marker) && !wanted.has_marker(marker))
    {
        return None;
    }

    let title = title_score(wanted, &candidate)?;
    let artist = artist_score(wanted, &candidate)?;
    let duration_delta = duration_s.map(|duration| (duration - record.duration).abs());
    let duration = match duration_delta {
        None => 0,
        Some(delta) if delta <= 2.0 => 20,
        Some(delta) if delta <= 5.0 => 14,
        Some(delta) if delta <= 10.0 => 8,
        Some(delta) if delta <= 15.0 => 3,
        // The spec's -20 band can only ever total 55, so nothing above 15 s is shown either way.
        Some(_) => return None,
    };
    if wanted.is_short_title()
        && (wanted.primary != candidate.primary || duration_delta.is_some_and(|delta| delta > 3.0))
    {
        return None;
    }

    let lines = record
        .synced_lyrics
        .as_deref()
        .map(parse_lrc)
        .unwrap_or_default();
    let has_plain = record
        .plain_lyrics
        .as_deref()
        .is_some_and(|plain| !plain.trim().is_empty());
    if lines.is_empty() && !has_plain && !record.instrumental {
        return None;
    }

    let mut score = title + artist + duration;
    if !lines.is_empty() {
        score += 5;
    }
    let runs_past_track = duration_s
        .zip(lines.last())
        .is_some_and(|(duration, last)| last.time_ms as f64 > (duration + 3.0) * 1000.0);
    if runs_past_track {
        score -= 25;
    }
    if duration_s.is_none() {
        score = score.min(UNKNOWN_DURATION_CAP);
    }

    Some(Candidate {
        approximate: title < 40 || artist < 28 || duration_delta.is_none_or(|delta| delta > 2.0),
        record,
        lines,
        score,
        duration_delta,
    })
}

/// `/api/get` already matched title and artist server side, so without a duration to check the
/// low-confidence cap does not apply. The wrong-song guards still do.
fn accept_exact(
    wanted: &Track,
    record: LrclibRecord,
    duration_s: Option<f64>,
) -> Option<Candidate> {
    score_candidate(wanted, record, duration_s)
        .filter(|candidate| candidate.score >= ACCEPT_SCORE || duration_s.is_none())
}

/// Synced beats plain when their scores are within 5, on top of the synced bonus in the score.
fn rank(candidate: &Candidate) -> i32 {
    candidate.score + if candidate.lines.is_empty() { 0 } else { 5 }
}

fn compare(left: &Candidate, right: &Candidate) -> Ordering {
    rank(left)
        .cmp(&rank(right))
        .then_with(|| {
            let delta = |candidate: &Candidate| candidate.duration_delta.unwrap_or(f64::INFINITY);
            delta(right).total_cmp(&delta(left))
        })
        .then_with(|| left.lines.is_empty().cmp(&right.lines.is_empty()).reverse())
        .then_with(|| left.record.has_word_sync.cmp(&right.record.has_word_sync))
}

pub fn best_search_match(
    wanted: &Track,
    records: Vec<LrclibRecord>,
    duration_s: Option<f64>,
) -> Option<Candidate> {
    let mut candidates: Vec<Candidate> = records
        .into_iter()
        .filter_map(|record| score_candidate(wanted, record, duration_s))
        .collect();
    let others_have_lyrics = candidates
        .iter()
        .any(|candidate| !candidate.record.instrumental);
    if others_have_lyrics && !wanted.qualifiers.contains("instrumental") {
        for candidate in &mut candidates {
            if candidate.record.instrumental {
                candidate.score -= 20;
            }
        }
    }
    candidates
        .into_iter()
        .filter(|candidate| candidate.score >= ACCEPT_SCORE)
        .max_by(compare)
}

/// The lookup order: `/api/get`, then `/api/search` when that finds nothing usable. Taking each
/// request as `FnOnce` is what holds a track to at most two. `Ok(None)` is a real "not found",
/// which may be cached; `Err` is a failure, which must not be.
pub fn resolve_match(
    wanted: &Track,
    duration_s: Option<f64>,
    get: impl FnOnce() -> Result<Option<LrclibRecord>, String>,
    search: impl FnOnce() -> Result<Option<Vec<LrclibRecord>>, String>,
) -> Result<Option<Candidate>, String> {
    if let Some(candidate) = get()?.and_then(|record| accept_exact(wanted, record, duration_s)) {
        return Ok(Some(candidate));
    }
    Ok(search()?.and_then(|records| best_search_match(wanted, records, duration_s)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn record(title: &str, artist: &str, duration: f64) -> LrclibRecord {
        LrclibRecord {
            id: 42,
            track_name: title.into(),
            artist_name: artist.into(),
            duration,
            instrumental: false,
            plain_lyrics: Some("Line".into()),
            synced_lyrics: Some("[00:01.00]Line".into()),
            has_word_sync: false,
        }
    }

    fn plain(title: &str, artist: &str, duration: f64) -> LrclibRecord {
        LrclibRecord {
            synced_lyrics: None,
            ..record(title, artist, duration)
        }
    }

    fn score(
        title: &str,
        artist: &str,
        duration: Option<f64>,
        record: LrclibRecord,
    ) -> Option<i32> {
        score_candidate(&Track::parse(title, artist), record, duration).map(|found| found.score)
    }

    fn accepted(title: &str, artist: &str, duration: f64, record: LrclibRecord) -> bool {
        score(title, artist, Some(duration), record).is_some_and(|value| value >= ACCEPT_SCORE)
    }

    #[test]
    fn reported_track_is_accepted_and_flagged_approximate() {
        let wanted = Track::parse("Big Thug Boys", "Babyboy AV");
        let found = best_search_match(
            &wanted,
            vec![
                record("Big Thug Boys (feat. Jireel)", "Babyboy AV", 163.0),
                LrclibRecord {
                    id: 7,
                    ..record("Big Thug Boys (feat. Jireel)", "Babyboy AV", 164.0)
                },
                record(
                    "Big Thug Boys (feat. Jireel)",
                    "Babyboy AV feat. Jireel",
                    163.0,
                ),
            ],
            Some(171.0),
        )
        .expect("the reported track must match");
        assert_eq!(found.score, 83, "worked example in the spec");
        assert_eq!(found.record.id, 7, "ties go to the smallest duration gap");
        assert!(found.approximate);
    }

    #[test]
    fn same_title_by_a_different_artist_is_rejected() {
        for (wanted, other) in [
            ("Artist One", "Artist Two"),
            ("Drake", "Drake Bell"),
            ("Tyler, The Creator", "Tyler Childers"),
            ("AV", "Dave"),
        ] {
            assert_eq!(
                score(
                    "Our Way Home",
                    wanted,
                    Some(180.0),
                    record("Our Way Home", other, 180.0)
                ),
                None,
                "{wanted} must not match {other}"
            );
        }
    }

    #[test]
    fn remaster_is_noise_but_live_is_a_different_recording() {
        assert!(accepted(
            "Northern Lights (2011 Remaster)",
            "Aster",
            200.0,
            record("Northern Lights - Remastered 2020", "Aster", 201.0),
        ));
        assert!(accepted(
            "Northern Lights - Single Version",
            "Aster",
            200.0,
            record("Northern Lights [Mono]", "Aster", 201.0),
        ));
        for (ours, theirs) in [
            ("Northern Lights (Live)", "Northern Lights"),
            ("Northern Lights", "Northern Lights - Live at Wembley"),
            ("Northern Lights (Acoustic)", "Northern Lights (Live)"),
            (
                "Northern Lights (Tiesto Remix)",
                "Northern Lights (Alok Remix)",
            ),
            ("Northern Lights (Pt. 1)", "Northern Lights (Part 2)"),
            ("Northern Lights", "Northern Lights (Karaoke Version)"),
            ("Northern Lights (Sped Up)", "Northern Lights"),
        ] {
            assert_eq!(
                score(ours, "Aster", Some(200.0), record(theirs, "Aster", 200.0)),
                None,
                "{ours} must not match {theirs}"
            );
        }
        assert!(accepted(
            "Northern Lights (Live)",
            "Aster",
            200.0,
            record("Northern Lights - Live at Wembley", "Aster", 200.0),
        ));
        assert!(accepted(
            "Northern Lights (Pt. 2)",
            "Aster",
            200.0,
            record("Northern Lights (Part 2)", "Aster", 200.0),
        ));
    }

    #[test]
    fn multi_artist_credits_match_on_the_primary_artist() {
        for ours in [
            "A, B",
            "A & B",
            "A feat. B",
            "A x B",
            "A; B",
            "A/B",
            "A with B",
        ] {
            for theirs in ["A", "A, B", "A & B", "A feat. B", "B, A"] {
                assert!(
                    accepted(
                        "Shared Song Title",
                        ours,
                        190.0,
                        record("Shared Song Title", theirs, 190.0)
                    ),
                    "{ours} should match {theirs}"
                );
            }
        }
    }

    #[test]
    fn a_partial_artist_overlap_only_passes_with_everything_else_exact() {
        let wanted = Track::parse("Shared Song Title", "A, B");
        let found = score_candidate(
            &wanted,
            record("Shared Song Title", "B", 190.0),
            Some(190.0),
        )
        .expect("B is credited on both");
        assert_eq!(found.score, 75);
        assert!(found.approximate);
        assert_eq!(
            score(
                "Shared Song Title",
                "A, B",
                Some(190.0),
                plain("Shared Song Title", "B", 190.0)
            ),
            Some(70),
            "without synced lyrics it stays below the threshold"
        );
    }

    #[test]
    fn names_containing_separators_keep_their_unsplit_form() {
        for artist in [
            "Simon & Garfunkel",
            "Earth, Wind & Fire",
            "Tyler, The Creator",
        ] {
            assert_eq!(
                score(
                    "Long Song Title",
                    artist,
                    Some(200.0),
                    record("Long Song Title", artist, 200.0)
                ),
                Some(95),
                "{artist}"
            );
        }
    }

    #[test]
    fn title_words_are_not_mistaken_for_tags() {
        let bare = Track::parse("With or Without You", "U2");
        assert_eq!(bare.core, "with or without you");
        assert_eq!(Track::parse("Stay With Me", "Sam").core, "stay with me");
        assert_eq!(Track::parse("Song (with Friend)", "Sam").core, "song");
        assert_eq!(Track::parse("Song ft. Friend", "Sam").core, "song");
        assert!(Track::parse("Song ft. Friend", "Sam")
            .artists
            .contains(&"friend".to_string()));
        assert_eq!(Track::parse("Echo (Monolith)", "Sam").core, "echo monolith");
        assert_eq!(
            Track::parse("Echo (Inversion)", "Sam").core,
            "echo inversion"
        );
        assert_eq!(
            Track::parse("(I Can't Get No) Satisfaction", "Sam").core,
            "i cant get no satisfaction"
        );
        assert_eq!(Track::parse("(Live)", "Sam").core, "live");
        assert_eq!(Track::parse("...", "Sam").core, "...");
        assert_eq!(
            score(
                "Stay With Me",
                "Sam",
                Some(180.0),
                record("Stay", "Sam", 180.0)
            ),
            None,
            "a different song by the same artist"
        );
    }

    #[test]
    fn non_latin_text_is_compared_in_its_own_script() {
        assert!(accepted(
            "Beyoncé 東京の夜",
            "Алиса",
            210.0,
            record("Beyonce 東京の夜", "Алиса", 210.0)
        ));
        assert!(accepted(
            "봄날 (Spring Day)",
            "방탄소년단",
            274.0,
            record("봄날 (Spring Day)", "방탄소년단", 274.0)
        ));
        assert_eq!(normalize_text("東京"), "東京");
        assert_eq!(normalize_text("Beyoncé & Björk"), "beyonce and bjork");
        assert_eq!(normalize_text("Don\u{2019}t  Stop!"), "dont stop");
        assert_ne!(
            normalize_text("はな"),
            normalize_text("ばな"),
            "kana marks change the word"
        );
        assert_eq!(
            score("東京", "Алиса", Some(210.0), record("大阪", "Алиса", 210.0)),
            None
        );
    }

    #[test]
    fn search_keeps_the_readable_form_of_the_query() {
        let wanted = Track::parse(
            "봄날 (feat. Someone) - Remastered 2020",
            "방탄소년단 & Someone",
        );
        let [title, artist] = wanted.search_query();
        assert_eq!(title.1, "봄날");
        assert_eq!(artist.1, "방탄소년단");
    }

    #[test]
    fn missing_duration_trusts_an_exact_lookup_but_not_a_search_hit() {
        let wanted = Track::parse("Three Word Title", "Exact Artist");
        let exact = accept_exact(
            &wanted,
            record("Three Word Title", "Exact Artist", 999.0),
            None,
        )
        .expect("/api/get matched title and artist itself");
        assert!(exact.approximate, "the timing could not be checked");
        assert_eq!(
            best_search_match(
                &wanted,
                vec![record("Three Word Title", "Exact Artist", 999.0)],
                None
            )
            .map(|found| found.score),
            None,
            "unknown duration caps a search hit at low confidence"
        );
        assert_eq!(
            score(
                "Three Word Title",
                "Exact Artist",
                None,
                record("Three Word Title", "Exact Artist", 999.0)
            ),
            Some(UNKNOWN_DURATION_CAP)
        );
    }

    #[test]
    fn short_titles_need_the_same_artist_and_a_close_duration() {
        assert!(accepted(
            "Home",
            "Aster",
            200.0,
            record("Home", "Aster", 202.0)
        ));
        assert_eq!(
            score("Home", "Aster", Some(200.0), record("Home", "Aster", 205.0)),
            None
        );
        assert_eq!(
            score(
                "22",
                "Aster",
                Some(200.0),
                record("22", "Comet, Aster", 200.0)
            ),
            None
        );
        assert!(
            accepted(
                "Dancing Queen",
                "Aster",
                200.0,
                record("Dancing Queen", "Aster", 204.0)
            ),
            "two longer words are not a short title"
        );
    }

    #[test]
    fn duration_outside_the_tolerance_is_rejected() {
        let title = "Long Song Title";
        assert_eq!(
            score(title, "Aster", Some(200.0), record(title, "Aster", 214.0)),
            Some(78)
        );
        assert_eq!(
            score(title, "Aster", Some(200.0), record(title, "Aster", 216.0)),
            None
        );
    }

    #[test]
    fn tribute_and_karaoke_uploads_are_rejected() {
        let title = "Long Song Title";
        for theirs in [
            "Long Song Title (Tribute to Aster)",
            "Long Song Title Karaoke",
            "Long Song Title (Made Famous by Aster)",
            "Long Song Title - Originally Performed by Aster",
        ] {
            assert_eq!(
                score(title, "Aster", Some(200.0), record(theirs, "Aster", 200.0)),
                None,
                "{theirs}"
            );
        }
    }

    #[test]
    fn lyrics_running_past_the_track_are_rejected() {
        let long = LrclibRecord {
            synced_lyrics: Some("[00:01.00]Start\n[05:00.00]Far past the end".into()),
            ..record("Long Song Title", "Aster", 200.0)
        };
        assert!(!accepted("Long Song Title", "Aster", 200.0, long));
    }

    #[test]
    fn a_record_with_no_lyrics_never_wins() {
        let empty = LrclibRecord {
            plain_lyrics: Some("  ".into()),
            synced_lyrics: None,
            ..record("Long Song Title", "Aster", 200.0)
        };
        let usable = plain("Long Song Title", "Aster", 196.0);
        let wanted = Track::parse("Long Song Title", "Aster");
        let found = best_search_match(&wanted, vec![empty, usable], Some(200.0)).expect("usable");
        assert_eq!(found.record.duration, 196.0);
    }

    #[test]
    fn synced_wins_a_close_score_but_not_a_distant_one() {
        let wanted = Track::parse("Long Song Title", "Aster");
        let close = best_search_match(
            &wanted,
            vec![
                plain("Long Song Title", "Aster", 196.0),
                record("Long Song Title", "Aster", 208.0),
            ],
            Some(200.0),
        )
        .expect("both pass");
        assert!(!close.lines.is_empty(), "84 plain against 83 synced");
        let distant = best_search_match(
            &wanted,
            vec![
                plain("Long Song Title", "Aster", 200.0),
                record("Long Song Title", "Aster", 212.0),
            ],
            Some(200.0),
        )
        .expect("both pass");
        assert!(distant.lines.is_empty(), "90 plain against 78 synced");
    }

    #[test]
    fn instrumental_records_are_marked_down_only_beside_real_lyrics() {
        let instrumental = LrclibRecord {
            instrumental: true,
            plain_lyrics: None,
            synced_lyrics: None,
            ..record("Long Song Title", "Aster", 200.0)
        };
        let wanted = Track::parse("Long Song Title", "Aster");
        let alone = accept_exact(&wanted, instrumental.clone(), Some(200.0)).expect("exact record");
        assert!(alone.record.instrumental);
        let found = best_search_match(
            &wanted,
            vec![instrumental, plain("Long Song Title", "Aster", 203.0)],
            Some(200.0),
        )
        .expect("the record with lyrics wins");
        assert!(!found.record.instrumental);
    }

    #[test]
    fn exact_duration_and_identity_are_not_flagged_approximate() {
        let wanted = Track::parse("Song Title Here (feat. Friend)", "Aster");
        let found = score_candidate(
            &wanted,
            record("Song Title Here", "Aster", 180.0),
            Some(181.0),
        )
        .expect("same song");
        assert!(!found.approximate);
    }

    #[test]
    fn exact_hit_stops_before_search() {
        let searched = Cell::new(false);
        let found = resolve_match(
            &Track::parse("Long Song Title", "Aster"),
            Some(200.0),
            || Ok(Some(record("Long Song Title", "Aster", 200.0))),
            || {
                searched.set(true);
                Ok(None)
            },
        )
        .expect("no failure")
        .expect("match");
        assert!(!found.approximate);
        assert!(!searched.get());
    }

    #[test]
    fn not_found_falls_back_to_search_and_empty_search_is_a_real_miss() {
        let wanted = Track::parse("Big Thug Boys", "Babyboy AV");
        let found = resolve_match(
            &wanted,
            Some(171.0),
            || Ok(None),
            || {
                Ok(Some(vec![record(
                    "Big Thug Boys (feat. Jireel)",
                    "Babyboy AV",
                    163.0,
                )]))
            },
        )
        .expect("no failure");
        assert!(found.is_some());
        let missing = resolve_match(&wanted, Some(171.0), || Ok(None), || Ok(Some(Vec::new())));
        assert!(matches!(missing, Ok(None)));
        let missing = resolve_match(&wanted, Some(171.0), || Ok(None), || Ok(None));
        assert!(matches!(missing, Ok(None)));
    }

    #[test]
    fn a_wrong_exact_record_falls_back_to_search() {
        let found = resolve_match(
            &Track::parse("Long Song Title", "Aster"),
            Some(200.0),
            || Ok(Some(record("Unrelated Thing", "Aster", 200.0))),
            || Ok(Some(vec![record("Long Song Title", "Aster", 200.0)])),
        )
        .expect("no failure")
        .expect("search result");
        assert_eq!(found.record.track_name, "Long Song Title");
    }

    #[test]
    fn a_failed_request_stops_the_lookup_and_is_not_a_miss() {
        let searched = Cell::new(false);
        let wanted = Track::parse("Long Song Title", "Aster");
        let result = resolve_match(
            &wanted,
            Some(200.0),
            || Err("rate limited".to_string()),
            || {
                searched.set(true);
                Ok(None)
            },
        );
        assert!(result.is_err());
        assert!(!searched.get());
        let result = resolve_match(
            &wanted,
            Some(200.0),
            || Ok(None),
            || Err("offline".to_string()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn cache_key_ignores_noise_and_seconds_but_not_identity() {
        let key = |title, artist| Track::parse(title, artist).cache_key();
        assert_eq!(
            key("Yellow (2011 Remaster)", " Coldplay "),
            key("yellow", "coldplay")
        );
        assert_eq!(
            key("Yellow (feat. Someone)", "Coldplay, Someone"),
            key("Yellow", "Coldplay")
        );
        assert_ne!(key("Yellow (Live)", "Coldplay"), key("Yellow", "Coldplay"));
    }
}
