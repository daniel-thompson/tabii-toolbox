//! Parser for the ASCII guitar tab format described in README.md.
//!
//! The format is "natural" ASCII tab enriched with a few markers:
//!
//! * `s:4/4` (or `signature:4/4`) sets the time signature for the following
//!   systems,
//! * lines of only dashes separate metadata from tab (or systems from each
//!   other),
//! * a system is six string lines, the top line being the high E string,
//! * each bar is `|`/`!` followed by the bar content and closed by `|`, `:|`
//!   (repeat) or `||` (double barline),
//! * `!` in place of a barline marks the lowest string belonging to the
//!   treble voice; strings below it belong to the bass voice,
//! * digits are fret numbers at that horizontal position,
//! * `^` marks that the previous note on the string keeps sounding (a tie),
//! * a line of chord symbols above a system aligns with the bar grid.

use std::collections::HashSet;

use crate::music::{self, Chord, Key, TimeSig, Tuning};

/// A parsed tab: metadata plus a list of systems (lines of bars).
#[derive(Debug)]
pub struct Tab {
    pub title: Option<String>,
    pub composer: Option<String>,
    pub arranger: Option<String>,
    pub key: Option<Key>,
    pub tuning: Tuning,
    pub systems: Vec<System>,
    pub warnings: Vec<String>,
}

/// One printed line of tab: a row of bars plus any chord symbols above it.
#[derive(Debug)]
pub struct System {
    pub time_sig: TimeSig,
    pub bars: Vec<Bar>,
    pub chords: Vec<ChordSlot>,
    pub final_barline: Barline,
}

/// A chord symbol together with the position it starts at.
#[derive(Debug)]
pub struct ChordSlot {
    pub bar: usize,
    pub col: usize,
    pub chord: Chord,
}

/// One bar: the events on each of the six strings plus the voice split.
#[derive(Debug)]
pub struct Bar {
    pub width: usize,
    /// The split in force at the start of the bar: an explicit `!` or the
    /// split carried over from the previous bar. `None` means no voice has
    /// ever been separated: all strings are one (treble) voice.
    pub voice_split: Option<usize>,
    /// `(column, string)` pairs where a `.` moves the voice split to that
    /// string, applying to every note from that column onward.
    pub split_changes: Vec<(usize, usize)>,
    pub strings: Vec<Vec<StringEvent>>,
}

/// An event on one string: a fretted note (optionally marked as part of an
/// arpeggio), a tie mark, or a `.` split change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringEvent {
    Onset {
        col: usize,
        fret: u8,
        arpeggio: bool,
    },
    Tie {
        col: usize,
    },
    Split {
        col: usize,
    },
}

impl StringEvent {
    pub fn col(self) -> usize {
        match self {
            StringEvent::Onset { col, .. }
            | StringEvent::Tie { col }
            | StringEvent::Split { col } => col,
        }
    }
}

/// Closing barline of a system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Barline {
    Single,
    Double,
    RepeatEnd,
}

/// One parsed tab line: per-bar voice markers, bar contents and the closing
/// barline.
type ParsedLine = (Vec<char>, Vec<String>, Barline);

/// Parse a whole tab document.
pub fn parse(input: &str) -> Result<Tab, String> {
    let mut parser = Parser {
        tab: Tab {
            title: None,
            composer: None,
            arranger: None,
            key: None,
            tuning: Tuning::standard(),
            systems: Vec::new(),
            warnings: Vec::new(),
        },
        time_sig: TimeSig { num: 4, den: 4 },
        voice_split: None,
        pending_chords: Vec::new(),
        tab_lines: Vec::new(),
        unknown_chars: HashSet::new(),
    };
    for (n, raw) in input.lines().enumerate() {
        parser.line(n + 1, raw.trim_end());
    }
    parser.finish()
}

struct Parser {
    tab: Tab,
    time_sig: TimeSig,
    /// The voice split currently in force: set by a `!` (or moved by a `.`)
    /// and carried over every later bar, including bars on later lines.
    voice_split: Option<usize>,
    pending_chords: Vec<(usize, String)>,
    tab_lines: Vec<(usize, String)>,
    unknown_chars: HashSet<char>,
}

impl Parser {
    fn warn(&mut self, lineno: usize, msg: impl Into<String>) {
        self.tab
            .warnings
            .push(format!("line {lineno}: {}", msg.into()));
    }

    fn line(&mut self, lineno: usize, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            self.flush_system();
            return;
        }
        if is_fence(trimmed) {
            return;
        }
        if is_separator(trimmed) {
            self.flush_system();
            return;
        }
        if is_tab_line(line) {
            // Systems may be run together without a blank line.
            if self.tab_lines.len() == 6 {
                self.flush_system();
            }
            self.tab_lines.push((lineno, line.to_string()));
            return;
        }
        self.flush_system();
        if let Some((key, value)) = metadata(trimmed) {
            self.metadata(lineno, &key, &value);
            return;
        }
        if let Some(tokens) = chord_tokens(line) {
            if !self.pending_chords.is_empty() {
                self.warn(
                    lineno,
                    "chord symbols without a following tab system; replaced",
                );
            }
            self.pending_chords = tokens;
            return;
        }
        self.warn(lineno, format!("ignoring unrecognised line {line:?}"));
    }

    fn metadata(&mut self, lineno: usize, key: &str, value: &str) {
        let value = value.trim();
        match key.to_ascii_lowercase().as_str() {
            "s" | "signature" => match parse_time_sig(value) {
                Some(sig) => self.time_sig = sig,
                None => self.warn(
                    lineno,
                    format!("ignoring malformed time signature {value:?}"),
                ),
            },
            "key" => match parse_key(value) {
                Some(k) => {
                    if self.tab.key.is_some() && self.tab.key != Some(k) {
                        self.warn(lineno, "key redefined; using the new value");
                    }
                    self.tab.key = Some(k);
                }
                None => self.warn(lineno, format!("ignoring malformed key {value:?}")),
            },
            "tuning" => {
                if !value.eq_ignore_ascii_case("standard") {
                    self.warn(
                        lineno,
                        format!("unsupported tuning {value:?}; assuming standard"),
                    );
                }
            }
            "title" => self.tab.title = Some(value.to_string()),
            "composer" => self.tab.composer = Some(value.to_string()),
            "arranger" => self.tab.arranger = Some(value.to_string()),
            _ => self.warn(lineno, format!("unknown metadata key {key:?}")),
        }
    }

    /// Turn the accumulated string lines into a system.
    fn flush_system(&mut self) {
        if self.tab_lines.is_empty() {
            return;
        }
        let lines = std::mem::take(&mut self.tab_lines);
        let lineno = lines[0].0;
        if lines.len() != 6 {
            self.warn(
                lineno,
                format!(
                    "tab system has {} string lines, expected 6; skipped",
                    lines.len()
                ),
            );
            return;
        }
        let parsed: Option<Vec<ParsedLine>> =
            lines.iter().map(|(_, l)| parse_tab_line(l)).collect();
        let Some(parsed) = parsed else {
            self.warn(lineno, "malformed tab system; skipped");
            return;
        };
        let n_bars = parsed[0].1.len();
        let width = parsed[0].1.first().map_or(0, String::len);
        let consistent = n_bars > 0
            && width > 0
            && parsed.iter().all(|(_, chunks, _)| {
                chunks.len() == n_bars && chunks.iter().all(|c| c.len() == width)
            });
        if !consistent {
            self.warn(lineno, "inconsistent bar widths in tab system; skipped");
            return;
        }
        let final_barline = parsed[0].2;
        if parsed.iter().any(|(_, _, b)| *b != final_barline) {
            self.warn(lineno, "inconsistent closing barlines; using the first");
        }
        let mut bars = Vec::with_capacity(n_bars);
        for b in 0..n_bars {
            let bangs: Vec<usize> = (0..6).filter(|&s| parsed[s].0[b] == '!').collect();
            if bangs.len() > 1 {
                self.warn(lineno, "multiple voice markers in one bar; using the first");
            }
            // An explicit `!` sets the split for this bar; without one the
            // split carries over from the previous bar, even across lines.
            if let Some(&string) = bangs.first() {
                self.voice_split = Some(string);
            }
            let voice_split = self.voice_split;
            let mut strings = Vec::with_capacity(6);
            let mut split_changes: Vec<(usize, usize)> = Vec::new();
            for (string, line) in parsed.iter().enumerate() {
                let events = self.parse_string_events(&line.1[b]);
                // A `.` moves the voice split from its column onward; it is
                // not a note position itself (it acts like a `-`).
                let (splits, notes): (Vec<_>, Vec<_>) = events
                    .into_iter()
                    .partition(|event| matches!(event, StringEvent::Split { .. }));
                split_changes.extend(splits.into_iter().map(|event| (event.col(), string)));
                strings.push(notes);
            }
            split_changes.sort_by_key(|&(col, string)| (col, string));
            // A split change keeps applying to every later bar.
            if let Some(&(_, string)) = split_changes.last() {
                self.voice_split = Some(string);
            }
            bars.push(Bar {
                width,
                voice_split,
                split_changes,
                strings,
            });
        }
        let chords = self.resolve_chords(lineno, n_bars, width);
        self.tab.systems.push(System {
            time_sig: self.time_sig,
            bars,
            chords,
            final_barline,
        });
    }

    /// Extract fret onsets, tie marks and split changes from one bar of one
    /// string.
    fn parse_string_events(&mut self, chunk: &str) -> Vec<StringEvent> {
        let mut events = Vec::new();
        // A `~` before a fret number marks an arpeggiated note that starts
        // on the `~`'s column rather than the number's.
        let mut arpeggio: Option<usize> = None;
        for (i, ch) in chunk.chars().enumerate() {
            let col = i + 1;
            match ch {
                '0'..='9' => {
                    let (onset, arpeggiated) = match arpeggio.take() {
                        Some(start) => (start, true),
                        None => (col, false),
                    };
                    events.push(StringEvent::Onset {
                        col: onset,
                        fret: ch as u8 - b'0',
                        arpeggio: arpeggiated,
                    });
                }
                '-' | ' ' => self.drop_arpeggio(&mut arpeggio),
                '^' => {
                    self.drop_arpeggio(&mut arpeggio);
                    events.push(StringEvent::Tie { col });
                }
                '.' => {
                    self.drop_arpeggio(&mut arpeggio);
                    events.push(StringEvent::Split { col });
                }
                '~' => arpeggio = Some(col),
                _ => {
                    self.drop_arpeggio(&mut arpeggio);
                    if self.unknown_chars.insert(ch) {
                        self.tab.warnings.push(format!(
                            "ignoring unsupported character {ch:?} in tab (treated as a gap)"
                        ));
                    }
                }
            }
        }
        self.drop_arpeggio(&mut arpeggio);
        events
    }

    /// A `~` must be followed by a fret number; anything else after it (or
    /// the end of the bar) leaves the mark dangling, which is warned about
    /// once and treated as a gap.
    fn drop_arpeggio(&mut self, arpeggio: &mut Option<usize>) {
        if arpeggio.take().is_some() && self.unknown_chars.insert('~') {
            self.tab
                .warnings
                .push("ignoring '~' not followed by a fret number (treated as a gap)".to_string());
        }
    }

    /// Map the pending chord symbols onto bars using the system's grid.
    fn resolve_chords(&mut self, lineno: usize, n_bars: usize, width: usize) -> Vec<ChordSlot> {
        let tokens = std::mem::take(&mut self.pending_chords);
        if tokens.is_empty() {
            return Vec::new();
        }
        let stride = width + 1;
        let beat = if width.is_multiple_of(self.time_sig.num as usize) {
            width / self.time_sig.num as usize
        } else {
            self.warn(
                lineno,
                "bar width is not a multiple of the beat count; chord positions not snapped",
            );
            0
        };
        let mut slots = Vec::new();
        for (linecol, token) in tokens {
            let Some(chord) = music::parse_chord(&token) else {
                self.warn(
                    lineno,
                    format!("ignoring unrecognised chord symbol {token:?}"),
                );
                continue;
            };
            // Content column c of bar i sits at line column 1 + i * stride + c.
            let (bar, mut col) = if linecol < 2 {
                (0, 1)
            } else {
                let bar = (linecol - 2) / stride;
                let col = linecol - 1 - bar * stride;
                (bar, col)
            };
            if bar >= n_bars {
                self.warn(
                    lineno,
                    format!("chord symbol {token:?} lies beyond the tab system; ignored"),
                );
                continue;
            }
            if let Some(k) = (col - 1).checked_div(beat) {
                // Chord symbols are hand aligned; snap to the nearest beat.
                let lower = 1 + k * beat;
                let upper = 1 + (k + 1) * beat;
                if upper <= width && upper - col < col - lower {
                    col = upper;
                } else {
                    col = lower;
                }
            }
            if slots
                .last()
                .is_some_and(|last: &ChordSlot| last.bar == bar && last.col == col)
            {
                self.warn(
                    lineno,
                    format!("duplicate chord symbol at the same position; ignoring {token:?}"),
                );
                continue;
            }
            slots.push(ChordSlot { bar, col, chord });
        }
        slots
    }

    fn finish(mut self) -> Result<Tab, String> {
        self.flush_system();
        if !self.pending_chords.is_empty() {
            self.tab
                .warnings
                .push("chord symbols without a following tab system; ignored".to_string());
        }
        if self.tab.systems.is_empty() {
            return Err("no tab systems found in input".to_string());
        }
        Ok(self.tab)
    }
}

/// A `~~~`/```` ``` ```` fence line (so README.md itself can be parsed).
fn is_fence(line: &str) -> bool {
    line.starts_with("~~~") || line.starts_with("```")
}

/// A line of only dashes (and spaces) separating sections.
fn is_separator(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|c| c == '-' || c == ' ')
}

/// A tab line starts with a barline or voice marker in column one.
fn is_tab_line(line: &str) -> bool {
    line.starts_with('|') || line.starts_with('!')
}

/// A `key: value` metadata line.
fn metadata(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once(':')?;
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

fn parse_time_sig(value: &str) -> Option<TimeSig> {
    let (num, den) = value.split_once('/')?;
    let num: u32 = num.trim().parse().ok()?;
    let den: u32 = den.trim().parse().ok()?;
    if num == 0 || den == 0 {
        return None;
    }
    Some(TimeSig { num, den })
}

fn parse_key(value: &str) -> Option<Key> {
    let mut chars = value.trim().chars();
    let letter = match chars.next()?.to_ascii_uppercase() {
        'A' => 5,
        'B' => 6,
        'C' => 0,
        'D' => 1,
        'E' => 2,
        'F' => 3,
        'G' => 4,
        _ => return None,
    };
    let mut acc = 0i32;
    let mut rest = chars.as_str();
    loop {
        if let Some(r) = rest.strip_prefix('#') {
            acc += 1;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('b') {
            acc -= 1;
            rest = r;
        } else {
            break;
        }
    }
    let minor = matches!(rest, "m" | "min" | "minor");
    if !minor && !rest.is_empty() {
        return None;
    }
    Some(Key { letter, acc, minor })
}

/// Tokenize a chord symbol line, returning each token with its 1-based
/// character column.  Returns `None` if the line does not look like chords.
fn chord_tokens(line: &str) -> Option<Vec<(usize, String)>> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        let token: String = chars[start..i].iter().collect();
        if !is_chord_token(&token) {
            return None;
        }
        tokens.push((start + 1, token));
    }
    if tokens.is_empty() {
        None
    } else {
        Some(tokens)
    }
}

fn is_chord_token(token: &str) -> bool {
    let chars: Vec<char> = token.chars().collect();
    let Some(&first) = chars.first() else {
        return false;
    };
    if !matches!(first, 'A'..='G') {
        return false;
    }
    let mut i = 1;
    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '#' | '+')) {
        i += 1;
    }
    if i < chars.len() {
        if chars[i] != '/' {
            return false;
        }
        i += 1;
        let Some(&bass) = chars.get(i) else {
            return false;
        };
        if !matches!(bass, 'A'..='G') {
            return false;
        }
        i += 1;
        while i < chars.len() && matches!(chars[i], '#' | 'b') {
            i += 1;
        }
    }
    i == chars.len()
}

/// Split one tab line into per-bar voice markers, bar contents and the
/// closing barline.
fn parse_tab_line(line: &str) -> Option<(Vec<char>, Vec<String>, Barline)> {
    let chars: Vec<char> = line.chars().collect();
    match chars.first()? {
        '|' | '!' => {}
        _ => return None,
    }
    if chars.last() != Some(&'|') {
        return None;
    }
    let mut end = chars.len() - 1;
    let mut barline = Barline::Single;
    if end >= 1 && chars[end - 1] == ':' {
        barline = Barline::RepeatEnd;
        end -= 1;
    } else if end >= 1 && chars[end - 1] == '|' {
        barline = Barline::Double;
        end -= 1;
    }
    let mut markers = Vec::new();
    let mut chunks = Vec::new();
    let mut current = String::new();
    for &c in &chars[..end] {
        if c == '|' || c == '!' {
            let first = markers.is_empty();
            markers.push(c);
            if !first {
                chunks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    chunks.push(current);
    Some((markers, chunks, barline))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{Qual, Spelling};

    const README_EXAMPLE: &str = "\
s:4/4
--
|----------2---5-|
|------4---------|
|--2-------------|
!4---------------|
|----2---4-------|
|2-5-------------|
";

    #[test]
    fn readme_example() {
        let tab = parse(README_EXAMPLE).unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        assert_eq!(tab.systems.len(), 1);
        let system = &tab.systems[0];
        assert_eq!(system.time_sig, TimeSig { num: 4, den: 4 });
        assert_eq!(system.bars.len(), 1);
        assert_eq!(system.final_barline, Barline::Single);
        let bar = &system.bars[0];
        assert_eq!(bar.width, 16);
        assert_eq!(bar.voice_split, Some(3));
        assert_eq!(
            bar.strings[0],
            vec![
                StringEvent::Onset {
                    col: 11,
                    fret: 2,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 15,
                    fret: 5,
                    arpeggio: false
                }
            ]
        );
        assert_eq!(
            bar.strings[1],
            vec![StringEvent::Onset {
                col: 7,
                fret: 4,
                arpeggio: false
            }]
        );
        assert_eq!(
            bar.strings[2],
            vec![StringEvent::Onset {
                col: 3,
                fret: 2,
                arpeggio: false
            }]
        );
        assert_eq!(
            bar.strings[3],
            vec![StringEvent::Onset {
                col: 1,
                fret: 4,
                arpeggio: false
            }]
        );
        assert_eq!(
            bar.strings[4],
            vec![
                StringEvent::Onset {
                    col: 5,
                    fret: 2,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 9,
                    fret: 4,
                    arpeggio: false
                }
            ]
        );
        assert_eq!(
            bar.strings[5],
            vec![
                StringEvent::Onset {
                    col: 1,
                    fret: 2,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 3,
                    fret: 5,
                    arpeggio: false
                }
            ]
        );
    }

    #[test]
    fn readme_md_itself_parses() {
        // All five fenced examples inside README.md should be found.
        let tab = parse(include_str!("../README.md")).unwrap();
        assert_eq!(tab.systems.len(), 5);
        assert_eq!(tab.systems[0].bars.len(), 1);
        assert_eq!(tab.systems[0].time_sig, TimeSig { num: 4, den: 4 });
        // The second example splits at the third string and moves the split
        // to the fourth string with a `.` in the eighth column.
        assert_eq!(tab.systems[1].bars.len(), 1);
        let bar = &tab.systems[1].bars[0];
        assert_eq!(bar.voice_split, Some(2));
        assert_eq!(bar.split_changes, vec![(8, 3)]);
        assert_eq!(
            bar.strings[3],
            vec![
                StringEvent::Onset {
                    col: 5,
                    fret: 2,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 9,
                    fret: 4,
                    arpeggio: false
                },
            ]
        );
        // The third example has no metadata (defaulting to 4/4) and no
        // marker on its second bar, which keeps the first bar's split.
        assert_eq!(tab.systems[2].bars.len(), 2);
        assert_eq!(tab.systems[2].time_sig, TimeSig { num: 4, den: 4 });
        assert_eq!(tab.systems[2].bars[0].voice_split, Some(2));
        assert_eq!(tab.systems[2].bars[1].voice_split, Some(2));
        // The fourth example marks its second bar single voice with a `!` on
        // the bottom string.
        assert_eq!(tab.systems[3].bars.len(), 2);
        assert_eq!(tab.systems[3].bars[0].voice_split, Some(2));
        assert_eq!(tab.systems[3].bars[1].voice_split, Some(5));
        // The fifth example marks an arpeggiated chord: every note starts on
        // the `~` in the first column, not on the fret number next to it. It
        // has no marker of its own, so it keeps the single-voice split (`!`
        // on the bottom string) from the fourth example.
        assert_eq!(tab.systems[4].bars.len(), 1);
        assert_eq!(tab.systems[4].bars[0].voice_split, Some(5));
        assert_eq!(
            tab.systems[4].bars[0].strings[0],
            vec![StringEvent::Onset {
                col: 1,
                fret: 0,
                arpeggio: true
            }]
        );
        assert_eq!(
            tab.systems[4].bars[0].strings[1],
            vec![StringEvent::Onset {
                col: 1,
                fret: 1,
                arpeggio: true
            }]
        );
        assert_eq!(
            tab.systems[4].bars[0].strings[2],
            vec![StringEvent::Onset {
                col: 1,
                fret: 2,
                arpeggio: true
            }]
        );
    }

    #[test]
    fn aura_lee() {
        let tab = parse(include_str!("../examples/aura_lee.tab")).unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        assert_eq!(tab.title.as_deref(), Some("Aura Lee"));
        assert_eq!(tab.composer.as_deref(), Some("Traditional"));
        assert_eq!(tab.arranger.as_deref(), Some("Chris Proctor"));
        assert_eq!(
            tab.key,
            Some(Key {
                letter: 0,
                acc: 0,
                minor: false
            })
        );
        assert_eq!(tab.systems.len(), 3);
        for system in &tab.systems {
            assert_eq!(system.bars.len(), 4);
            assert_eq!(system.bars[0].width, 16);
        }
        assert_eq!(tab.systems[0].final_barline, Barline::RepeatEnd);
        assert_eq!(tab.systems[1].final_barline, Barline::Single);
        assert_eq!(tab.systems[2].final_barline, Barline::Double);

        // First system, first bar (C major): treble on the top three strings.
        let bar = &tab.systems[0].bars[0];
        assert_eq!(bar.voice_split, Some(2));
        assert_eq!(
            bar.strings[2],
            vec![StringEvent::Onset {
                col: 1,
                fret: 0,
                arpeggio: false
            }]
        );
        assert_eq!(
            bar.strings[1],
            vec![
                StringEvent::Onset {
                    col: 5,
                    fret: 1,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 9,
                    fret: 0,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 11,
                    fret: 1,
                    arpeggio: false
                }
            ]
        );
        assert_eq!(
            bar.strings[4],
            vec![
                StringEvent::Onset {
                    col: 1,
                    fret: 3,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 9,
                    fret: 3,
                    arpeggio: false
                }
            ]
        );
        assert!(bar.strings[5].is_empty());

        // The voice split moves per bar: bar 3 of the second system has the
        // treble voice on the top two strings only, the last system's third
        // bar keeps that split without a marker, and the final bar is marked
        // single voice with a `!` on the bottom string.
        assert_eq!(tab.systems[1].bars[2].voice_split, Some(1));
        // The `.` in the eighth column moves the split to the third string.
        assert_eq!(tab.systems[1].bars[2].split_changes, vec![(8, 2)]);
        assert_eq!(tab.systems[2].bars[2].voice_split, Some(2));
        assert_eq!(tab.systems[2].bars[3].voice_split, Some(5));

        // Tie marks are parsed.
        let bar = &tab.systems[2].bars[0];
        assert_eq!(
            bar.strings[1],
            vec![
                StringEvent::Onset {
                    col: 1,
                    fret: 1,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 5,
                    fret: 1,
                    arpeggio: false
                },
                StringEvent::Onset {
                    col: 9,
                    fret: 2,
                    arpeggio: false
                },
                StringEvent::Tie { col: 11 }
            ]
        );

        // Chord symbols: seven in the second system, including two half-bar
        // changes snapped to beat three.
        let chords = &tab.systems[1].chords;
        assert_eq!(chords.len(), 7);
        let positions: Vec<(usize, usize)> = chords.iter().map(|c| (c.bar, c.col)).collect();
        assert_eq!(
            positions,
            vec![(0, 1), (0, 9), (1, 1), (1, 9), (2, 1), (2, 9), (3, 1)]
        );
        assert_eq!(chords[1].chord.qual, Qual::Seven);
        assert_eq!(chords[4].chord.qual, Qual::MajSeven);
        let chords = &tab.systems[2].chords;
        assert_eq!(chords.len(), 5);
        assert_eq!(chords[1].chord.qual, Qual::Seven);
        assert_eq!(chords[2].chord.bass, Some(Spelling { letter: 3, acc: 1 }));
    }

    #[test]
    fn rejects_bad_systems() {
        assert!(parse("hello").is_err());
        // A lone tab line is not a system, so the document has no music.
        let err = parse("|---|\n").unwrap_err();
        assert_eq!(err, "no tab systems found in input");
    }
}
