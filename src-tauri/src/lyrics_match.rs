//! Pure LRCLIB matching, with no I/O or shared state. Implements the algorithm in section 2.2
//! of `docs/research/resources-lyrics-models.md`; see that table for where the scores come from.

use crate::lyrics::{parse_lrc, LyricLine};
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub const ACCEPT_SCORE: i32 = 75;
/// A duration that cannot be compared never reaches "accept" through search, only "low confidence".
const UNKNOWN_DURATION_CAP: i32 = ACCEPT_SCORE - 1;
const FEATURE_MARKERS: [&str; 5] = ["feat", "feat.", "ft", "ft.", "featuring"];
/// Bare in a title these can be ordinary words, as in "Six Ft Under".
const AMBIGUOUS_MARKERS: [&str; 2] = ["ft", "ft."];
const WRONG_ARTIST_MARKERS: [&str; 4] = [
    "karaoke",
    "tribute",
    "made famous by",
    "originally performed by",
];
const NOISE_WORDS: [&str; 7] = [
    "remaster",
    "remastered",
    "mono",
    "stereo",
    "explicit",
    "clean",
    "deluxe",
];
/// These leave the recording the same; any other "<word> version", edit or mix is a different one.
const NEUTRAL_VERSIONS: [&str; 6] = ["single", "album", "radio", "original", "soundtrack", "lp"];
const VIDEO_TAG_WORDS: [&str; 10] = [
    "official",
    "music",
    "video",
    "audio",
    "lyric",
    "lyrics",
    "visualizer",
    "visualiser",
    "hd",
    "hq",
];
/// A bracketed subtitle with one of these is another arrangement, not a longer name for the song.
const VARIANT_SUBTITLE_WORDS: [&str; 12] = [
    "dub",
    "piano",
    "reprise",
    "interlude",
    "intro",
    "outro",
    "skit",
    "acapella",
    "cappella",
    "orchestral",
    "unplugged",
    "extended",
];
/// Titles use these separators interchangeably.
const DASHES: [&str; 3] = [" - ", " \u{2013} ", " \u{2014} "];

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LrclibRecord {
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

/// A track reduced to the parts that decide whether two metadata records are the same song.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    core: String,
    qualifiers: BTreeSet<String>,
    /// Names after "(with ...)" or a bare "ft" that the credits do not confirm. They may be title
    /// words, so a match without them has to agree on the artist and the duration too.
    unverified: Vec<String>,
    /// The title without a trailing unclassified subtitle: `love nwantiti (ah ah ah)`.
    subtitle_base: Option<String>,
    full_artist: String,
    primary: String,
    /// Who the credit names, without guests: "A & B feat. C" is {a, b}.
    leads: BTreeSet<String>,
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
    /// The timing could not be confirmed: the lengths differ, or one is unknown.
    pub approximate: bool,
    /// Same song, but the credit or a subtitle differs. Says nothing about the timing.
    pub credit_differs: bool,
    duration_delta: Option<f64>,
}

enum Tag {
    /// Changes what is being sung; the candidate must carry the same set.
    Identity(Vec<String>),
    Noise,
    /// Noise that also says this is a video upload, where the title usually names the artist.
    VideoNoise,
    Featuring(String),
    PossibleCredit(String),
}

struct TitleParts {
    core: String,
    qualifiers: BTreeSet<String>,
    featured: Vec<String>,
    possible_credits: Vec<String>,
    from_video: bool,
}

fn is_latin(ch: char) -> bool {
    matches!(ch, 'a'..='z' | 'A'..='Z' | '\u{c0}'..='\u{24f}' | '\u{1e00}'..='\u{1eff}')
}

/// Accents that don't change the word: on Latin and Greek letters, and `ё`, usually written `е`.
/// Elsewhere a mark does, as in `мой` and `мои` or `はな` and `ばな`.
fn is_benign_mark(base: char, mark: char) -> bool {
    is_latin(base)
        || ('\u{370}'..='\u{3ff}').contains(&base)
        || (matches!(base, 'е' | 'Е') && mark == '\u{308}')
}

/// Comparison form: letters and digits of every script, with case, punctuation and benign accents
/// folded.
pub fn normalize_text(value: &str) -> String {
    let value = value.replace('&', " and ");
    let mut normalized = String::with_capacity(value.len());
    let mut pending_space = false;
    let mut base = '\0';
    for ch in value.nfkd() {
        if is_combining_mark(ch) {
            if is_benign_mark(base, ch) {
                continue;
            }
        } else {
            base = ch;
        }
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

fn is_guest_joiner(word: &str) -> bool {
    is_marker(word, &FEATURE_MARKERS) || word.eq_ignore_ascii_case("with")
}

fn is_artist_joiner(word: &str) -> bool {
    is_guest_joiner(word) || is_marker(word, &["&", "and", "x"])
}

/// The tag a "... version", "... edit" or "... mix" adds, or `None` when the recording is the same.
fn cut_identity(words: &[&str], tail: &str) -> Option<String> {
    let kind: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| {
            *word != tail
                && !NOISE_WORDS.contains(word)
                && !NEUTRAL_VERSIONS.contains(word)
                && !word.chars().all(|ch| ch.is_ascii_digit())
        })
        .collect();
    (!kind.is_empty()).then(|| format!("{} {tail}", kind.join(" ")))
}

fn is_video_tag(words: &[&str]) -> bool {
    words.iter().all(|word| VIDEO_TAG_WORDS.contains(word))
        && words
            .iter()
            .any(|word| !matches!(*word, "official" | "music" | "hd" | "hq"))
}

fn is_year(words: &[&str]) -> bool {
    matches!(words, [word] if word.len() == 4
        && word.chars().all(|ch| ch.is_ascii_digit())
        && matches!(&word[..2], "19" | "20"))
}

/// Classifies the text of a bracket group or a ` - ` suffix. `None` means it belongs to the title.
fn classify(text: &str, is_suffix: bool) -> Option<Tag> {
    let text = text.trim();
    if let Some((first, rest)) = text.split_once(char::is_whitespace) {
        if is_marker(first, &FEATURE_MARKERS) {
            return Some(Tag::Featuring(rest.to_string()));
        }
        if first.eq_ignore_ascii_case("with") {
            return Some(Tag::PossibleCredit(rest.to_string()));
        }
    }

    let normalized = normalize_text(text);
    let words: Vec<&str> = normalized.split_whitespace().collect();
    // In brackets "from" needs its quotes, because "(From Me To You)" can be part of a title.
    if words.first() == Some(&"from")
        && words.len() > 1
        && (is_suffix || text.contains(['"', '\u{201c}', '\u{2018}', '\'']))
    {
        return Some(Tag::Noise);
    }
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
    if normalized.contains("bonus track") || normalized.contains("radio edit") || has("edition") {
        return Some(Tag::Noise);
    }

    let tail = ["edit", "mix"]
        .into_iter()
        .find(|tail| words.last() == Some(tail))
        .or(has("version").then_some("version"));
    if let Some(tail) = tail {
        return Some(
            cut_identity(&words, tail).map_or(Tag::Noise, |kind| Tag::Identity(vec![kind])),
        );
    }
    if !words.is_empty() && is_video_tag(&words) {
        return Some(Tag::VideoNoise);
    }
    (NOISE_WORDS.into_iter().any(has) || is_year(&words)).then_some(Tag::Noise)
}

/// Splits a title into its core text, identity qualifiers and credited names.
fn split_title(title: &str) -> TitleParts {
    let mut qualifiers = BTreeSet::new();
    let mut featured = Vec::new();
    let mut possible_credits = Vec::new();
    let mut from_video = false;
    let mut apply = |tag: Tag| match tag {
        Tag::Identity(tags) => qualifiers.extend(tags),
        Tag::Noise => {}
        Tag::VideoNoise => from_video = true,
        Tag::Featuring(names) => featured.push(names),
        Tag::PossibleCredit(names) => possible_credits.push(names),
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
        match classify(&rest[open + 1..close], false) {
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
            let names = core.get(at + word.len()..)?.trim().to_string();
            Some((at, is_marker(word, &AMBIGUOUS_MARKERS), names))
        });
    if let Some((at, ambiguous, names)) = marker {
        apply(if ambiguous {
            Tag::PossibleCredit(names)
        } else {
            Tag::Featuring(names)
        });
        core.truncate(at);
    }

    while let Some((head, suffix)) = DASHES
        .iter()
        .filter_map(|dash| core.rsplit_once(dash))
        .max_by_key(|(head, _)| head.len())
    {
        let Some(tag) = classify(suffix, true) else {
            break;
        };
        apply(tag);
        core = head.to_string();
    }
    TitleParts {
        core,
        qualifiers,
        featured,
        possible_credits,
        from_video,
    }
}

/// Splits an artist credit on the spec's separators. The unsplit form is kept by the caller,
/// because "Simon & Garfunkel" and "Tyler, The Creator" contain them.
fn split_artists(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for segment in value.split([',', ';', '/']) {
        let mut current: Vec<&str> = Vec::new();
        for word in segment.split_whitespace() {
            if is_artist_joiner(word) {
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

/// The credit before any guest: "A & B feat. C" is {a, b}.
fn lead_artists(artist: &str) -> BTreeSet<String> {
    let guest = artist
        .split_whitespace()
        .find(|word| is_guest_joiner(word))
        .map(|word| word.as_ptr() as usize - artist.as_ptr() as usize);
    split_artists(&artist[..guest.unwrap_or(artist.len())])
        .iter()
        .map(|part| normalize_text(part))
        .collect()
}

/// Browser sessions report the channel: "RemaVEVO" or "Rema - Topic".
fn clean_channel_name(artist: &str) -> &str {
    let artist = artist.trim();
    let artist = artist.strip_suffix(" - Topic").unwrap_or(artist);
    let cut = artist.len().saturating_sub(4);
    match artist.get(cut..) {
        Some(tail) if cut > 0 && tail.eq_ignore_ascii_case("vevo") => artist[..cut].trim_end(),
        _ => artist,
    }
}

/// Finds the artist in `Artist - Title`. The head must name the credited artist, unless a tag such
/// as "(Official Video)" shows this is an upload whose channel may be anyone.
fn leading_artist(core: &str, artist: &str, from_video: bool) -> Option<(String, String)> {
    let (head, rest) = DASHES
        .iter()
        .filter_map(|dash| core.split_once(dash))
        .min_by_key(|(head, _)| head.len())?;
    if normalize_text(head).is_empty() || normalize_text(rest).is_empty() {
        return None;
    }
    let credited: Vec<String> = split_artists(artist)
        .iter()
        .map(|part| normalize_text(part))
        .collect();
    let named = normalize_text(head) == normalize_text(artist)
        || split_artists(head)
            .iter()
            .any(|part| credited.contains(&normalize_text(part)));
    if named {
        Some((artist.to_string(), rest.to_string()))
    } else {
        from_video.then(|| (head.trim().to_string(), rest.to_string()))
    }
}

/// `Song Pt. 2` is `Song (Part 2)`: returns the title without its part and the part as a tag.
fn split_part(core: &str) -> (String, Option<String>) {
    let mut words: Vec<&str> = core.split_whitespace().collect();
    if let [.., "part" | "pt", number] = words.as_slice() {
        if words.len() > 2 && number.chars().all(|ch| ch.is_ascii_digit()) {
            let part = format!("part {number}");
            words.truncate(words.len() - 2);
            return (words.join(" "), Some(part));
        }
    }
    (core.to_string(), None)
}

/// The title before a trailing bracket of two or more ordinary words. One word is too likely to
/// be an arrangement (`Song (Dub)`), so it stays part of the name.
fn subtitle_base(core: &str) -> Option<String> {
    let trimmed = core.trim_end();
    let open = match trimmed.chars().last()? {
        ')' => trimmed.rfind('(')?,
        ']' => trimmed.rfind('[')?,
        _ => return None,
    };
    let subtitle = normalize_text(&trimmed[open + 1..trimmed.len() - 1]);
    let words: Vec<&str> = subtitle.split_whitespace().collect();
    let variant = words
        .iter()
        .any(|word| VARIANT_SUBTITLE_WORDS.contains(word))
        || WRONG_ARTIST_MARKERS
            .iter()
            .any(|marker| format!(" {subtitle} ").contains(&format!(" {marker} ")));
    let base = normalize_text(&trimmed[..open]);
    (words.len() >= 2 && !variant && !base.is_empty()).then_some(base)
}

/// The folded form, or the literal text when nothing survives folding, as for the band `!!!`.
fn comparable(value: &str) -> String {
    let normalized = normalize_text(value);
    if normalized.is_empty() {
        value.trim().to_lowercase()
    } else {
        normalized
    }
}

impl Track {
    pub fn parse(title: &str, artist: &str) -> Self {
        let parts = split_title(title);
        let (mut raw_core, mut qualifiers, possible_credits) =
            if normalize_text(&parts.core).is_empty() {
                // Nothing but tags, or nothing but punctuation: the whole title is the name.
                (title.to_string(), BTreeSet::new(), Vec::new())
            } else {
                (parts.core, parts.qualifiers, parts.possible_credits)
            };
        let mut artist = clean_channel_name(artist).to_string();
        if let Some((head, rest)) = leading_artist(&raw_core, &artist, parts.from_video) {
            artist = head;
            raw_core = rest;
        }
        let artist = artist.as_str();

        let credits = split_artists(artist);
        let full_artist = comparable(artist);
        let primary = credits
            .first()
            .map_or_else(|| full_artist.clone(), |part| normalize_text(part));
        let names = |raw: &[String]| -> Vec<String> {
            raw.iter()
                .flat_map(|names| split_artists(names))
                .map(|name| normalize_text(&name))
                .collect()
        };
        let mut artists: Vec<String> = Vec::new();
        let credited = credits.iter().map(|part| normalize_text(part));
        for name in [full_artist.clone()]
            .into_iter()
            .chain(credited)
            .chain(names(&parts.featured))
        {
            if !name.is_empty() && !artists.contains(&name) {
                artists.push(name);
            }
        }
        let mut unverified = names(&possible_credits);
        unverified.retain(|name| !artists.contains(name));

        let (core, part) = split_part(&comparable(&raw_core));
        qualifiers.extend(part);

        Track {
            core,
            qualifiers,
            unverified,
            subtitle_base: subtitle_base(&raw_core),
            full_artist,
            primary,
            leads: lead_artists(artist),
            artists,
            search_title: raw_core.split_whitespace().collect::<Vec<_>>().join(" "),
            search_artist: credits
                .first()
                .map_or_else(|| artist.trim().to_string(), |part| part.trim().to_string()),
        }
    }

    /// Identifies this recording. The duration is part of it: an artist can have several songs
    /// with one title, and a player can report the duration late, so an answer found for one
    /// length must never be served for another.
    pub fn cache_key(&self, duration_s: Option<f64>) -> String {
        let qualifiers: Vec<&str> = self.qualifiers.iter().map(String::as_str).collect();
        let duration = duration_s.map_or_else(
            || "unknown".to_string(),
            |duration| (duration.round() as i64).to_string(),
        );
        format!(
            "{}\n{}\n{}\n{}\n{duration}",
            self.primary,
            self.core,
            qualifiers.join(","),
            self.unverified.join(","),
        )
    }

    /// Readable text for LRCLIB's full-text search; the folded comparison form is not suitable,
    /// because decomposing Hangul, say, would stop it matching anything.
    pub fn search_query(&self) -> [(&'static str, String); 2] {
        [
            ("track_name", self.search_title.clone()),
            ("artist_name", self.search_artist.clone()),
        ]
    }

    fn is_short_title(&self) -> bool {
        let words: Vec<&str> = self.core.split_whitespace().collect();
        words.len() <= 2 && words.iter().all(|word| word.nfc().count() <= 5)
    }

    fn explains(&self, names: &[String]) -> bool {
        names
            .iter()
            .all(|name| self.artists.contains(name) || self.unverified.contains(name))
    }
}

/// How a candidate's title relates to ours, given what else agrees.
enum TitleMatch {
    Same,
    /// Probably the same song under another credit or subtitle; only trusted with exact agreement.
    Corroborated,
}

/// Word order and repeats matter, so only the same title counts: "Money Money Money" is not
/// "Money", nor "Me and You" "You and Me". Two looser cases need the rest to agree. A name removed
/// as a possible credit needs the same primary artist and a length within 2 s, so "STAY (with
/// Justin Bieber)" finds "STAY", while another song called "Run" will rarely match "Run (With Me)"
/// on both. A trailing subtitle needs the exact credit and a length within 2 s.
fn title_match(
    wanted: &Track,
    candidate: &Track,
    exact_artist: bool,
    duration_delta: Option<f64>,
) -> Option<TitleMatch> {
    let close = duration_delta.is_some_and(|delta| delta <= 2.0);
    if wanted.core == candidate.core {
        if wanted.explains(&candidate.unverified) && candidate.explains(&wanted.unverified) {
            return Some(TitleMatch::Same);
        }
        return (close && wanted.primary == candidate.primary).then_some(TitleMatch::Corroborated);
    }
    let subtitled = wanted.subtitle_base.as_deref() == Some(candidate.core.as_str())
        || candidate.subtitle_base.as_deref() == Some(wanted.core.as_str());
    (subtitled && close && exact_artist && !wanted.is_short_title() && !candidate.is_short_title())
        .then_some(TitleMatch::Corroborated)
}

/// Points and whether the credit is really the same. A guest spot or a fragment of a name scores
/// as the spec says, since LRCLIB files real lyrics under `Tyler` and `AC`, but gets flagged.
fn artist_score(wanted: &Track, candidate: &Track) -> Option<(i32, bool)> {
    if wanted.full_artist.is_empty() {
        return None;
    }
    if wanted.full_artist == candidate.full_artist
        || (!wanted.leads.is_empty() && wanted.leads == candidate.leads)
    {
        return Some((30, true));
    }
    if candidate.artists.contains(&wanted.primary) {
        return Some((30, false));
    }
    // Whole words of a multi-word name only: "Drake" must not match "Drake Bell", nor the
    // "Tyler" of "Tyler, The Creator" match "Tyler Childers".
    let padded = format!(" {} ", candidate.full_artist);
    if wanted.primary.contains(' ') && padded.contains(&format!(" {} ", wanted.primary)) {
        return Some((28, false));
    }
    wanted
        .artists
        .iter()
        .any(|artist| candidate.artists.contains(artist))
        .then_some((10, false))
}

fn has_wrong_artist_marker(wanted: &Track, candidate: &Track) -> bool {
    let contains =
        |artist: &str, marker: &str| format!(" {artist} ").contains(&format!(" {marker} "));
    WRONG_ARTIST_MARKERS.iter().any(|marker| {
        contains(&candidate.full_artist, marker) && !contains(&wanted.full_artist, marker)
    })
}

/// Scores one record against the wanted track. `None` means a wrong-song guard rejected it.
pub fn score_candidate(
    wanted: &Track,
    record: LrclibRecord,
    duration_s: Option<f64>,
) -> Option<Candidate> {
    let candidate = Track::parse(&record.track_name, &record.artist_name);
    if wanted.qualifiers != candidate.qualifiers || has_wrong_artist_marker(wanted, &candidate) {
        return None;
    }

    let (artist, exact_artist) = artist_score(wanted, &candidate)?;
    let duration_delta = duration_s
        .zip(record.duration)
        .map(|(duration, theirs)| (duration - theirs).abs());
    let title = title_match(wanted, &candidate, exact_artist, duration_delta)?;
    let duration = match duration_delta {
        None => 0,
        Some(delta) if delta <= 2.0 => 20,
        Some(delta) if delta <= 5.0 => 14,
        Some(delta) if delta <= 10.0 => 8,
        Some(delta) if delta <= 15.0 => 3,
        // The spec's -20 band can only ever total 55, so nothing above 15 s is shown either way.
        Some(_) => return None,
    };
    // Order doesn't matter: "Calm Down" by "Selena Gomez, Rema" is by Rema as well.
    let shares_lead = wanted.primary == candidate.primary
        || candidate.leads.contains(&wanted.primary)
        || wanted.leads.contains(&candidate.primary);
    if wanted.is_short_title() && (!shares_lead || duration_delta.is_some_and(|delta| delta > 3.0))
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

    let mut score = 40 + artist + duration;
    if !lines.is_empty() {
        score += 5;
    }
    let runs_past_track = duration_s
        .zip(lines.last())
        .is_some_and(|(duration, last)| last.time_ms as f64 > (duration + 3.0) * 1000.0);
    if runs_past_track {
        score -= 25;
    }
    if duration_delta.is_none() {
        score = score.min(UNKNOWN_DURATION_CAP);
    }

    Some(Candidate {
        approximate: duration_delta.is_none_or(|delta| delta > 2.0),
        credit_differs: !exact_artist || matches!(title, TitleMatch::Corroborated),
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
        .filter(|candidate| candidate.score >= ACCEPT_SCORE || candidate.duration_delta.is_none())
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
        .then_with(|| right.credit_differs.cmp(&left.credit_differs))
        .then_with(|| right.lines.is_empty().cmp(&left.lines.is_empty()))
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
            track_name: title.into(),
            artist_name: artist.into(),
            duration: Some(duration),
            instrumental: false,
            plain_lyrics: Some("Line".into()),
            synced_lyrics: Some("[00:01.00]Line".into()),
        }
    }

    fn plain(title: &str, artist: &str, duration: f64) -> LrclibRecord {
        LrclibRecord {
            synced_lyrics: None,
            ..record(title, artist, duration)
        }
    }

    fn found(
        title: &str,
        artist: &str,
        duration: Option<f64>,
        record: LrclibRecord,
    ) -> Option<Candidate> {
        score_candidate(&Track::parse(title, artist), record, duration)
    }

    fn score(
        title: &str,
        artist: &str,
        duration: Option<f64>,
        record: LrclibRecord,
    ) -> Option<i32> {
        found(title, artist, duration, record).map(|found| found.score)
    }

    fn accepted(title: &str, artist: &str, duration: f64, record: LrclibRecord) -> bool {
        score(title, artist, Some(duration), record).is_some_and(|value| value >= ACCEPT_SCORE)
    }

    /// (score, credit differs) for a record with the same 200 s duration.
    fn same_length(
        title: &str,
        artist: &str,
        their_title: &str,
        their_artist: &str,
    ) -> Option<(i32, bool)> {
        found(
            title,
            artist,
            Some(200.0),
            record(their_title, their_artist, 200.0),
        )
        .map(|found| (found.score, found.credit_differs))
    }

    #[test]
    fn reported_track_is_accepted_and_flagged_approximate() {
        let wanted = Track::parse("Big Thug Boys", "Babyboy AV");
        let found = best_search_match(
            &wanted,
            vec![
                record("Big Thug Boys (feat. Jireel)", "Babyboy AV", 163.0),
                record("Big Thug Boys (feat. Jireel)", "Babyboy AV", 164.0),
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
        assert_eq!(
            found.record.duration,
            Some(164.0),
            "ties go to the smallest gap"
        );
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
                same_length("Our Way Home", wanted, "Our Way Home", other),
                None,
                "{wanted} / {other}"
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
                same_length(ours, "Aster", theirs, "Aster"),
                None,
                "{ours} / {theirs}"
            );
        }
        assert!(same_length(
            "Northern Lights (Live)",
            "Aster",
            "Northern Lights - Live at Wembley",
            "Aster"
        )
        .is_some());
        assert!(same_length(
            "Northern Lights (Pt. 2)",
            "Aster",
            "Northern Lights (Part 2)",
            "Aster"
        )
        .is_some());
    }

    #[test]
    fn only_neutral_versions_are_noise() {
        let title = "Long Song Title";
        for neutral in [
            "Single Version",
            "Album Version",
            "Radio Version",
            "Original Version",
            "Remastered Version",
            "Mono Version",
            "Stereo Version",
            "Soundtrack Version",
            "LP Version",
            "2011 Remastered Version",
            "Bonus Track Version",
            "2011 Version",
        ] {
            assert_eq!(
                same_length(title, "Aster", &format!("{title} ({neutral})"), "Aster"),
                Some((95, false)),
                "({neutral})"
            );
            assert_eq!(
                same_length(&format!("{title} - {neutral}"), "Aster", title, "Aster"),
                Some((95, false)),
                "- {neutral}"
            );
        }
        for different in [
            "Spanish Version",
            "Dub Version",
            "Piano Version",
            "Extended Version",
            "Taylor's Version",
        ] {
            assert_eq!(
                same_length(title, "Aster", &format!("{title} ({different})"), "Aster"),
                None,
                "{different}"
            );
            assert_eq!(
                same_length(
                    &format!("{title} - {different}"),
                    "Aster",
                    &format!("{title} ({different})"),
                    "Aster"
                ),
                Some((95, false)),
                "{different} matches itself"
            );
        }
        assert_eq!(
            same_length(title, "Aster", &format!("{title} (Dub)"), "Aster"),
            None
        );
        assert_eq!(
            same_length(title, "Aster", &format!("{title} (Piano)"), "Aster"),
            None
        );
    }

    #[test]
    fn release_tags_do_not_hide_the_song() {
        for (ours, theirs) in [
            (
                "Let It Go - From \"Frozen\"/Soundtrack Version",
                "Let It Go",
            ),
            ("Let It Go - Soundtrack Version", "Let It Go"),
            ("Let It Go - From Frozen", "Let It Go"),
            (
                "Bohemian Rhapsody - 2011 Remastered Version",
                "Bohemian Rhapsody",
            ),
            ("Bohemian Rhapsody (Official Audio)", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody (Official Video)", "Bohemian Rhapsody"),
            (
                "Bohemian Rhapsody (Official Music Video)",
                "Bohemian Rhapsody",
            ),
            ("Bohemian Rhapsody (Lyric Video)", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody (Audio)", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody - Edit", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody - Single Edit", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody - Album Edit", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody - 2011 Mix", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody (2011)", "Bohemian Rhapsody"),
            (
                "Bohemian Rhapsody - Anniversary Edition",
                "Bohemian Rhapsody",
            ),
            (
                "Bohemian Rhapsody \u{2014} Remastered 2011",
                "Bohemian Rhapsody",
            ),
            (
                "Bohemian Rhapsody \u{2013} Remastered 2011",
                "Bohemian Rhapsody",
            ),
        ] {
            assert_eq!(
                same_length(ours, "Aster", theirs, "Aster"),
                Some((95, false)),
                "{ours}"
            );
        }
        for (ours, theirs) in [
            ("Bohemian Rhapsody - Club Edit", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody - Tiesto Mix", "Bohemian Rhapsody"),
            ("Bohemian Rhapsody (Live Video)", "Bohemian Rhapsody"),
            ("Love (From Me To You)", "Love"),
        ] {
            assert_eq!(same_length(ours, "Aster", theirs, "Aster"), None, "{ours}");
        }
    }

    #[test]
    fn video_uploads_name_the_artist_in_the_title() {
        let theirs = ("Calm Down Again", "Rema");
        for (title, channel) in [
            ("Rema - Calm Down Again (Official Video)", "Rema"),
            ("Rema - Calm Down Again (Official Video)", "RemaVEVO"),
            ("Rema - Calm Down Again (Official Video)", "Some Label"),
            ("Rema \u{2014} Calm Down Again (Lyric Video)", "Some Label"),
            ("Rema - Calm Down Again", "Rema - Topic"),
        ] {
            assert_eq!(
                same_length(title, channel, theirs.0, theirs.1),
                Some((95, false)),
                "{title} / {channel}"
            );
        }
        assert_eq!(
            same_length("Rema - Calm Down Again", "Some Label", theirs.0, theirs.1),
            None,
            "without a video tag the head may be part of the title"
        );
        let [title, artist] =
            Track::parse("Rema - Calm Down Again (Official Video)", "Some Label").search_query();
        assert_eq!(
            (title.1.as_str(), artist.1.as_str()),
            ("Calm Down Again", "Rema")
        );
    }

    #[test]
    fn parts_compare_however_they_are_written() {
        for ours in [
            "Northern Lights Pt. 2",
            "Northern Lights, Pt. 2",
            "Northern Lights Part 2",
            "Northern Lights (Part 2)",
            "Northern Lights - Pt. 2",
        ] {
            assert_eq!(
                same_length(ours, "Aster", "Northern Lights (Pt. 2)", "Aster"),
                Some((95, false)),
                "{ours}"
            );
        }
        assert_eq!(
            same_length(
                "Northern Lights Pt. 1",
                "Aster",
                "Northern Lights Part 2",
                "Aster"
            ),
            None
        );
        assert_eq!(Track::parse("Part 2", "Aster").core, "part 2");
    }

    #[test]
    fn benign_accents_fold_but_letters_do_not() {
        assert_eq!(
            normalize_text("Ёлка"),
            normalize_text("елка"),
            "ё is commonly written е"
        );
        assert_eq!(
            normalize_text("Άλφα"),
            normalize_text("αλφα"),
            "tonos is an accent"
        );
        assert_ne!(
            normalize_text("мой"),
            normalize_text("мои"),
            "й is its own letter"
        );
    }

    #[test]
    fn title_words_cannot_repeat_or_reorder() {
        assert_eq!(
            same_length("Money Money Money", "Aster", "Money", "Aster"),
            None
        );
        assert_eq!(
            same_length("Me and You", "Aster", "You and Me", "Aster"),
            None
        );
        assert_eq!(
            same_length(
                "One Two Three Four Five Six Seven Eight Nine Ten",
                "Aster",
                "One Two Three Four Five Six Seven Eight Nine Eleven",
                "Aster"
            ),
            None
        );
    }

    #[test]
    fn unconfirmed_credits_need_the_artist_and_duration_to_agree() {
        let at = |ours: &str, artist: &str, theirs: &str, their_artist: &str, seconds: f64| {
            found(
                ours,
                artist,
                Some(200.0),
                record(theirs, their_artist, seconds),
            )
            .map(|found| (found.score, found.approximate, found.credit_differs))
        };
        assert_eq!(
            at(
                "STAY (with Justin Bieber)",
                "The Kid LAROI",
                "STAY",
                "The Kid LAROI",
                199.0
            ),
            Some((95, false, true))
        );
        assert_eq!(
            at(
                "Calm Down (with Selena Gomez)",
                "Rema",
                "Calm Down",
                "Rema",
                200.0
            ),
            Some((95, false, true))
        );
        assert_eq!(
            at(
                "Calm Down",
                "Rema",
                "Calm Down (with Selena Gomez)",
                "Rema",
                200.0
            ),
            Some((95, false, true))
        );
        assert_eq!(
            at("Six Ft Under", "Aster", "Six", "Aster", 200.0),
            Some((95, false, true))
        );
        // Another song that happens to share the short title rarely agrees on both.
        assert_eq!(at("Run (With Me)", "Aster", "Run", "Aster", 205.0), None);
        assert_eq!(at("Run (With Me)", "Aster", "Run", "Comet", 200.0), None);
        assert_eq!(at("Run", "Aster", "Run (With Me)", "Comet", 200.0), None);
        assert_eq!(
            same_length("Run (With Me)", "Aster", "Run (With Me)", "Aster"),
            Some((95, false))
        );
        assert_eq!(
            same_length("Six Ft Under", "Aster", "Six Ft Under", "Aster"),
            Some((95, false))
        );
        assert_eq!(
            found(
                "Run (With Me)",
                "Aster",
                None,
                record("Run", "Aster", 200.0)
            )
            .map(|f| f.score),
            None,
            "no duration to agree on"
        );
        assert_ne!(
            Track::parse("Run (With Me)", "Aster").cache_key(Some(200.0)),
            Track::parse("Run", "Aster").cache_key(Some(200.0))
        );
    }

    #[test]
    fn the_closest_duration_wins_among_equal_credits() {
        let wanted = Track::parse("Essence (with Tems)", "Wizkid");
        let best = best_search_match(
            &wanted,
            vec![
                record("Essence", "Wizkid; Tems", 245.0),
                record("Essence", "Wizkid", 248.0),
            ],
            Some(248.0),
        )
        .expect("both are the song");
        assert_eq!(best.record.duration, Some(248.0));
        assert!(
            !best.approximate,
            "the timing agrees, only the credit differs"
        );
    }

    #[test]
    fn a_trailing_subtitle_needs_the_exact_credit_and_length() {
        let (short, long) = ("Love Nwantiti", "love nwantiti (ah ah ah)");
        assert_eq!(same_length(short, "CKay", long, "CKay"), Some((95, true)));
        assert_eq!(same_length(long, "CKay", short, "CKay"), Some((95, true)));
        assert_eq!(
            same_length(short, "Comet", long, "CKay"),
            None,
            "another artist"
        );
        assert_eq!(
            found(short, "CKay", Some(200.0), record(long, "CKay", 203.0)).map(|f| f.score),
            None,
            "another length"
        );
        assert_eq!(
            same_length("Love Nwantiti (Piano Ballad)", "CKay", short, "CKay"),
            None,
            "an arrangement"
        );
        assert_eq!(
            same_length("Love Nwantiti (Interlude Take)", "CKay", short, "CKay"),
            None,
            "an arrangement"
        );
        assert_eq!(
            same_length("Love Nwantiti (Dance)", "CKay", short, "CKay"),
            None,
            "one word"
        );
        assert_eq!(
            same_length("Run (Far Away)", "CKay", "Run", "CKay"),
            None,
            "a short base title"
        );
        assert_eq!(
            same_length("Love Nwantiti (ah ah ah) (Live)", "CKay", short, "CKay"),
            None,
            "live stays apart"
        );
        assert_eq!(
            same_length(
                "Love Nwantiti (ah ah ah) (Tiesto Remix)",
                "CKay",
                long,
                "CKay"
            ),
            None,
            "remix stays apart"
        );
    }

    #[test]
    fn confirmed_credits_are_removed_from_the_title() {
        assert_eq!(
            same_length("Run (with Friend)", "Aster, Friend", "Run", "Aster, Friend"),
            Some((95, false))
        );
        assert_eq!(
            same_length("Run ft. Friend", "Aster, Friend", "Run", "Aster, Friend"),
            Some((95, false))
        );
        // Confirmed by the other record's credits; the differing credit is what gets flagged.
        assert_eq!(
            same_length("Run (with Friend)", "Aster", "Run", "Aster, Friend"),
            Some((95, true))
        );
        assert_eq!(
            same_length("Run", "Aster, Friend", "Run (with Friend)", "Aster"),
            Some((95, true))
        );
        // Brackets and "feat." are never ordinary words.
        assert_eq!(
            same_length("Run (ft. Friend)", "Aster", "Run", "Aster"),
            Some((95, false))
        );
        assert_eq!(
            same_length("Run feat. Friend", "Aster", "Run", "Aster"),
            Some((95, false))
        );
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
        let title = "Shared Song Title";
        assert_eq!(
            same_length(title, "A & B", title, "B, A"),
            Some((95, false)),
            "same people"
        );
        assert_eq!(
            same_length(title, "A", title, "A feat. B"),
            Some((95, false)),
            "a guest on the record"
        );
        assert_eq!(
            same_length(title, "A feat. B", title, "A"),
            Some((95, false)),
            "a guest on ours"
        );
    }

    #[test]
    fn guest_and_fragment_credits_are_kept_but_flagged() {
        let title = "Long Song Title";
        assert_eq!(
            same_length(title, "Drake", &format!("{title} (feat. Drake)"), "Rihanna"),
            Some((95, true))
        );
        for (ours, theirs) in [
            ("Tyler, The Creator", "Tyler"),
            ("Earth, Wind & Fire", "Earth"),
            ("AC/DC", "AC"),
            ("Earth", "Earth, Wind & Fire"),
        ] {
            assert_eq!(
                same_length(title, ours, title, theirs),
                Some((95, true)),
                "{ours} / {theirs}"
            );
        }
        assert_eq!(
            same_length(title, "Foo Fighters", title, "Foo Fighters Live Band"),
            Some((93, true))
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
                same_length(title, "Aster", theirs, "Aster"),
                None,
                "{theirs}"
            );
        }
        assert_eq!(
            same_length(title, "Foo Fighters", title, "Karaoke Foo Fighters"),
            None
        );
        assert_eq!(
            same_length(title, "Foo Fighters", title, "Foo Fighters Tribute Band"),
            None
        );
        assert_eq!(
            same_length(title, "Tribute Band", title, "Tribute Band"),
            Some((95, false)),
            "their real name"
        );
        assert_eq!(
            same_length("Contribute", "Aster", "Contribute", "Aster"),
            Some((95, false))
        );
    }

    #[test]
    fn a_partial_artist_overlap_only_passes_with_everything_else_exact() {
        let found = same_length("Shared Song Title", "A, B", "Shared Song Title", "B")
            .expect("B is on both");
        assert_eq!(found, (75, true));
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
            "AC/DC",
        ] {
            assert_eq!(
                same_length("Long Song Title", artist, "Long Song Title", artist),
                Some((95, false)),
                "{artist}"
            );
        }
    }

    #[test]
    fn artists_with_nothing_comparable_only_match_themselves() {
        let title = "Long Song Title";
        assert_eq!(same_length(title, "!!!", title, "???"), None);
        assert_eq!(same_length(title, "!!!", title, ""), None);
        assert_eq!(same_length(title, "", title, ""), None);
        assert_eq!(same_length(title, "", title, "Aster"), None);
        assert_eq!(same_length(title, "!!!", title, "!!!"), Some((95, false)));
    }

    #[test]
    fn title_words_are_not_mistaken_for_tags() {
        assert_eq!(
            Track::parse("With or Without You", "U2").core,
            "with or without you"
        );
        assert_eq!(Track::parse("Stay With Me", "Sam").core, "stay with me");
        assert_eq!(Track::parse("Song (with Friend)", "Sam").core, "song");
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
        assert!(Track::parse("Song feat. Friend", "Sam")
            .artists
            .contains(&"friend".to_string()));
        assert_eq!(
            same_length("Stay With Me", "Sam", "Stay", "Sam"),
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
        assert_ne!(
            normalize_text("мой"),
            normalize_text("мои"),
            "й is its own letter"
        );
        assert_eq!(
            same_length("Долгая песня мой", "Алиса", "Долгая песня мои", "Алиса"),
            None
        );
        assert_eq!(same_length("東京", "Алиса", "大阪", "Алиса"), None);
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
    fn a_null_record_duration_counts_as_unknown() {
        let parsed: LrclibRecord = serde_json::from_str(
            r#"{"trackName":"Three Word Title","artistName":"Aster","duration":null,"plainLyrics":"Line"}"#,
        )
        .expect("a null duration must not break parsing");
        assert_eq!(parsed.duration, None);
        let wanted = Track::parse("Three Word Title", "Aster");
        let exact = accept_exact(&wanted, parsed.clone(), Some(200.0)).expect("exact lookup");
        assert!(exact.approximate);
        assert!(best_search_match(&wanted, vec![parsed], Some(200.0)).is_none());
    }

    #[test]
    fn short_titles_need_a_shared_artist_and_a_close_duration() {
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
            score("22", "Aster", Some(200.0), record("22", "Comet", 200.0)),
            None
        );
        assert_eq!(
            same_length(
                "Calm Down",
                "Selena Gomez, Rema",
                "Calm Down",
                "Rema, Selena Gomez"
            ),
            Some((95, false)),
            "order"
        );
        assert_eq!(
            same_length("Calm Down", "Rema", "Calm Down", "Selena Gomez, Rema"),
            Some((95, true)),
            "second lead"
        );
        assert_eq!(
            same_length("Calm Down", "Selena Gomez, Rema", "Calm Down", "Rema"),
            Some((75, true)),
            "their lead is one of ours"
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
    fn short_titles_are_counted_in_whole_characters() {
        assert!(
            Track::parse("봄날", "Aster").is_short_title(),
            "two syllables, six jamo"
        );
        assert_eq!(
            score("봄날", "Aster", Some(200.0), record("봄날", "Aster", 206.0)),
            None
        );
        assert!(accepted(
            "봄날",
            "Aster",
            200.0,
            record("봄날", "Aster", 202.0)
        ));
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
        assert_eq!(found.record.duration, Some(196.0));
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
    fn the_real_credit_beats_a_fragment_at_the_same_score() {
        let wanted = Track::parse("Long Song Title", "Tyler, The Creator");
        let best = best_search_match(
            &wanted,
            vec![
                record("Long Song Title", "Tyler", 200.0),
                record("Long Song Title", "Tyler, The Creator", 200.0),
            ],
            Some(200.0),
        )
        .expect("both pass");
        assert_eq!(best.record.artist_name, "Tyler, The Creator");
        assert!(!best.approximate);
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
        assert_eq!(
            same_length(
                "Song Title Here (feat. Friend)",
                "Aster",
                "Song Title Here",
                "Aster"
            ),
            Some((95, false))
        );
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
    fn cache_key_separates_recordings_but_not_their_metadata_noise() {
        let key = |title, artist, duration| Track::parse(title, artist).cache_key(duration);
        let seconds = Some(180.0);
        assert_eq!(
            key("Yellow (2011 Remaster)", " Coldplay ", seconds),
            key("yellow", "coldplay", seconds)
        );
        assert_eq!(
            key("Yellow (feat. Someone)", "Coldplay, Someone", seconds),
            key("Yellow", "Coldplay", seconds)
        );
        assert_eq!(
            key("Yellow", "Coldplay", Some(180.4)),
            key("Yellow", "Coldplay", seconds),
            "rounded"
        );
        assert_ne!(
            key("Yellow (Live)", "Coldplay", seconds),
            key("Yellow", "Coldplay", seconds)
        );
        assert_ne!(
            key("Intro", "Drake", Some(52.0)),
            key("Intro", "Drake", Some(25.0))
        );
        assert_ne!(
            key("Yellow", "Coldplay", None),
            key("Yellow", "Coldplay", seconds)
        );
    }
}
