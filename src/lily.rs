//! LilyPond source generation.
//!
//! Each system of tab becomes a line of music.  Note timings come from the
//! horizontal grid: one bar of tab spans exactly one bar of music, so a
//! 16 column bar in 4/4 gives one semiquaver per column.  A note lasts until
//! the next event in its voice (or the end of its bar); notes that start off
//! the beat and cross a beat boundary are written as tied notes, which is
//! what turns the off-beat crotchets of the README example into pairs of
//! tied quavers.  On a wider ternary grid (24 columns per 4/4 bar) lengths
//! that only exist as tuplets, such as triplet crotchets, are written as
//! single tuplet notes, and consecutive tuplet notes are grouped into
//! `\tuplet` blocks.

use crate::music::{
    Chord, Dur, Rational, Spelling, SpellingMap, TimeSig, chord_spelling_map, key_spelling_map,
    lily_note_name, note_durs, pitch_class_name, spell,
};
use crate::parser::{Bar, Barline, StringEvent, System, Tab};

const LILYPOND_VERSION: &str = "2.24.0";
pub(crate) const STRING_COUNT: usize = 6;

/// Which staff voice a stream of music belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Voice {
    Single,
    Upper,
    Lower,
}

/// The volta structure at one bar of the whole tab.
#[derive(Clone, Copy, Default)]
pub(crate) struct VoltaBar {
    /// `\repeat volta N {` opens at this bar.
    pub(crate) start: Option<usize>,
    /// The repeat body closes at this bar (`}`).
    pub(crate) body_end: bool,
    /// `\alternative {` opens at this bar, before the first alternative.
    pub(crate) alt_list_start: bool,
    /// `\volta N {` opens at this bar.
    pub(crate) alt_start: Option<usize>,
    /// The current `\volta` block closes at this bar (`}`).
    pub(crate) alt_end: bool,
    /// The `\alternative` block closes after this bar (`}`).
    pub(crate) alt_list_end: bool,
}

/// Generate LilyPond source for a parsed tab, plus any warnings raised while
/// generating.
pub fn generate(tab: &Tab) -> (String, Vec<String>) {
    let mut warnings = Vec::new();
    let mut out = String::new();
    out.push_str(&format!("\\version \"{LILYPOND_VERSION}\"\n\n"));
    out.push_str(
        "\\paper {\n  #(set-paper-size \"a4\")\n  left-margin = 20\n  line-width = 180\n  indent = #0\n}\n\n",
    );

    let mut header: Vec<(&str, &String)> = Vec::new();
    if let Some(title) = &tab.title {
        header.push(("title", title));
    }
    if let Some(composer) = &tab.composer {
        header.push(("composer", composer));
    }
    if let Some(arranger) = &tab.arranger {
        header.push(("arranger", arranger));
    }
    // The tagline is a header field, so the header block is always
    // emitted, even when the tab has no metadata.
    out.push_str("\\header {\n");
    for (field, value) in header {
        out.push_str(&format!("  {field} = {}\n", quote(value)));
    }
    out.push_str("  tagline = ##f\n");
    out.push_str("}\n\n");

    let has_chords = tab.systems.iter().any(|system| !system.chords.is_empty());
    let key_map = tab.key.map(|key| key_spelling_map(key.fifths()));
    let volta = volta_marks(tab, &mut warnings);

    // Each voice's music is defined once in a top level variable and shared
    // by both staves.  The tab-only string numbers and arpeggios carry the
    // `tab` tag and the tempo mark the `staff` tag, so each staff strips
    // what it does not want with \removeWithTag.
    let two_voice = tab.systems.iter().any(|system| {
        system
            .bars
            .iter()
            .any(|bar| bar.voice_split.is_some() || !bar.split_changes.is_empty())
    });
    let voices: Vec<(&str, Voice)> = if two_voice {
        vec![("VoiceOne", Voice::Upper), ("VoiceTwo", Voice::Lower)]
    } else {
        vec![("VoiceOne", Voice::Single)]
    };
    for (name, voice) in &voices {
        let mut state = VoiceState::new();
        let piece = Piece {
            tab,
            voice: *voice,
            key_map: key_map.as_ref(),
            volta: &volta,
        };
        let lines = render_voice_piece(&piece, &mut state, &mut warnings);
        out.push_str(&format!("{name} = {{\n"));
        // Tab staves suppress arpeggios by default; the override is tagged
        // so only the tablature staff keeps it.
        out.push_str("  \\tag #'tab { \\override Arpeggio.stencil = #ly:arpeggio::print }\n");
        // The string numbers that pin the tablature staff's frets would
        // print as circled numbers on the conventional staff; one tagged
        // omit hides them there, and the tablature staff strips it.
        out.push_str("  \\tag #'staff { \\omit StringNumber }\n");
        match voice {
            Voice::Upper => out.push_str("  \\voiceOne\n"),
            Voice::Lower => out.push_str("  \\voiceTwo\n"),
            Voice::Single => {}
        }
        for line in lines {
            out.push_str(&format!("  {line}\n"));
        }
        out.push_str("}\n\n");
    }

    out.push_str("\\score {\n");
    out.push_str("  <<\n");
    if has_chords {
        out.push_str("    \\new ChordNames \\chordmode {\n");
        for line in chord_lines(tab, &volta) {
            out.push_str(&format!("      {line}\n"));
        }
        out.push_str("    }\n");
    }
    out.push_str("    \\new StaffGroup <<\n");
    if two_voice {
        out.push_str("      \\new Staff <<\n");
        out.push_str("        \\clef \"treble_8\"\n");
        out.push_str("        \\new Voice = \"upper\" { \\removeWithTag #'tab \\VoiceOne }\n");
        out.push_str("        \\new Voice = \"lower\" { \\removeWithTag #'tab \\VoiceTwo }\n");
        out.push_str("      >>\n");
        out.push_str("      \\new TabStaff <<\n");
        out.push_str("        \\new TabVoice = \"upper\" { \\removeWithTag #'staff \\VoiceOne }\n");
        out.push_str("        \\new TabVoice = \"lower\" { \\removeWithTag #'staff \\VoiceTwo }\n");
        out.push_str("      >>\n");
    } else {
        out.push_str(
            "      \\new Staff {\n        \\clef \"treble_8\"\n        \\removeWithTag #'tab \\VoiceOne\n      }\n",
        );
        out.push_str("      \\new TabStaff { \\removeWithTag #'staff \\VoiceOne }\n");
    }
    out.push_str("    >>\n");
    out.push_str("  >>\n");
    // The last system is spaced naturally instead of being stretched to
    // the full page width.
    out.push_str("  \\layout {\n    ragged-last = ##t\n  }\n");
    out.push_str("}\n");
    (out, warnings)
}

/// Volta structure, one entry per bar of the whole tab.  A `:|` closes the
/// section that began at the last `|:` or, implicitly, just after the
/// previous `:|` (or at the beginning).  Bars carrying a volta ending
/// number (from a `+N` marker) become the alternatives of the section: the
/// body ends just before the first of them and each run of equal numbers
/// forms one `\volta`.
pub(crate) fn volta_marks(tab: &Tab, warnings: &mut Vec<String>) -> Vec<VoltaBar> {
    let bars: Vec<&Bar> = tab.systems.iter().flat_map(|s| s.bars.iter()).collect();
    let total = bars.len();
    let mut marks = vec![VoltaBar::default(); total];
    let mut used = vec![false; total];
    let mut section_start: Option<usize> = None;
    let mut section_from = 0usize;
    for idx in 0..total {
        if bars[idx].repeat_start {
            if section_start.is_some() {
                warnings.push("ignoring nested repeat start (|:)".to_string());
            } else {
                section_start = Some(idx);
            }
        }
        if bars[idx].close != Barline::RepeatEnd {
            continue;
        }
        let from = section_start.take().unwrap_or(section_from);
        // The alternatives region: the contiguous run of ending-marked bars
        // containing the `:|`, never reaching back before the section start.
        let mut first = idx;
        while first > from && bars[first - 1].ending.is_some() {
            first -= 1;
        }
        let mut last = idx;
        while last + 1 < total && bars[last + 1].ending.is_some() {
            last += 1;
        }
        if bars[idx].ending.is_some() && first > from {
            // Group the region into runs of equal ending number.
            let mut runs: Vec<(usize, usize, usize)> = Vec::new();
            let mut b = first;
            while b <= last {
                let num = bars[b].ending.unwrap_or(0);
                let mut end = b;
                while end < last && bars[end + 1].ending == Some(num) {
                    end += 1;
                }
                runs.push((num, b, end));
                b = end + 1;
            }
            marks[from].start = Some(runs.len().max(2));
            marks[first - 1].body_end = true;
            marks[first].alt_list_start = true;
            for &(num, start, end) in &runs {
                marks[start].alt_start = Some(num);
                marks[end].alt_end = true;
                for mark in &mut used[start..=end] {
                    *mark = true;
                }
            }
            marks[last].alt_list_end = true;
            section_from = last + 1;
        } else {
            if from == idx {
                marks[idx].start = Some(2);
            } else {
                marks[from].start = Some(2);
            }
            marks[idx].body_end = true;
            section_from = idx + 1;
        }
    }
    if section_start.is_some() {
        warnings.push("ignoring repeat start (|:) without an end".to_string());
    }
    if bars.iter().any(|bar| bar.ending.is_some()) && !used.iter().any(|&u| u) {
        warnings.push("ignoring volta endings that do not end a repeated section".to_string());
    }
    marks
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Tie state carried between events while rendering one voice.
struct VoiceState {
    last_sig: Option<TimeSig>,
    last_event_id: usize,
    strings: [Option<StringState>; STRING_COUNT],
}

/// The note currently sounding on one string.
struct StringState {
    event_id: usize,
    midi: i32,
    name: String,
}

impl VoiceState {
    fn new() -> Self {
        VoiceState {
            last_sig: None,
            last_event_id: 0,
            strings: [None, None, None, None, None, None],
        }
    }
}

/// One event in a voice's timeline: new notes and/or tied continuations.
pub(crate) struct Event {
    /// 0-based column within the system.
    pub(crate) abs: usize,
    pub(crate) bar: usize,
    /// `(string, fret, arpeggiated)` per struck string.
    pub(crate) onsets: Vec<(usize, u8, bool)>,
    pub(crate) ties: Vec<usize>,
    pub(crate) duration: usize,
}

/// The immutable context for rendering one voice of one piece.
struct Piece<'a> {
    tab: &'a Tab,
    voice: Voice,
    key_map: Option<&'a SpellingMap>,
    /// One entry per bar of the whole tab.
    volta: &'a [VoltaBar],
}

fn render_voice_piece(
    piece: &Piece,
    state: &mut VoiceState,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut lines = Vec::new();
    // The tempo mark is printed once, above the conventional staff; the
    // tablature staff strips it with \removeWithTag.
    if piece.voice != Voice::Lower
        && let Some(tempo) = piece.tab.tempo
    {
        lines.push(format!("\\tag #'staff {{ \\tempo 4 = {tempo} }}"));
    }
    if piece.voice != Voice::Lower
        && let Some(key) = piece.tab.key
    {
        let root = Spelling {
            letter: key.letter,
            acc: key.acc,
        };
        let mode = if key.minor { "minor" } else { "major" };
        lines.push(format!("\\key {} \\{}", pitch_class_name(root), mode));
    }
    let mut bar_offset = 0usize;
    for (si, system) in piece.tab.systems.iter().enumerate() {
        if piece.voice != Voice::Lower && state.last_sig != Some(system.time_sig) {
            lines.push(format!(
                "\\time {}/{}",
                system.time_sig.num, system.time_sig.den
            ));
            state.last_sig = Some(system.time_sig);
        }
        lines.extend(render_voice_system(
            piece, system, si, bar_offset, state, warnings,
        ));
        bar_offset += system.bars.len();
    }
    lines
}

fn render_voice_system(
    piece: &Piece,
    system: &System,
    system_index: usize,
    bar_offset: usize,
    state: &mut VoiceState,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let tab = piece.tab;
    let voice = piece.voice;
    let key_map = piece.key_map;
    let last_system = tab.systems.len() - 1;
    let n_bars = system.bars.len();
    // Each bar may have its own width, so the time value of one column and
    // the beat positions are per bar.
    let timings: Vec<(Rational, usize)> = system
        .bars
        .iter()
        .map(|bar| {
            let unit = system.time_sig.bar_len() / Rational::integer(bar.width as i64);
            let beat_cols = if bar.width.is_multiple_of(system.time_sig.num as usize) {
                bar.width / system.time_sig.num as usize
            } else {
                0
            };
            (unit, beat_cols)
        })
        .collect();
    let events = build_events(system, voice, warnings);
    // A bar in which this voice has no notes at all is filled with invisible
    // rests (skips): the README specifies this for bars where every note
    // belongs to the other voice.
    let mut voiced = vec![false; n_bars];
    for event in &events {
        voiced[event.bar] = true;
    }

    let mut tokens: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut tuplets = TupletGroups::new(beat_len(&timings[0]));
    let mut pos = 0usize;
    let mut bar = 0usize;
    push_volta_opens(&mut tokens, &piece.volta[bar_offset]);

    for event in &events {
        // Tie marks must continue the immediately preceding event with no
        // gap in between.
        let mut tied: Vec<(usize, i32, String)> = Vec::new();
        for &string in &event.ties {
            match &state.strings[string] {
                Some(note) if note.event_id == state.last_event_id && pos == event.abs => {
                    tied.push((string, note.midi, note.name.clone()));
                }
                _ => warnings.push(format!(
                    "ignoring tie mark on string {}: no note to tie from at this point",
                    string + 1
                )),
            }
        }
        let mut tie_pending = !tied.is_empty();

        // Close any bars that end before this event.
        while bar < n_bars && system.bars[bar].start + system.bars[bar].width <= event.abs {
            let bar_end = system.bars[bar].start + system.bars[bar].width;
            if pos < bar_end {
                let prefix = if voiced[bar] { 'r' } else { 's' };
                tuplets.push_filler(
                    &mut tokens,
                    prefix,
                    &note_durs(
                        col_of(pos, &system.bars[bar]),
                        bar_end - pos,
                        timings[bar].1,
                        timings[bar].0,
                    ),
                );
            }
            pos = bar_end;
            if tie_pending {
                tuplets.tie_last(&mut tokens);
                tie_pending = false;
            }
            let suffix = bar_suffix(system, bar, n_bars, voice, system_index == last_system);
            // The closing brace of a volta repeat draws the `:|` barline
            // itself; so do the braces around alternatives.
            let suffix = close_volta_blocks(suffix, &piece.volta[bar_offset + bar]);
            tuplets.close(&mut tokens);
            flush_bar(&mut tokens, &mut lines, &suffix);
            bar += 1;
            if bar < n_bars {
                tuplets.beat = beat_len(&timings[bar]);
                push_volta_opens(&mut tokens, &piece.volta[bar_offset + bar]);
            }
        }
        if event.abs > pos {
            tuplets.push_filler(
                &mut tokens,
                'r',
                &note_durs(
                    col_of(pos, &system.bars[bar]),
                    event.abs - pos,
                    timings[bar].1,
                    timings[bar].0,
                ),
            );
        }
        if tie_pending {
            tuplets.tie_last(&mut tokens);
        }

        // Tied notes keep their spelling; onsets are spelled in the context
        // of the active chord, falling back to the key. An arpeggio mark is
        // engraved in the tablature staff only.
        let mut notes: Vec<(usize, i32, String)> = tied;
        let arpeggio = event.onsets.iter().any(|&(_, _, arp)| arp);
        for &(string, fret, _) in &event.onsets {
            let midi = tab.tuning.open_midi[string] + fret as i32;
            let chord_map = active_chord(
                system,
                event.bar,
                col_of(event.abs, &system.bars[event.bar]),
            )
            .map(|chord| chord_spelling_map(chord.root, chord.qual));
            let name = lily_note_name(midi, spell(midi, chord_map.as_ref(), key_map));
            notes.push((string, midi, name));
        }
        notes.sort_by_key(|&(_, midi, _)| midi);

        let event_id = state.last_event_id + 1;
        if notes.is_empty() {
            tuplets.push_filler(
                &mut tokens,
                'r',
                &note_durs(
                    col_of(event.abs, &system.bars[event.bar]),
                    event.duration,
                    timings[event.bar].1,
                    timings[event.bar].0,
                ),
            );
        } else {
            let (pitch_text, string_suffix) = chord_text(&notes);
            let durs = note_durs(
                col_of(event.abs, &system.bars[event.bar]),
                event.duration,
                timings[event.bar].1,
                timings[event.bar].0,
            );
            for (i, dur) in durs.iter().enumerate() {
                if i > 0 {
                    tuplets.tie_last(&mut tokens);
                }
                let mut token = format!("{pitch_text}{}{string_suffix}", dur.to_lily());
                // The arpeggio marks the attack, so only the first of a
                // chain of tied notes carries it.
                if arpeggio && i == 0 {
                    token.push_str("\\tag #'tab \\arpeggio");
                }
                tuplets.push(&mut tokens, *dur, token);
            }
            for &(string, midi, ref name) in &notes {
                state.strings[string] = Some(StringState {
                    event_id,
                    midi,
                    name: name.clone(),
                });
            }
        }
        state.last_event_id = event_id;
        pos = event.abs + event.duration;
    }

    // Close any bars after the last event.
    while bar < n_bars {
        let bar_end = system.bars[bar].start + system.bars[bar].width;
        if pos < bar_end {
            let prefix = if voiced[bar] { 'r' } else { 's' };
            tuplets.push_filler(
                &mut tokens,
                prefix,
                &note_durs(
                    col_of(pos, &system.bars[bar]),
                    bar_end - pos,
                    timings[bar].1,
                    timings[bar].0,
                ),
            );
        }
        pos = bar_end;
        let suffix = bar_suffix(system, bar, n_bars, voice, system_index == last_system);
        let suffix = close_volta_blocks(suffix, &piece.volta[bar_offset + bar]);
        tuplets.close(&mut tokens);
        flush_bar(&mut tokens, &mut lines, &suffix);
        bar += 1;
        if bar < n_bars {
            tuplets.beat = beat_len(&timings[bar]);
            push_volta_opens(&mut tokens, &piece.volta[bar_offset + bar]);
        }
    }
    lines
}

/// Build a voice's event timeline for one system, merging simultaneous
/// events and computing durations.
pub(crate) fn build_events(
    system: &System,
    voice: Voice,
    warnings: &mut Vec<String>,
) -> Vec<Event> {
    let n_bars = system.bars.len();
    let mut raw: Vec<Event> = Vec::new();
    for bar in 0..n_bars {
        for string in 0..STRING_COUNT {
            for event in &system.bars[bar].strings[string] {
                match *event {
                    StringEvent::Onset {
                        col,
                        fret,
                        arpeggio,
                    } => {
                        if in_voice(voice, voice_split_at(&system.bars[bar], col), string) {
                            raw.push(Event {
                                abs: system.bars[bar].start + col - 1,
                                bar,
                                onsets: vec![(string, fret, arpeggio)],
                                ties: Vec::new(),
                                duration: 0,
                            });
                        }
                    }
                    StringEvent::Tie { col } => {
                        match last_string_event_before(system, string, bar, col) {
                            Some((source_bar, source_col)) => {
                                // A tie rings on in the voice of the note it
                                // continues, even when a `.` has since moved
                                // the split. A tie from an earlier bar rings
                                // from the barline; otherwise it splits at
                                // its own column.
                                if in_voice(
                                    voice,
                                    voice_split_at(&system.bars[source_bar], source_col),
                                    string,
                                ) {
                                    let abs = if source_bar < bar {
                                        system.bars[bar].start
                                    } else {
                                        system.bars[bar].start + col - 1
                                    };
                                    raw.push(Event {
                                        abs,
                                        bar,
                                        onsets: Vec::new(),
                                        ties: vec![string],
                                        duration: 0,
                                    });
                                }
                            }
                            None => {
                                if in_voice(voice, voice_split_at(&system.bars[bar], col), string) {
                                    warnings.push(format!(
                                        "ignoring tie mark on string {}: no earlier note on the string in this system",
                                        string + 1
                                    ));
                                }
                            }
                        }
                    }
                    // Split changes are folded into `Bar.split_changes`.
                    StringEvent::Split { .. } => {}
                }
            }
        }
    }
    raw.sort_by_key(|event| event.abs);
    let mut events: Vec<Event> = Vec::new();
    for event in raw {
        match events.last_mut() {
            Some(last) if last.abs == event.abs && last.bar == event.bar => {
                last.onsets.extend(event.onsets);
                last.ties.extend(event.ties);
            }
            _ => events.push(event),
        }
    }
    for event in &mut events {
        event.ties.sort_unstable();
        event.ties.dedup();
        // A string that starts a new note here cannot also be tied.
        event
            .ties
            .retain(|string| !event.onsets.iter().any(|(onset, _, _)| onset == string));
    }
    for i in 0..events.len() {
        let bar = &system.bars[events[i].bar];
        let bar_end = bar.start + bar.width;
        let next = events.get(i + 1).map_or(bar_end, |event| event.abs);
        events[i].duration = next.min(bar_end) - events[i].abs;
    }
    events
}

/// The bar of the last event on `string` before the tie mark at (`bar`,
/// `col`), searching within the current system only.
fn last_string_event_before(
    system: &System,
    string: usize,
    bar: usize,
    col: usize,
) -> Option<(usize, usize)> {
    for b in (0..=bar).rev() {
        for event in system.bars[b].strings[string].iter().rev() {
            if b == bar && event.col() >= col {
                continue;
            }
            return Some((b, event.col()));
        }
    }
    None
}

/// The voice split in effect at a column of a bar: the bar's `!` marker as
/// overridden by any `.` split change at or before that column.
fn voice_split_at(bar: &Bar, col: usize) -> Option<usize> {
    let mut split = bar.voice_split;
    for &(change_col, string) in &bar.split_changes {
        if change_col <= col {
            split = Some(string);
        }
    }
    split
}

/// Whether `string` belongs to `voice` under the given split. `None` means
/// the bar has no split: everything is one voice.
fn in_voice(voice: Voice, split: Option<usize>, string: usize) -> bool {
    match voice {
        Voice::Single => true,
        Voice::Upper => split.is_none_or(|s| string <= s),
        Voice::Lower => split.is_some_and(|s| string > s),
    }
}

fn col_of(abs: usize, bar: &Bar) -> usize {
    abs - bar.start + 1
}

/// The chord symbol sounding at (`bar`, `col`), if any.
fn active_chord(system: &System, bar: usize, col: usize) -> Option<Chord> {
    system
        .chords
        .iter()
        .take_while(|slot| slot.bar < bar || (slot.bar == bar && slot.col <= col))
        .last()
        .map(|slot| slot.chord)
}

/// The sounding length of one beat of a bar (zero when the bar width has no
/// beat grid).
fn beat_len(timing: &(Rational, usize)) -> Rational {
    timing.0 * Rational::integer(timing.1 as i64)
}

/// Emits tokens while wrapping runs of tuplet-duration tokens in
/// `\tuplet n/d { }` blocks.  A block closes when its contents fill a whole
/// number of beats (so triplet crotchets group in threes across two beats),
/// when a non-tuplet token arrives, or at the end of a bar.
struct TupletGroups {
    /// The ratio of the currently open block, if any.
    open: Option<(u32, u32)>,
    /// Sounding length accumulated inside the open block.
    filled: Rational,
    /// Length of one beat of the current bar (zero without a beat grid).
    beat: Rational,
}

impl TupletGroups {
    fn new(beat: Rational) -> Self {
        TupletGroups {
            open: None,
            filled: Rational::integer(0),
            beat,
        }
    }

    fn push(&mut self, tokens: &mut Vec<String>, dur: Dur, token: String) {
        if self.open != dur.tuplet {
            self.close(tokens);
            if let Some((num, den)) = dur.tuplet {
                tokens.push(format!("\\tuplet {num}/{den} {{"));
                self.filled = Rational::integer(0);
            }
            self.open = dur.tuplet;
        }
        tokens.push(token);
        if dur.tuplet.is_some() {
            self.filled = self.filled + dur.len();
            if self.beat.is_positive() && (self.filled / self.beat).is_integer() {
                self.close(tokens);
            }
        }
    }

    /// Push rest or skip tokens covering a stretch of time (`prefix` is `r`
    /// or `s`).
    fn push_filler(&mut self, tokens: &mut Vec<String>, prefix: char, durs: &[Dur]) {
        for dur in durs {
            self.push(tokens, *dur, format!("{prefix}{}", dur.to_lily()));
        }
    }

    /// Attach a tie to the most recent note token, looking past a tuplet
    /// block that closed right after it.
    fn tie_last(&self, tokens: &mut Vec<String>) {
        if tokens.last().map(String::as_str) == Some("}") {
            tokens.pop();
            if let Some(note) = tokens.last_mut() {
                note.push('~');
            }
            tokens.push("}".to_string());
        } else if let Some(last) = tokens.last_mut() {
            last.push('~');
        }
    }

    fn close(&mut self, tokens: &mut Vec<String>) {
        if self.open.take().is_some() {
            tokens.push("}".to_string());
        }
    }
}

/// Pitch text for one note event, plus a suffix to place after the duration.
/// LilyPond wants `c4\5` for single notes (duration before the string number)
/// but `<c\5 e\4 g\3>1` for chords (string numbers inside, duration outside).
/// Every note carries its string number so the tablature staff derives each
/// fret from the string the tab actually uses instead of guessing a
/// first-position fingering (`minimumFret` only steers that guesswork, and
/// the automatic selector may put two chord notes on one string).  The
/// numbers are not tagged individually: the conventional staff hides them
/// with the variable's single tagged `\omit StringNumber`.
fn chord_text(notes: &[(usize, i32, String)]) -> (String, String) {
    if notes.len() == 1 {
        let &(string, _, ref name) = &notes[0];
        // String 1 is the highest string, which is index 0 in the tuning.
        (name.clone(), format!("\\{}", string + 1))
    } else {
        let names: Vec<String> = notes
            .iter()
            .map(|&(string, _, ref name)| format!("{name}\\{}", string + 1))
            .collect();
        (format!("<{}>", names.join(" ")), String::new())
    }
}

/// Push the tokens that open the volta blocks starting at a bar.
fn push_volta_opens(tokens: &mut Vec<String>, volta: &VoltaBar) {
    if let Some(count) = volta.start {
        tokens.push(format!("\\repeat volta {count} {{"));
    }
    if volta.alt_list_start {
        tokens.push("\\alternative {".to_string());
    }
    if let Some(num) = volta.alt_start {
        tokens.push(format!("\\volta {num} {{"));
    }
}

/// Prepend the closing braces of the volta blocks that end at a bar, in
/// nesting order (innermost first). A `\break` at the end of the repeat
/// body must stay inside it: LilyPond associates an `\alternative` with a
/// `\repeat` only when the body's closing brace immediately precedes it,
/// so the body's `}` is emitted after the break.
fn close_volta_blocks(suffix: String, volta: &VoltaBar) -> String {
    if volta.body_end {
        let (head, brk) = match suffix.strip_suffix("\\break") {
            Some(head) => (head.trim_end(), true),
            None => (suffix.as_str(), false),
        };
        let mut out = String::new();
        if !head.is_empty() {
            out.push_str(head);
            out.push(' ');
        }
        if brk {
            out.push_str("\\break ");
        }
        out.push('}');
        return out;
    }
    let mut out = suffix;
    for close in [volta.alt_end, volta.alt_list_end] {
        if close {
            out = if out.is_empty() {
                "}".to_string()
            } else {
                format!("}} {out}")
            };
        }
    }
    out
}

/// Barline and line break directives that close a bar, if any.
fn bar_suffix(
    system: &System,
    bar: usize,
    n_bars: usize,
    voice: Voice,
    is_last_system: bool,
) -> String {
    if bar + 1 != n_bars {
        return String::new();
    }
    let mut suffix = String::new();
    if voice != Voice::Lower {
        match system.final_barline {
            // A repeat end is drawn by the closing brace of the volta
            // repeat itself.
            Barline::RepeatEnd => {}
            Barline::Double => suffix.push_str("\\bar \"||\""),
            Barline::Single => {}
        }
    }
    if !is_last_system {
        if !suffix.is_empty() {
            suffix.push(' ');
        }
        suffix.push_str("\\break");
    }
    suffix
}

fn flush_bar(tokens: &mut Vec<String>, lines: &mut Vec<String>, suffix: &str) {
    tokens.push("|".to_string());
    let mut line = tokens.join(" ");
    if !suffix.is_empty() {
        line.push(' ');
        line.push_str(suffix);
    }
    lines.push(line);
    tokens.clear();
}

/// The chord symbol line for each system.
fn chord_lines(tab: &Tab, volta: &[VoltaBar]) -> Vec<String> {
    let mut lines = Vec::new();
    let last_system = tab.systems.len() - 1;
    let mut bar_offset = 0usize;
    for (si, system) in tab.systems.iter().enumerate() {
        let mut slots: Vec<(usize, &Chord)> = system
            .chords
            .iter()
            .map(|slot| (system.bars[slot.bar].start + slot.col - 1, &slot.chord))
            .collect();
        slots.sort_by_key(|&(abs, _)| abs);
        for bar in 0..system.bars.len() {
            let bar_ref = &system.bars[bar];
            let start = bar_ref.start;
            let end = start + bar_ref.width;
            let unit = system.time_sig.bar_len() / Rational::integer(bar_ref.width as i64);
            let beat_cols = if bar_ref.width.is_multiple_of(system.time_sig.num as usize) {
                bar_ref.width / system.time_sig.num as usize
            } else {
                0
            };
            let mut tokens: Vec<String> = Vec::new();
            let mut tuplets = TupletGroups::new(unit * Rational::integer(beat_cols as i64));
            push_volta_opens(&mut tokens, &volta[bar_offset]);
            let mut pos = start;
            for (i, &(abs, chord)) in slots.iter().enumerate() {
                if abs < start {
                    continue;
                }
                if abs >= end {
                    break;
                }
                if abs > pos {
                    tuplets.push_filler(
                        &mut tokens,
                        's',
                        &note_durs(col_of(pos, bar_ref), abs - pos, beat_cols, unit),
                    );
                }
                let next = slots
                    .get(i + 1)
                    .map_or(end, |&(next, _)| next)
                    .min(end)
                    .max(abs + 1);
                let durs = note_durs(col_of(abs, bar_ref), next - abs, beat_cols, unit);
                for (di, dur) in durs.iter().enumerate() {
                    if di > 0 {
                        tuplets.tie_last(&mut tokens);
                    }
                    tuplets.push(&mut tokens, *dur, chord.lily_with_duration(&dur.to_lily()));
                }
                pos = next;
            }
            if pos < end {
                tuplets.push_filler(
                    &mut tokens,
                    's',
                    &note_durs(col_of(pos, bar_ref), end - pos, beat_cols, unit),
                );
            }
            let mut suffix = String::new();
            if bar + 1 == system.bars.len() && si != last_system {
                suffix.push_str("\\break");
            }
            let suffix = close_volta_blocks(suffix, &volta[bar_offset]);
            tuplets.close(&mut tokens);
            flush_bar(&mut tokens, &mut lines, &suffix);
            bar_offset += 1;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use crate::parser::{Barline, parse};

    use super::generate;

    const README_EXAMPLE: &str = "\
{signature:4/4}
|----------2---5-|
|------4---------|
|--2-------------|
!4---------------|
|----2---4-------|
|2-5-------------|
";

    #[test]
    fn readme_example_lilypond() {
        let tab = parse(README_EXAMPLE).unwrap();
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert!(lilypond.contains("\\version \"2.24.0\""));
        // The music is defined once in top level variables shared by both
        // staves; each staff strips the tags it does not want.
        assert!(lilypond.contains("VoiceOne = {"));
        assert!(lilypond.contains("VoiceTwo = {"));
        assert!(lilypond.contains("\\new Staff <<"));
        assert!(lilypond.contains("\\clef \"treble_8\""));
        assert!(lilypond.contains("\\voiceOne"));
        assert!(lilypond.contains("\\voiceTwo"));
        assert!(lilypond.contains("\\time 4/4"));
        assert!(lilypond.contains("\\new Voice = \"upper\" { \\removeWithTag #'tab \\VoiceOne }"));
        assert!(
            lilypond.contains("\\new TabVoice = \"lower\" { \\removeWithTag #'staff \\VoiceTwo }")
        );
        // Treble: a quaver, three pairs of tied quavers and a final quaver.
        // Every note carries its string number, so the tablature staff
        // derives the frets the tab specifies instead of guessing a
        // first-position fingering; the conventional staff hides the
        // numbers with the variable's tagged \omit StringNumber.
        assert!(
            lilypond
                .contains("fis8\\4 a8\\3~ a8\\3 dis'8\\2~ dis'8\\2 fis'8\\1~ fis'8\\1 a'8\\1 |",)
        );
        // Bass: two quavers, a crotchet and a minim.
        assert!(lilypond.contains("fis,8\\6 a,8\\6 b,4\\5 cis2\\5 |"));
        assert!(lilypond.contains("\\tag #'staff { \\omit StringNumber }"));
        assert!(lilypond.contains("\\new StaffGroup <<"));
        assert!(lilypond.contains("\\new TabStaff <<"));
        // The score is laid out for A4 paper with a 20 mm left margin, a
        // 180 mm line width and no first-system indent, and the last
        // system is not stretched to the full page width.
        assert!(lilypond.contains(
            "\\paper {\n  #(set-paper-size \"a4\")\n  left-margin = 20\n  line-width = 180\n  indent = #0\n}"
        ));
        // The tagline is a header field, so the header block is always
        // emitted, even without any metadata.
        assert!(lilypond.contains("\\header {\n  tagline = ##f\n}"));
        assert!(lilypond.contains("\\layout {\n    ragged-last = ##t\n  }"));
    }

    #[test]
    fn single_voice_lilypond() {
        let tab = parse(
            "\
{signature:4/4}
|0-------5-------|
|----------------|
|----------------|
|----------------|
|----------------|
|3---------------|
",
        )
        .unwrap();
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // Without a voice split one variable holds the music and both staves
        // take it directly.
        assert!(lilypond.contains("\\new Staff {"));
        assert!(lilypond.contains("\\new TabStaff {"));
        assert!(!lilypond.contains("\\new Voice"));
        assert!(!lilypond.contains("\\new TabVoice"));
        assert!(lilypond.contains("\\removeWithTag #'tab \\VoiceOne"));
        assert!(lilypond.contains("\\new TabStaff { \\removeWithTag #'staff \\VoiceOne }"));
        assert!(lilypond.contains("<g,\\6 e'\\1>2 a'2\\1 |"));
    }

    #[test]
    fn aura_lee_lilypond() {
        let tab = parse(include_str!("../examples/aura_lee.tab")).unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert!(lilypond.contains("title = \"Aura Lee\""));
        assert!(lilypond.contains("composer = \"Traditional\""));
        assert!(lilypond.contains("arranger = \"Chris Proctor\""));
        assert!(lilypond.contains("\\key c \\major"));
        assert!(lilypond.contains("\\clef \"treble_8\""));
        assert!(lilypond.contains("\\new ChordNames \\chordmode {"));
        assert!(lilypond.contains("c1 |"));
        assert!(lilypond.contains("d1/fis |"));
        assert!(lilypond.contains("c2 e2:7 |"));
        assert!(lilypond.contains("a2:m c2:7 |"));
        assert!(lilypond.contains("f2:maj7 g2 |"));
        assert!(lilypond.contains("c2 a2:7 |"));
        assert!(lilypond.contains("g1 |"));
        assert!(lilypond.contains("s1 |"));
        // The music is defined once per voice; every note carries its string
        // number, so the tablature staff derives the frets the tab specifies
        // and the conventional staff hides the numbers with the tagged omit.
        // First bar: C major shape in the treble, Travis-picking bass.
        assert!(
            lilypond.contains("<g\\3 c'\\2 e'\\1>4 <g\\3 c'\\2 e'\\1>4 <gis\\3 d'\\2 e'\\1>2 |")
        );
        assert!(lilypond.contains("c4\\5 e4\\4 e,4\\6 e4\\4 |"));
        // The tie over the barline at the end of the first system.
        assert!(lilypond.contains("c'4\\2 b4\\2 a8\\3 b8\\2~ b8\\2 c'8\\2~ |"));
        // The first system is a repeated section: it opens with a volta at
        // the first bar and the `:|` closes it, so the barline itself is
        // drawn by the repeat and both voices wrap the same bars.
        assert!(lilypond.contains("\\repeat volta 2 { c2 a2:7 |"));
        assert!(lilypond.contains("c'1\\2 | \\break }"));
        assert!(lilypond.contains("\\repeat volta 2 { g4\\3 c'4\\2 b8\\2 c'8\\2~ c'4\\2 |"));
        assert!(lilypond.contains("\\repeat volta 2 { c4\\5 e4\\4 c4\\5 e4\\4 |"));
        assert!(!lilypond.contains("\\bar \":|\""));
        // Bar 9 changes to A7 half way through, so the first-string F is
        // spelled F natural (the key's spelling) rather than E sharp; the
        // move to the chord's E natural then avoids an E sharp to E motion.
        assert!(
            lilypond
                .contains("<g\\3 cis'\\2 f'\\1>8~ <g\\3 cis'\\2 e'\\1>8~ <g\\3 cis'\\2 e'\\1>4 |")
        );
        // The final bar is marked single voice (`!` on the bottom string):
        // the whole C major chord sounds in the treble while the bass is
        // filled with an invisible rest.
        assert!(lilypond.contains("<c\\5 e\\4 g\\3 c'\\2>1\\tag #'tab \\arpeggio | \\bar \"||\""));
        // The third bar of the last system has no marker, so it keeps the
        // split of the bar before it.
        assert!(lilypond.contains("c'4\\2 b4\\2 a8\\3 b8\\2~ b4\\2 |"));
        assert!(lilypond.contains("g,4\\6 d4\\4 g,4\\6 d4\\4 |"));
        assert!(lilypond.contains("\\new TabStaff <<"));
        // The `.` in the eighth column of the second system's third bar moves
        // the split to the third string: the G string joins the treble voice
        // from the next note on.
        assert!(lilypond.contains("<c'\\2 e'\\1>4 d'4\\2 <g\\3 c'\\2>8 d'8\\2~ d'8\\2 e'8\\1~ |"));
        assert!(lilypond.contains("f4\\4 a4\\3 g,4\\6 d4\\4 |"));
        // Tab staves re-enable arpeggio printing; the override is tagged so
        // only the tablature staff keeps it.
        assert!(
            lilypond.contains("\\tag #'tab { \\override Arpeggio.stencil = #ly:arpeggio::print }")
        );
    }

    #[test]
    fn double_digit_lilypond() {
        // The second README example: double digit frets, each note starting
        // at the first digit and taking two columns.
        let tab = parse(
            "\
|--------|
|------11|
|----10--|
|--11----|
|12------|
|--------|
",
        )
        .unwrap();
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert!(lilypond.contains("a4\\5 cis'4\\4 f'4\\3 ais'4\\2 |"));
    }

    #[test]
    fn triplet_grid_lilypond() {
        // The third README example: a 24 column bar in 4/4 holds eight
        // quavers three columns apart, or six triplet crotchets four
        // columns apart.
        let tab = parse(
            "\
{signature:4/4}
|------------------------|------------------------|
|------------------------|------------------------|
|------------------------|------------------------|
|------------------------|------------------------|
|------------------------|------------------------|
|1--1--1--1--1--1--1--1--|1---1---1---1---1---1---|
",
        )
        .unwrap();
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // Eight quavers in the first bar.
        assert!(lilypond.contains("f,8\\6 f,8\\6 f,8\\6 f,8\\6 f,8\\6 f,8\\6 f,8\\6 f,8\\6 |"));
        // Six triplet crotchets in the second bar, grouped in threes.
        assert!(lilypond.contains(
            "\\tuplet 3/2 { f,4\\6 f,4\\6 f,4\\6 } \\tuplet 3/2 { f,4\\6 f,4\\6 f,4\\6 } |"
        ));
    }

    #[test]
    fn cant_help_falling_in_love_lilypond() {
        // The opening system of examples/cant_help_falling_in_love.tab: a
        // 24 column grid whose second bar mixes quavers with triplet
        // crotchets and whose third holds nothing but triplet lengths.
        let tab = parse(
            "\
{title:Can't Help Falling In Love}
{tempo:90}

|------------------------|----------------0---1---|------------------------|
|-----------.0-----------!1-----------3-----------|------------------------|
!2-----------------0-----|------0---------0---2---|------------------------|
|------3--------0-----0--|---2-----2--0-----------|------------------------|
|---3-----3--------------|------------------------|------------------------|
|1-----------1-----------|0-----------------------|0-------0---------------|
",
        )
        .unwrap();
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // Bar 1: eight quavers in the bass under two minims in the treble.
        assert!(lilypond.contains("a2\\3 b2\\2 |"));
        assert!(lilypond.contains("f,8\\6 c8\\5 f8\\4 c8\\5 f,8\\6 d8\\4 g8\\3 d8\\4 |"));
        // Bar 2: four quavers then three triplet crotchets in both voices.
        assert!(lilypond.contains("c'2\\2 \\tuplet 3/2 { d'4\\2 e'4\\1 f'4\\1 } |"));
        assert!(lilypond.contains("e,8\\6 e8\\4 g8\\3 e8\\4 \\tuplet 3/2 { d4\\4 g4\\3 a4\\3 } |"));
        // Bar 3: a triplet minim and a triplet semibreve in the bass.
        assert!(lilypond.contains("\\tuplet 3/2 { e,2\\6 e,1\\6 } |"));
    }

    #[test]
    fn suicide_is_painless_lilypond() {
        let tab = parse(include_str!("../examples/suicide_is_painless.tab")).unwrap();
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert!(lilypond.contains("\\tempo 4 = 110"));
        // The tempo mark is tagged `staff`: the conventional staff keeps it
        // and the tablature staff strips it, so it is printed only once.
        assert!(lilypond.contains("\\tag #'staff { \\tempo 4 = 110 }"));
        // The `|:` before the second bar opens the repeated section that
        // the `:|` in the last system closes; the single voice variable is
        // shared by the staff and the tab staff, so both repeat together.
        assert_eq!(lilypond.matches("\\repeat volta 2 {").count(), 1);
        // The endings line turns the last system's two bars into the two
        // alternatives of the repeat. The body's closing brace comes after
        // the line break (a `\\break` between the body and the alternative
        // would stop LilyPond associating them), volta 1 ends with the `:|`
        // that the repeat draws itself, and volta 2 closes into the final
        // double barline.
        assert_eq!(lilypond.matches("\\alternative {").count(), 1);
        assert_eq!(lilypond.matches("\\volta 1 {").count(), 1);
        assert_eq!(lilypond.matches("\\volta 2 {").count(), 1);
        assert!(lilypond.contains("a'4\\1 g'4\\1 | \\break }"));
        assert!(lilypond.contains("\\alternative { \\volta 1 { <a,\\5 a'\\1>2. g'4\\1 | }"));
        assert!(lilypond.contains("\\volta 2 { <a,\\5 a'\\1>1 | } } \\bar \"||\""));
        // Bar 15 ends with the double digit fret 10.
        assert!(
            lilypond
                .contains("<d\\4 a\\3 c'\\2 f'\\1>4 g'8\\1 a'8\\1 a,8\\5 a'8\\1 c''8\\1 d''8\\1 |")
        );
    }

    #[test]
    fn repeat_lilypond() {
        // The seventh README example: `|:` starts a repeated section and
        // `:|` ends it; the volta draws the repeat barlines itself.
        let tab = parse(
            "\
{signature:4/4}
|:0-------0-------|0-------0-------:|
|:----------------|----------------:|
|:----------------|----------------:|
|:----------------|----------------:|
|:----------------|----------------:|
|:----------------|----------------:|
",
        )
        .unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        assert!(tab.systems[0].bars[0].repeat_start);
        assert_eq!(tab.systems[0].bars[1].close, Barline::RepeatEnd);
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // The volta lives in the single shared voice variable, so both
        // staves repeat together.
        assert_eq!(
            lilypond
                .matches("\\repeat volta 2 { e'2\\1 e'2\\1 |")
                .count(),
            1
        );
        assert_eq!(lilypond.matches("e'2\\1 e'2\\1 | }").count(), 1);
        assert!(!lilypond.contains("\\bar"));
    }

    #[test]
    fn bang_colon_repeat_lilypond() {
        // `!:` starts the repeat on the bar that also sets the voice split,
        // so the volta is engraved and the split produces two voices.
        let tab = parse(
            "\
{signature:4/4}
|0-------|0-------:|
|--------|--------:|
!:1-------|1-------:|
|--------|--------:|
|--------|--------:|
|--------|--------:|
",
        )
        .unwrap();
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // The volta appears once per voice variable, and both staves share
        // each variable.
        assert_eq!(lilypond.matches("\\repeat volta 2 {").count(), 2);
        assert!(lilypond.contains("\\new Voice = \"upper\""));
        assert!(lilypond.contains("\\new Voice = \"lower\""));
    }

    #[test]
    fn volta_endings_lilypond() {
        // The ninth README example: the endings share their system with
        // the body, and the indented endings line puts `+1` on the third
        // bar's barline and `+2` on the fourth's.
        let tab = parse(
            "\
{signature:4/4}
                                   +1----------------+2---------------+
|:0-------0-------|0-------0-------|0-------0-------:|0-------0-------|0---|
|:----------------|----------------|----------------:|----------------|----|
|:----------------|----------------|----------------:|----------------|----|
|:----------------|----------------|----------------:|----------------|----|
|:----------------|----------------|----------------:|----------------|----|
|:----------------|----------------|----------------:|----------------|----|
",
        )
        .unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(lilypond.matches("\\repeat volta 2 {").count(), 1);
        assert_eq!(lilypond.matches("\\alternative {").count(), 1);
        // The body ends mid-system, so its closing brace carries no line
        // break and the alternatives continue in the same system.
        assert!(lilypond.contains("\\repeat volta 2 { e'2\\1 e'2\\1 |\n  e'2\\1 e'2\\1 | }"));
        assert!(lilypond.contains("\\alternative { \\volta 1 { e'2\\1 e'2\\1 | }"));
        assert!(lilypond.contains("\\volta 2 { e'2\\1 e'2\\1 |\n  e'1\\1 | } }"));
        // The system ends with a plain barline and has no line breaks.
        assert!(!lilypond.contains("\\bar"));
        assert!(!lilypond.contains("\\break"));
    }

    #[test]
    fn mixed_width_lilypond() {
        // The first README example: the same four crotchets written in bars
        // of four different widths.
        let tab = parse(
            "\
{signature:4/4}
|----|--------|----------------|--------------------------------|
|---6|------6-|------------6---|------------------------6-------|
|--5-|----5---|--------5-------|----------------5---------------|
|-7--|--7-----|----7-----------|--------7-----------------------|
|8---|8-------|8---------------|8-------------------------------|
|----|--------|----------------|--------------------------------|
",
        )
        .unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // Every bar holds the same four crotchet notes however wide it is.
        let bar = "f4\\5 a4\\4 c'4\\3 f'4\\2 |";
        assert_eq!(lilypond.matches(bar).count(), 4);
    }

    #[test]
    fn arpeggio_lilypond() {
        // The fifth README example: `~` before a fret number marks an
        // arpeggiated note that starts on the `~`'s column.
        let tab = parse(
            "\
{signature:4/4}
|~0--------------|
|~1--------------|
|~2--------------|
|----------------|
|----------------|
|----------------|
",
        )
        .unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // The chord starts on the first column (a whole note).  The arpeggio
        // mark is tagged `tab`: the tablature staff engraves it (after
        // re-enabling the arpeggio stencil) and the conventional staff
        // strips it.
        assert!(lilypond.contains("<a\\3 c'\\2 e'\\1>1\\tag #'tab \\arpeggio |"));
        assert!(
            lilypond.contains("\\tag #'tab { \\override Arpeggio.stencil = #ly:arpeggio::print }")
        );
    }

    #[test]
    fn dot_split_change_lilypond() {
        // The second README example: the split starts at the third string and
        // the `.` moves it to the fourth string from the eighth column on.
        let tab = parse(
            "\
{signature:4/4}
|----------------|
|--------------2-|
!2-4---2---2-4---|
|----2--.4-------|
|------------2---|
|0-------3-------|
",
        )
        .unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // Treble: the fourth-string F sharp only after the split change.
        assert!(lilypond.contains("a8\\3 b8\\3~ b8\\3 a8\\3 fis8\\4 a8\\3 b8\\3 cis'8\\2 |"));
        // Bass: alternating crotchets, the low E open and third fret.
        assert!(lilypond.contains("e,4\\6 e4\\4 g,4\\6 b,4\\5 |"));
    }

    #[test]
    fn voice_split_persists_lilypond() {
        // The third README example: the `!` only marks the first bar, and the
        // second bar (with no metadata line either, so 4/4 by default) keeps
        // the split.
        let tab = parse(
            "\
|0-------1-------|3-----1-3-------|
|--1---3---1---3-|--0-3-----3-0---|
!----0-------0---|--------------0-|
|----2-------2---|----0-------0---|
|3-------3-------|----------------|
|----------------|3-------3-------|
",
        )
        .unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert!(lilypond.contains("\\time 4/4"));
        // First bar: alternating melody and bass.
        assert!(lilypond.contains("e'8\\1 c'8\\2 g8\\3 d'8\\2 f'8\\1 c'8\\2 g8\\3 d'8\\2 |"));
        assert!(lilypond.contains("c4\\5 e4\\4 c4\\5 e4\\4 |"));
        // Second bar keeps the split: the bass still has its own notes
        // instead of everything merging into one voice.
        assert!(lilypond.contains("g'8\\1 b8\\2 d'8\\2 f'8\\1 g'8\\1 d'8\\2 b8\\2 g8\\3 |"));
        assert!(lilypond.contains("g,4\\6 d4\\4 g,4\\6 d4\\4 |"));
    }

    #[test]
    fn single_voice_bar_invisible_rests() {
        // The fourth README example: the second bar is marked single voice
        // with a `!` on the bottom string, so the whole chord sounds in the
        // treble and the bass gets invisible rests.
        let tab = parse(
            "\
{signature:4/4}
|----------------|----------------|
|1---0-----0-----|1---------------|
!--------2-------|0---------------|
|----0-------0---|2---------------|
|----------------|3---------------|
|3-------3-------!----------------|
",
        )
        .unwrap();
        assert!(tab.warnings.is_empty(), "warnings: {:?}", tab.warnings);
        let (lilypond, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert!(lilypond.contains("c'4\\2 b4\\2 a8\\3 b8\\2~ b4\\2 |"));
        assert!(lilypond.contains("g,4\\6 d4\\4 g,4\\6 d4\\4 |"));
        assert!(lilypond.contains("<c\\5 e\\4 g\\3 c'\\2>1 |"));
        assert!(lilypond.contains("s1 |"));
        assert!(
            !lilypond.contains("r1"),
            "rests must be invisible: {lilypond}"
        );
    }
}
