//! LilyPond source generation.
//!
//! Each system of tab becomes a line of music.  Note timings come from the
//! horizontal grid: one bar of tab spans exactly one bar of music, so a
//! 16 column bar in 4/4 gives one semiquaver per column.  A note lasts until
//! the next event in its voice (or the end of its bar); notes that start off
//! the beat and cross a beat boundary are written as tied notes, which is
//! what turns the off-beat crotchets of the README example into pairs of
//! tied quavers.

use crate::music::{
    Chord, Dur, Rational, Spelling, SpellingMap, TimeSig, chord_spelling_map, key_spelling_map,
    lily_note_name, note_durs, pitch_class_name, spell,
};
use crate::parser::{Bar, Barline, StringEvent, System, Tab};

const LILYPOND_VERSION: &str = "2.24.0";
const STRING_COUNT: usize = 6;

/// Which staff voice a stream of music belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Voice {
    Single,
    Upper,
    Lower,
}

/// Generate LilyPond source for a parsed tab, plus any warnings raised while
/// generating.
pub fn generate(tab: &Tab) -> (String, Vec<String>) {
    let mut warnings = Vec::new();
    let mut out = String::new();
    out.push_str(&format!("\\version \"{LILYPOND_VERSION}\"\n\n"));

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
    if !header.is_empty() {
        out.push_str("\\header {\n");
        for (field, value) in header {
            out.push_str(&format!("  {field} = {}\n", quote(value)));
        }
        out.push_str("}\n\n");
    }

    let has_chords = tab.systems.iter().any(|system| !system.chords.is_empty());
    let key_map = tab.key.map(|key| key_spelling_map(key.fifths()));
    let volta = volta_marks(tab, &mut warnings);

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
    for tab_mode in [false, true] {
        write_staff(
            &mut out,
            tab,
            key_map.as_ref(),
            &mut warnings,
            tab_mode,
            &volta,
        );
    }
    out.push_str("    >>\n");
    out.push_str("  >>\n");
    out.push_str("  \\layout { }\n");
    out.push_str("}\n");
    (out, warnings)
}

/// Volta section boundaries, one `(starts, ends)` pair per bar of the
/// whole tab: a `:|` closes the section that began at the last `|:` or,
/// implicitly, just after the previous `:|` (or at the beginning).
fn volta_marks(tab: &Tab, warnings: &mut Vec<String>) -> Vec<(bool, bool)> {
    let mut marks: Vec<(bool, bool)> = Vec::new();
    let mut section_start: Option<usize> = None;
    let mut section_from = 0usize;
    for system in &tab.systems {
        for bar in &system.bars {
            let idx = marks.len();
            let (mut start, mut end) = (false, false);
            if bar.repeat_start {
                if section_start.is_some() {
                    warnings.push("ignoring nested repeat start (|:)".to_string());
                } else {
                    section_start = Some(idx);
                }
            }
            if bar.close == Barline::RepeatEnd {
                let from = section_start.take().unwrap_or(section_from);
                if from == idx {
                    start = true;
                } else {
                    marks[from].0 = true;
                }
                end = true;
                section_from = idx + 1;
            }
            marks.push((start, end));
        }
    }
    if section_start.is_some() {
        warnings.push("ignoring repeat start (|:) without an end".to_string());
    }
    marks
}

/// Write one staff of the score: the conventional `Staff` when `tab_mode` is
/// false, the matching `TabStaff` when true. The tab staff repeats the same
/// music with a string number attached to every note, so LilyPond computes
/// each fret from the string the tab actually uses instead of guessing a
/// first-position fingering (`minimumFret` only steers that guesswork, and
/// the automatic selector may put two chord notes on one string).
fn write_staff(
    out: &mut String,
    tab: &Tab,
    key_map: Option<&SpellingMap>,
    warnings: &mut Vec<String>,
    tab_mode: bool,
    volta: &[(bool, bool)],
) {
    let two_voice = tab.systems.iter().any(|system| {
        system
            .bars
            .iter()
            .any(|bar| bar.voice_split.is_some() || !bar.split_changes.is_empty())
    });
    let (staff, voice_ctx) = if tab_mode {
        ("TabStaff", "TabVoice")
    } else {
        ("Staff", "Voice")
    };
    // The tab staff renders the same events, so any warnings it raises have
    // already been reported by the conventional staff.
    let mut scratch = Vec::new();
    let warn: &mut Vec<String> = if tab_mode { &mut scratch } else { warnings };
    if two_voice {
        out.push_str(&format!("      \\new {staff} <<\n"));
        if !tab_mode {
            out.push_str("        \\clef \"treble_8\"\n");
        }
        for (name, voice, command) in [
            ("upper", Voice::Upper, "\\voiceOne"),
            ("lower", Voice::Lower, "\\voiceTwo"),
        ] {
            out.push_str(&format!("        \\new {voice_ctx} = \"{name}\" {{\n"));
            if tab_mode {
                // Tab staves suppress arpeggios by default.
                out.push_str("          \\override Arpeggio.stencil = #ly:arpeggio::print\n");
            }
            out.push_str(&format!("          {command}\n"));
            let mut state = VoiceState::new();
            let piece = Piece {
                tab,
                voice,
                key_map,
                tab_mode,
                volta,
            };
            let lines = render_voice_piece(&piece, &mut state, warn);
            for line in lines {
                out.push_str(&format!("          {line}\n"));
            }
            out.push_str("        }\n");
        }
        out.push_str("      >>\n");
    } else {
        out.push_str(&format!("      \\new {staff} {{\n"));
        if !tab_mode {
            out.push_str("        \\clef \"treble_8\"\n");
        } else {
            out.push_str("        \\override Arpeggio.stencil = #ly:arpeggio::print\n");
        }
        let mut state = VoiceState::new();
        let piece = Piece {
            tab,
            voice: Voice::Single,
            key_map,
            tab_mode,
            volta,
        };
        let lines = render_voice_piece(&piece, &mut state, warn);
        for line in lines {
            out.push_str(&format!("          {line}\n"));
        }
        out.push_str("      }\n");
    }
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
struct Event {
    /// 0-based column within the system.
    abs: usize,
    bar: usize,
    /// `(string, fret, arpeggiated)` per struck string.
    onsets: Vec<(usize, u8, bool)>,
    ties: Vec<usize>,
    duration: usize,
}

/// The immutable context for rendering one voice of one piece.
struct Piece<'a> {
    tab: &'a Tab,
    voice: Voice,
    key_map: Option<&'a SpellingMap>,
    tab_mode: bool,
    /// One `(starts, ends)` pair per bar of the whole tab.
    volta: &'a [(bool, bool)],
}

fn render_voice_piece(
    piece: &Piece,
    state: &mut VoiceState,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut lines = Vec::new();
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
    let tab_mode = piece.tab_mode;
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
    let mut pos = 0usize;
    let mut bar = 0usize;
    if piece.volta[bar_offset].0 {
        tokens.push("\\repeat volta 2 {".to_string());
    }

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
                tokens.extend(filler(
                    prefix,
                    &note_durs(
                        col_of(pos, &system.bars[bar]),
                        bar_end - pos,
                        timings[bar].1,
                        timings[bar].0,
                    ),
                ));
            }
            pos = bar_end;
            if tie_pending {
                push_tie(&mut tokens);
                tie_pending = false;
            }
            let mut suffix = bar_suffix(system, bar, n_bars, voice, system_index == last_system);
            if piece.volta[bar_offset + bar].1 {
                // The closing brace of a volta repeat draws the `:|`
                // barline itself.
                suffix = if suffix.is_empty() {
                    "}".to_string()
                } else {
                    format!("}} {suffix}")
                };
            }
            flush_bar(&mut tokens, &mut lines, &suffix);
            bar += 1;
            if bar < n_bars && piece.volta[bar_offset + bar].0 {
                tokens.push("\\repeat volta 2 {".to_string());
            }
        }
        if event.abs > pos {
            tokens.extend(filler(
                'r',
                &note_durs(
                    col_of(pos, &system.bars[bar]),
                    event.abs - pos,
                    timings[bar].1,
                    timings[bar].0,
                ),
            ));
        }
        if tie_pending {
            push_tie(&mut tokens);
        }

        // Tied notes keep their spelling; onsets are spelled in the context
        // of the active chord, falling back to the key. An arpeggio mark is
        // engraved in the tablature staff only.
        let mut notes: Vec<(usize, i32, String)> = tied;
        let arpeggio = tab_mode && event.onsets.iter().any(|&(_, _, arp)| arp);
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
            tokens.extend(filler(
                'r',
                &note_durs(
                    col_of(event.abs, &system.bars[event.bar]),
                    event.duration,
                    timings[event.bar].1,
                    timings[event.bar].0,
                ),
            ));
        } else {
            let (pitch_text, string_suffix) = chord_text(&notes, tab_mode);
            let durs = note_durs(
                col_of(event.abs, &system.bars[event.bar]),
                event.duration,
                timings[event.bar].1,
                timings[event.bar].0,
            );
            for (i, dur) in durs.iter().enumerate() {
                if i > 0 {
                    push_tie(&mut tokens);
                }
                let mut token = format!("{pitch_text}{}{string_suffix}", dur.to_lily());
                // The arpeggio marks the attack, so only the first of a
                // chain of tied notes carries it.
                if arpeggio && i == 0 {
                    token.push_str("\\arpeggio");
                }
                tokens.push(token);
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
            tokens.extend(filler(
                prefix,
                &note_durs(
                    col_of(pos, &system.bars[bar]),
                    bar_end - pos,
                    timings[bar].1,
                    timings[bar].0,
                ),
            ));
        }
        pos = bar_end;
        let mut suffix = bar_suffix(system, bar, n_bars, voice, system_index == last_system);
        if piece.volta[bar_offset + bar].1 {
            suffix = if suffix.is_empty() {
                "}".to_string()
            } else {
                format!("}} {suffix}")
            };
        }
        flush_bar(&mut tokens, &mut lines, &suffix);
        bar += 1;
        if bar < n_bars && piece.volta[bar_offset + bar].0 {
            tokens.push("\\repeat volta 2 {".to_string());
        }
    }
    lines
}

/// Build a voice's event timeline for one system, merging simultaneous
/// events and computing durations.
fn build_events(system: &System, voice: Voice, warnings: &mut Vec<String>) -> Vec<Event> {
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

/// Rest or skip tokens (`prefix` is `r` or `s`) covering a stretch of time.
fn filler(prefix: char, durs: &[Dur]) -> Vec<String> {
    durs.iter()
        .map(|dur| format!("{prefix}{}", dur.to_lily()))
        .collect()
}

/// Pitch text for one note event, plus a suffix to place after the duration.
/// LilyPond wants `c4\5` for single notes (duration before the string number)
/// but `<c\5 e\4 g\3>1` for chords (string numbers inside, duration outside).
fn chord_text(notes: &[(usize, i32, String)], with_strings: bool) -> (String, String) {
    if notes.len() == 1 {
        let &(string, _, ref name) = &notes[0];
        // String 1 is the highest string, which is index 0 in the tuning.
        let suffix = if with_strings {
            format!("\\{}", string + 1)
        } else {
            String::new()
        };
        (name.clone(), suffix)
    } else {
        let names: Vec<String> = notes
            .iter()
            .map(|&(string, _, ref name)| {
                if with_strings {
                    format!("{name}\\{}", string + 1)
                } else {
                    name.clone()
                }
            })
            .collect();
        (format!("<{}>", names.join(" ")), String::new())
    }
}

/// Attach a tie to the most recent token.
fn push_tie(tokens: &mut [String]) {
    if let Some(last) = tokens.last_mut() {
        last.push('~');
    }
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
fn chord_lines(tab: &Tab, volta: &[(bool, bool)]) -> Vec<String> {
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
            if volta[bar_offset].0 {
                tokens.push("\\repeat volta 2 {".to_string());
            }
            let mut pos = start;
            for (i, &(abs, chord)) in slots.iter().enumerate() {
                if abs < start {
                    continue;
                }
                if abs >= end {
                    break;
                }
                if abs > pos {
                    tokens.extend(filler(
                        's',
                        &note_durs(col_of(pos, bar_ref), abs - pos, beat_cols, unit),
                    ));
                }
                let next = slots
                    .get(i + 1)
                    .map_or(end, |&(next, _)| next)
                    .min(end)
                    .max(abs + 1);
                let durs = note_durs(col_of(abs, bar_ref), next - abs, beat_cols, unit);
                for (di, dur) in durs.iter().enumerate() {
                    if di > 0 {
                        push_tie(&mut tokens);
                    }
                    tokens.push(chord.lily_with_duration(&dur.to_lily()));
                }
                pos = next;
            }
            if pos < end {
                tokens.extend(filler(
                    's',
                    &note_durs(col_of(pos, bar_ref), end - pos, beat_cols, unit),
                ));
            }
            let mut suffix = if bar + 1 == system.bars.len() && si != last_system {
                "\\break"
            } else {
                ""
            };
            if volta[bar_offset].1 {
                suffix = if suffix.is_empty() { "}" } else { "} \\break" };
            }
            flush_bar(&mut tokens, &mut lines, suffix);
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
        assert!(lilypond.contains("\\new Staff <<"));
        assert!(lilypond.contains("\\clef \"treble_8\""));
        assert!(lilypond.contains("\\voiceOne"));
        assert!(lilypond.contains("\\voiceTwo"));
        assert!(lilypond.contains("\\time 4/4"));
        // Treble: a quaver, three pairs of tied quavers and a final quaver.
        assert!(lilypond.contains("fis8 a8~ a8 dis'8~ dis'8 fis'8~ fis'8 a'8 |"));
        // Bass: two quavers, a crotchet and a minim.
        assert!(lilypond.contains("fis,8 a,8 b,4 cis2 |"));
        assert!(lilypond.contains("\\new StaffGroup <<"));
        assert!(lilypond.contains("\\new TabStaff <<"));
        assert!(lilypond.contains("\\new TabVoice = \"upper\""));
        assert!(lilypond.contains("\\new TabVoice = \"lower\""));
        // The tab staff repeats the music with a string number on every note,
        // so LilyPond derives the frets the tab specifies instead of guessing
        // a first-position fingering.
        assert!(
            lilypond
                .contains("fis8\\4 a8\\3~ a8\\3 dis'8\\2~ dis'8\\2 fis'8\\1~ fis'8\\1 a'8\\1 |")
        );
        assert!(lilypond.contains("fis,8\\6 a,8\\6 b,4\\5 cis2\\5 |"));
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
        // Without a voice split both staves take their music directly.
        assert!(lilypond.contains("\\new Staff {"));
        assert!(lilypond.contains("\\new TabStaff {"));
        assert!(!lilypond.contains("\\new Voice"));
        assert!(!lilypond.contains("\\new TabVoice"));
        assert!(lilypond.contains("<g, e'>2 a'2 |"));
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
        // First bar: C major shape in the treble, Travis-picking bass.
        assert!(lilypond.contains("<g c' e'>4 <g c' e'>4 <gis d' e'>2 |"));
        assert!(lilypond.contains("c4 e4 e,4 e4 |"));
        // The tie over the barline at the end of the first system.
        assert!(lilypond.contains("c'4 b4 a8 b8~ b8 c'8~ |"));
        // The first system is a repeated section: it opens with a volta at
        // the first bar and the `:|` closes it, so the barline itself is
        // drawn by the repeat and every context wraps the same bars.
        assert!(lilypond.contains("\\repeat volta 2 { c2 a2:7 |"));
        assert!(lilypond.contains("c'1 | } \\break"));
        assert!(lilypond.contains("\\repeat volta 2 { g4 c'4 b8 c'8~ c'4 |"));
        assert!(lilypond.contains("\\repeat volta 2 { c4 e4 c4 e4 |"));
        assert!(lilypond.contains("\\repeat volta 2 { g4\\3 c'4\\2 b8\\2 c'8\\2~ c'4\\2 |"));
        assert!(!lilypond.contains("\\bar \":|\""));
        // Bar 9 changes to A7 half way through, so the first-string F is
        // spelled F natural (the key's spelling) rather than E sharp; the
        // move to the chord's E natural then avoids an E sharp to E motion.
        assert!(
            lilypond.contains("<g c' e'>4 <g c' e'>4 <g cis' f'>8~ <g cis' e'>8~ <g cis' e'>4 |")
        );
        // The final bar is marked single voice (`!` on the bottom string):
        // the whole C major chord sounds in the treble while the bass is
        // filled with an invisible rest.
        assert!(lilypond.contains("<c e g c'>1 | \\bar \"||\""));
        assert!(lilypond.contains("s1 |"));
        // The third bar of the last system has no marker, so it keeps the
        // split of the bar before it.
        assert!(lilypond.contains("c'4 b4 a8 b8~ b4 |"));
        assert!(lilypond.contains("g,4 d4 g,4 d4 |"));
        assert!(lilypond.contains("\\new TabStaff <<"));
        // The tab staff pins every note to the string the tab uses, so the
        // frets LilyPond derives match the tab exactly.
        assert!(lilypond.contains("g4\\3 c'4\\2 b8\\2 c'8\\2~ c'4\\2 |"));
        assert!(
            lilypond.contains("<g\\3 c'\\2 e'\\1>4 <g\\3 c'\\2 e'\\1>4 <gis\\3 d'\\2 e'\\1>2 |")
        );
        assert!(lilypond.contains("c4\\5 e4\\4 e,4\\6 e4\\4 |"));
        assert!(
            lilypond
                .contains("<g\\3 cis'\\2 f'\\1>8~ <g\\3 cis'\\2 e'\\1>8~ <g\\3 cis'\\2 e'\\1>4 |")
        );
        assert!(lilypond.contains("<c\\5 e\\4 g\\3 c'\\2>1\\arpeggio | \\bar \"||\""));
        assert!(lilypond.contains("c'4\\2 b4\\2 a8\\3 b8\\2~ b4\\2 |"));
        assert!(lilypond.contains("g,4\\6 d4\\4 g,4\\6 d4\\4 |"));
        // Tab staves re-enable arpeggio printing.
        assert!(lilypond.contains("\\override Arpeggio.stencil = #ly:arpeggio::print"));
        // The `.` in the eighth column of the second system's third bar moves
        // the split to the third string: the G string joins the treble voice
        // from the next note on.
        assert!(lilypond.contains("<c' e'>4 d'4 <g c'>8 d'8~ d'8 e'8~ |"));
        assert!(lilypond.contains("f4 a4 g,4 d4 |"));
        assert!(lilypond.contains("<c'\\2 e'\\1>4 d'4\\2 <g\\3 c'\\2>8 d'8\\2~ d'8\\2 e'8\\1~ |"));
        assert!(lilypond.contains("f4\\4 a4\\3 g,4\\6 d4\\4 |"));
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
        assert_eq!(lilypond.matches("\\repeat volta 2 { e'2 e'2 |").count(), 1);
        assert_eq!(lilypond.matches("e'2 e'2 | }").count(), 1);
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
        let bar = "f4 a4 c'4 f'4 |";
        assert_eq!(lilypond.matches(bar).count(), 4);
        let tab_bar = "f4\\5 a4\\4 c'4\\3 f'4\\2 |";
        assert_eq!(lilypond.matches(tab_bar).count(), 4);
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
        // The chord starts on the first column (a whole note), and the
        // arpeggio is engraved in the tablature staff only.
        assert!(lilypond.contains("<a c' e'>1 |"));
        assert!(lilypond.contains("<a\\3 c'\\2 e'\\1>1\\arpeggio |"));
        assert!(lilypond.contains("\\override Arpeggio.stencil = #ly:arpeggio::print"));
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
        assert!(lilypond.contains("a8 b8~ b8 a8 fis8 a8 b8 cis'8 |"));
        // Bass: alternating crotchets, the low E open and third fret.
        assert!(lilypond.contains("e,4 e4 g,4 b,4 |"));
        assert!(lilypond.contains("a8\\3 b8\\3~ b8\\3 a8\\3 fis8\\4 a8\\3 b8\\3 cis'8\\2 |"));
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
        assert!(lilypond.contains("e'8 c'8 g8 d'8 f'8 c'8 g8 d'8 |"));
        assert!(lilypond.contains("c4 e4 c4 e4 |"));
        // Second bar keeps the split: the bass still has its own notes
        // instead of everything merging into one voice.
        assert!(lilypond.contains("g'8 b8 d'8 f'8 g'8 d'8 b8 g8 |"));
        assert!(lilypond.contains("g,4 d4 g,4 d4 |"));
        assert!(lilypond.contains("e'8\\1 c'8\\2 g8\\3 d'8\\2 f'8\\1 c'8\\2 g8\\3 d'8\\2 |"));
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
        assert!(lilypond.contains("c'4 b4 a8 b8~ b4 |"));
        assert!(lilypond.contains("g,4 d4 g,4 d4 |"));
        assert!(lilypond.contains("<c e g c'>1 |"));
        assert!(lilypond.contains("s1 |"));
        assert!(
            !lilypond.contains("r1"),
            "rests must be invisible: {lilypond}"
        );
        assert!(lilypond.contains("c'4\\2 b4\\2 a8\\3 b8\\2~ b4\\2 |"));
        assert!(lilypond.contains("<c\\5 e\\4 g\\3 c'\\2>1 |"));
    }
}
