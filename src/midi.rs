//! MIDI file generation.
//!
//! Playback follows the same voice and event model as the LilyPond backend:
//! a note sounds from its onset until the next event in its voice (or the
//! end of its bar), tie marks merge notes into one sustained sound, and
//! repeat sections are expanded so every pass is played with its own
//! alternative ending.  The file is a format 1 standard MIDI file at 480
//! ticks per quarter note: track 0 is the tempo map (track name, key and
//! time signatures, tempo) and each voice gets its own track on the General
//! MIDI acoustic guitar (steel) program, so voices can be mixed or muted
//! independently.

use crate::lily::{Event, STRING_COUNT, Voice, VoltaBar, build_events, volta_marks};
use crate::music::{Rational, TimeSig};
use crate::parser::Tab;

/// Ticks per quarter note.
const TPQ: i64 = 480;
/// General MIDI program 26 (zero based 25): Acoustic Guitar (steel).
const PROGRAM: u8 = 25;
const VELOCITY: u8 = 100;

/// One scheduled MIDI event.  `order` sequences simultaneous events: meta
/// messages and program changes first, then note-offs (so a re-struck pitch
/// is released before it restarts), then note-ons.
struct MidiEvent {
    tick: u32,
    order: u8,
    bytes: Vec<u8>,
}

impl MidiEvent {
    fn meta(tick: u32, kind: u8, data: Vec<u8>) -> Self {
        let mut bytes = vec![0xFF, kind];
        bytes.extend(vlq(data.len() as u32));
        bytes.extend(data);
        MidiEvent {
            tick,
            order: 0,
            bytes,
        }
    }
}

/// Converts quarter-note times to MIDI ticks, warning once if any time is
/// not an exact whole number of ticks.
struct Ticker {
    rounded: bool,
}

impl Ticker {
    fn tick(&mut self, quarters: Rational, warnings: &mut Vec<String>) -> u32 {
        if !self.rounded {
            let scaled = quarters * Rational::integer(TPQ);
            if !scaled.is_integer() {
                self.rounded = true;
                warnings.push(format!(
                    "timing is not a whole number of MIDI ticks at {TPQ} ticks per quarter; rounding to the nearest tick"
                ));
            }
        }
        quarters.round_scaled(TPQ) as u32
    }
}

/// The note currently sounding on one string: its index in the note list,
/// the id of the event that last touched it, and where that happened.
#[derive(Clone, Copy)]
struct OpenNote {
    idx: usize,
    event_id: usize,
    /// The system and bar-within-system of the last touching event.
    bar: (usize, usize),
    /// The within-system column of the last touching event.
    abs: usize,
}

/// Generate a standard MIDI file for a parsed tab, plus any warnings raised
/// while generating.
pub fn generate(tab: &Tab) -> (Vec<u8>, Vec<String>) {
    let mut warnings = Vec::new();
    let marks = volta_marks(tab, &mut warnings);
    let order = playback_order(&marks, total_bars(tab));

    // The bars in playback order, with the time each starts, in quarter
    // notes.
    let mut timeline: Vec<(usize, usize, Rational)> = Vec::with_capacity(order.len());
    let mut t = Rational::integer(0);
    for &bar_idx in &order {
        let (si, bi) = locate(tab, bar_idx);
        timeline.push((si, bi, t));
        t = t + tab.systems[si].time_sig.bar_len();
    }

    let mut ticker = Ticker { rounded: false };
    let mut chunks = vec![build_track(meta_track(
        tab,
        &timeline,
        &mut ticker,
        &mut warnings,
    ))];

    let two_voice = tab.systems.iter().any(|system| {
        system
            .bars
            .iter()
            .any(|bar| bar.voice_split.is_some() || !bar.split_changes.is_empty())
    });
    let voices: &[(Voice, u8, &str)] = if two_voice {
        &[(Voice::Upper, 0, "upper"), (Voice::Lower, 1, "lower")]
    } else {
        &[(Voice::Single, 0, "guitar")]
    };
    for &(voice, channel, name) in voices {
        let events = voice_track(
            tab,
            voice,
            channel,
            name,
            &timeline,
            &mut ticker,
            &mut warnings,
        );
        chunks.push(build_track(events));
    }

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // format 1
    out.extend_from_slice(&(chunks.len() as u16).to_be_bytes());
    out.extend_from_slice(&(TPQ as u16).to_be_bytes());
    for chunk in chunks {
        out.extend(chunk);
    }
    (out, warnings)
}

fn total_bars(tab: &Tab) -> usize {
    tab.systems.iter().map(|system| system.bars.len()).sum()
}

/// The system index and bar-within-system index of a whole-tab bar number.
fn locate(tab: &Tab, bar_idx: usize) -> (usize, usize) {
    let mut rest = bar_idx;
    for (si, system) in tab.systems.iter().enumerate() {
        if rest < system.bars.len() {
            return (si, rest);
        }
        rest -= system.bars.len();
    }
    unreachable!("bar index outside the tab");
}

/// The bars of the tab in playback order: repeat sections (`|:` … `:|`) are
/// expanded, and each pass takes its own alternative ending when the
/// section has volta endings.
fn playback_order(marks: &[VoltaBar], total: usize) -> Vec<usize> {
    let mut order = Vec::with_capacity(total);
    let mut i = 0usize;
    while i < total {
        if let Some(count) = marks[i].start {
            let mut body_end = i;
            while body_end < total && !marks[body_end].body_end {
                body_end += 1;
            }
            let body_end = body_end.min(total - 1);
            // The alternative endings, one run per pass, if the section has
            // any.
            let mut alts: Vec<Vec<usize>> = Vec::new();
            let mut after = body_end + 1;
            if after < total && marks[after].alt_list_start {
                let mut current: Vec<usize> = Vec::new();
                let mut k = after;
                while k < total {
                    current.push(k);
                    if marks[k].alt_end {
                        alts.push(std::mem::take(&mut current));
                    }
                    if marks[k].alt_list_end {
                        break;
                    }
                    k += 1;
                }
                after = k + 1;
            }
            for pass in 0..count {
                order.extend(i..=body_end);
                if let Some(alt) = alts.get(pass) {
                    order.extend_from_slice(alt);
                }
            }
            i = after;
        } else {
            order.push(i);
            i += 1;
        }
    }
    order
}

/// Track 0: the tempo map — track name, key signature, tempo and the time
/// signature changes along the playback timeline.
fn meta_track(
    tab: &Tab,
    timeline: &[(usize, usize, Rational)],
    ticker: &mut Ticker,
    warnings: &mut Vec<String>,
) -> Vec<MidiEvent> {
    let mut events = Vec::new();
    if let Some(title) = &tab.title {
        events.push(MidiEvent::meta(0, 0x03, title.clone().into_bytes()));
    }
    if let Some(key) = tab.key {
        // The first byte is a signed fifths count, the second the mode.
        events.push(MidiEvent::meta(
            0,
            0x59,
            vec![key.fifths() as u8, key.minor as u8],
        ));
    }
    if let Some(tempo) = tab.tempo {
        // Microseconds per quarter note, rounded, as a 24 bit big-endian
        // number.
        let usec = ((60_000_000 + tempo as i64 / 2) / tempo as i64).min(0xFF_FFFF);
        events.push(MidiEvent::meta(0, 0x51, usec.to_be_bytes()[5..8].to_vec()));
    }
    let mut last_sig: Option<TimeSig> = None;
    for &(si, _, start) in timeline {
        let sig = tab.systems[si].time_sig;
        if last_sig != Some(sig) {
            last_sig = Some(sig);
            if let Some(data) = time_sig_data(sig, warnings) {
                let tick = ticker.tick(start, warnings);
                events.push(MidiEvent::meta(tick, 0x58, data));
            }
        }
    }
    events
}

/// The `FF 58` time signature data: numerator, denominator as a power of
/// two, and the standard metronome click settings.
fn time_sig_data(sig: TimeSig, warnings: &mut Vec<String>) -> Option<Vec<u8>> {
    let num = match u8::try_from(sig.num) {
        Ok(num) => num,
        Err(_) => {
            warnings.push(format!(
                "time signature numerator {} is out of MIDI range; omitted from the MIDI file",
                sig.num
            ));
            return None;
        }
    };
    if !sig.den.is_power_of_two() {
        warnings.push(format!(
            "time signature denominator {} is not a power of two; omitted from the MIDI file",
            sig.den
        ));
        return None;
    }
    Some(vec![num, sig.den.trailing_zeros() as u8, 24, 8])
}

/// One voice's track: track name and program change, then the notes of
/// every playback bar, with tied notes merged into single sustained notes.
fn voice_track(
    tab: &Tab,
    voice: Voice,
    channel: u8,
    name: &str,
    timeline: &[(usize, usize, Rational)],
    ticker: &mut Ticker,
    warnings: &mut Vec<String>,
) -> Vec<MidiEvent> {
    let mut events = vec![
        MidiEvent::meta(0, 0x03, name.as_bytes().to_vec()),
        MidiEvent {
            tick: 0,
            order: 0,
            bytes: vec![0xC0 | channel, PROGRAM],
        },
    ];
    // Events per system, computed once so repeat passes reuse them without
    // repeating any warnings.
    let per_system: Vec<Vec<Event>> = tab
        .systems
        .iter()
        .map(|system| build_events(system, voice, warnings))
        .collect();
    let mut notes: Vec<(u32, u32, u8)> = Vec::new();
    let mut strings: [Option<OpenNote>; STRING_COUNT] = [None; STRING_COUNT];
    let mut last_event_id = 0usize;
    for &(si, bi, start) in timeline {
        let system = &tab.systems[si];
        let bar = &system.bars[bi];
        let col = system.time_sig.bar_len() / Rational::integer(bar.width as i64);
        for event in per_system[si].iter().filter(|event| event.bar == bi) {
            let event_id = last_event_id + 1;
            let on_q = start + col * Rational::integer((event.abs - bar.start) as i64);
            let off_q = on_q + col * Rational::integer(event.duration as i64);
            let on = ticker.tick(on_q, warnings);
            let off = ticker.tick(off_q, warnings);
            for &string in &event.ties {
                match strings[string] {
                    // A tie continues the note the immediately preceding
                    // event left sounding on the string, when that event is
                    // where the tie's source lives: the same bar at an
                    // earlier column, or the previous bar of the system.  At
                    // a repeat boundary the previous pass's note is not the
                    // source the tie mark points at, so it is ignored.
                    Some(open)
                        if open.event_id == last_event_id
                            && ((open.bar == (si, bi) && open.abs < event.abs)
                                || (bi > 0 && open.bar == (si, bi - 1))) =>
                    {
                        notes[open.idx].1 = off;
                        strings[string] = Some(OpenNote {
                            idx: open.idx,
                            event_id,
                            bar: (si, bi),
                            abs: event.abs,
                        });
                    }
                    _ => warnings.push(format!(
                        "ignoring tie mark on string {}: no note to tie from at this point",
                        string + 1
                    )),
                }
            }
            for &(string, fret, _) in &event.onsets {
                if let Some(open) = strings[string] {
                    notes[open.idx].1 = notes[open.idx].1.min(on);
                }
                let pitch = tab.tuning.open_midi[string] + fret as i32;
                if !(0..=127).contains(&pitch) {
                    warnings.push(format!(
                        "ignoring note on string {}: pitch {pitch} is outside the MIDI range",
                        string + 1
                    ));
                    strings[string] = None;
                    continue;
                }
                notes.push((on, off, pitch as u8));
                strings[string] = Some(OpenNote {
                    idx: notes.len() - 1,
                    event_id,
                    bar: (si, bi),
                    abs: event.abs,
                });
            }
            last_event_id = event_id;
        }
    }
    for &(on, off, pitch) in &notes {
        events.push(MidiEvent {
            tick: on,
            order: 2,
            bytes: vec![0x90 | channel, pitch, VELOCITY],
        });
        events.push(MidiEvent {
            tick: off,
            order: 1,
            bytes: vec![0x80 | channel, pitch, 0],
        });
    }
    events
}

/// Encode one track chunk: the events sorted by time, as delta times.
fn build_track(mut events: Vec<MidiEvent>) -> Vec<u8> {
    events.sort_by_key(|event| (event.tick, event.order));
    let mut data = Vec::new();
    let mut last = 0u32;
    for event in events {
        data.extend(vlq(event.tick - last));
        data.extend(event.bytes);
        last = event.tick;
    }
    data.extend(vlq(0));
    data.extend([0xFF, 0x2F, 0x00]); // end of track
    let mut chunk = b"MTrk".to_vec();
    chunk.extend((data.len() as u32).to_be_bytes());
    chunk.extend(data);
    chunk
}

/// A MIDI variable-length quantity: 7 bits per byte, most significant
/// first, with the high bit marking continuation.
fn vlq(mut value: u32) -> Vec<u8> {
    let mut bytes = vec![value as u8 & 0x7F];
    value >>= 7;
    while value > 0 {
        bytes.push((value as u8 & 0x7F) | 0x80);
        value >>= 7;
    }
    bytes.reverse();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn vlq_encoding() {
        assert_eq!(vlq(0), vec![0x00]);
        assert_eq!(vlq(0x40), vec![0x40]);
        assert_eq!(vlq(0x7F), vec![0x7F]);
        assert_eq!(vlq(0x80), vec![0x81, 0x00]);
        assert_eq!(vlq(0x2000), vec![0xC0, 0x00]);
        assert_eq!(vlq(0x1FFFFF), vec![0xFF, 0xFF, 0x7F]);
        assert_eq!(vlq(0x200000), vec![0x81, 0x80, 0x80, 0x00]);
    }

    /// A minimal standard MIDI file reader, enough to check what the
    /// backend wrote: one vector of `(tick, bytes)` per track.
    fn tracks(bytes: &[u8]) -> Vec<Vec<(u32, Vec<u8>)>> {
        assert_eq!(&bytes[0..4], b"MThd");
        let header_len = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(u16::from_be_bytes(bytes[8..10].try_into().unwrap()), 1);
        let ntrks = u16::from_be_bytes(bytes[10..12].try_into().unwrap()) as usize;
        assert_eq!(u16::from_be_bytes(bytes[12..14].try_into().unwrap()), 480);
        let mut pos = 8 + header_len;
        let mut out = Vec::new();
        for _ in 0..ntrks {
            assert_eq!(&bytes[pos..pos + 4], b"MTrk");
            let len = u32::from_be_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            let end = pos + 8 + len;
            let mut p = pos + 8;
            let mut tick = 0u32;
            let mut events = Vec::new();
            while p < end {
                let (delta, n) = read_vlq(&bytes[p..]);
                tick += delta;
                p += n;
                if bytes[p] == 0xFF {
                    let kind = bytes[p + 1];
                    let (len, n) = read_vlq(&bytes[p + 2..]);
                    p += 2 + n;
                    let mut data = vec![0xFF, kind];
                    data.extend(&bytes[p..p + len as usize]);
                    p += len as usize;
                    events.push((tick, data));
                } else {
                    let bytes_per_data = match bytes[p] & 0xF0 {
                        0xC0 | 0xD0 => 1,
                        _ => 2,
                    };
                    events.push((tick, bytes[p..p + 1 + bytes_per_data].to_vec()));
                    p += 1 + bytes_per_data;
                }
            }
            assert_eq!(p, end, "track data does not end where its length says");
            out.push(events);
            pos = end;
        }
        assert_eq!(pos, bytes.len(), "trailing bytes after the last track");
        out
    }

    fn read_vlq(bytes: &[u8]) -> (u32, usize) {
        let mut value = 0u32;
        let mut n = 0usize;
        loop {
            let byte = bytes[n];
            value = (value << 7) | (byte & 0x7F) as u32;
            n += 1;
            if byte & 0x80 == 0 {
                return (value, n);
            }
        }
    }

    /// The `(tick, channel, pitch)` of every note-on in a track.
    fn note_ons(track: &[(u32, Vec<u8>)]) -> Vec<(u32, u8, u8)> {
        track
            .iter()
            .filter(|(_, bytes)| bytes[0] & 0xF0 == 0x90 && bytes[2] > 0)
            .map(|(tick, bytes)| (*tick, bytes[0] & 0x0F, bytes[1]))
            .collect()
    }

    /// The `(tick, channel, pitch)` of every note-off in a track.
    fn note_offs(track: &[(u32, Vec<u8>)]) -> Vec<(u32, u8, u8)> {
        track
            .iter()
            .filter(|(_, bytes)| bytes[0] & 0xF0 == 0x80)
            .map(|(tick, bytes)| (*tick, bytes[0] & 0x0F, bytes[1]))
            .collect()
    }

    fn program_change(track: &[(u32, Vec<u8>)]) -> Option<(u8, u8)> {
        track
            .iter()
            .find(|(_, bytes)| bytes[0] & 0xF0 == 0xC0)
            .map(|(_, bytes)| (bytes[0] & 0x0F, bytes[1]))
    }

    fn metas(track: &[(u32, Vec<u8>)], kind: u8) -> Vec<(u32, Vec<u8>)> {
        track
            .iter()
            .filter(|(_, bytes)| bytes[0] == 0xFF && bytes[1] == kind)
            .map(|(tick, bytes)| (*tick, bytes[2..].to_vec()))
            .collect()
    }

    #[test]
    fn single_voice_notes() {
        let tab = parse(
            "\
{signature:4/4}
|----------------|
|----------------|
|----------------|
|----------------|
|----------------|
|0-------5-------|
",
        )
        .unwrap();
        let (bytes, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let tracks = tracks(&bytes);
        assert_eq!(tracks.len(), 2);
        // The guitar track plays the open low E for a quarter and the fifth
        // fret for a quarter: 480 ticks per quarter note.
        let guitar = &tracks[1];
        assert_eq!(program_change(guitar), Some((0, PROGRAM)));
        assert_eq!(note_ons(guitar), vec![(0, 0, 40), (960, 0, 45)]);
        assert_eq!(note_offs(guitar), vec![(960, 0, 40), (1920, 0, 45)]);
    }

    #[test]
    fn tie_marks_merge_notes() {
        // A `^` continues the note it follows: within the bar and across
        // the barline of the next bar in the same system.
        let tab = parse(
            "\
{signature:4/4}
|--------|--------|
|--------|--------|
|--------|--------|
|--------|--------|
|--------|--------|
|0---^---|^-------|
",
        )
        .unwrap();
        let (bytes, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let guitar = &tracks(&bytes)[1];
        // One sustained note over both bars instead of three separate ones.
        assert_eq!(note_ons(guitar), vec![(0, 0, 40)]);
        assert_eq!(note_offs(guitar), vec![(3840, 0, 40)]);
    }

    #[test]
    fn repeat_is_played_twice() {
        // The README's repeat example: two bars, each with two crotchets on
        // the open low E, played through twice.
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
        let (bytes, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let guitar = &tracks(&bytes)[1];
        let ticks: Vec<u32> = note_ons(guitar).iter().map(|&(t, _, _)| t).collect();
        assert_eq!(ticks, vec![0, 960, 1920, 2880, 3840, 4800, 5760, 6720]);
        assert!(note_ons(guitar).iter().all(|&(_, _, pitch)| pitch == 64));
    }

    #[test]
    fn alternative_endings_are_expanded() {
        // The README's endings example: two body bars, a first ending of
        // one bar and a second ending of two.  The body plays twice, once
        // with each ending.
        let tab = parse(
            "\
{signature:4/4}
                   +1--------+2-------+
|:0---0---|0---0---|0---0---:|0---0---|0-------|
|:--------|--------|--------:|--------|--------|
|:--------|--------|--------:|--------|--------|
|:--------|--------|--------:|--------|--------|
|:--------|--------|--------:|--------|--------|
|:--------|--------|--------:|--------|--------|
",
        )
        .unwrap();
        let (bytes, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let guitar = &tracks(&bytes)[1];
        let ticks: Vec<u32> = note_ons(guitar).iter().map(|&(t, _, _)| t).collect();
        // Four notes per body pass, two in the first ending, two at the
        // start of the second ending and one in its last bar.
        assert_eq!(
            ticks,
            vec![
                0, 960, 1920, 2880, // first pass, body
                3840, 4800, // first ending
                5760, 6720, 7680, 8640, // second pass, body
                9600, 10560, 11520, // second ending
            ]
        );
    }

    #[test]
    fn tempo_time_signature_and_key() {
        let tab = parse(
            "\
{tempo:90}
{signature:3/4}
{key:F}
|0-----|
|------|
|------|
|------|
|------|
|------|
",
        )
        .unwrap();
        let (bytes, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let meta = &tracks(&bytes)[0];
        // 60 000 000 µs / 90 bpm, rounded: 666 667.
        assert_eq!(metas(meta, 0x51), vec![(0, vec![0x0A, 0x2C, 0x2B])]);
        // 3/4 with the standard metronome settings.
        assert_eq!(metas(meta, 0x58), vec![(0, vec![3, 2, 24, 8])]);
        // One flat, major.
        assert_eq!(metas(meta, 0x59), vec![(0, vec![0xFF, 0x00])]);
    }

    #[test]
    fn two_voices_on_separate_channels() {
        // The README's voices example: the upper voice on channel 0 and the
        // lower voice on channel 1, both on the guitar program.
        let tab = parse(
            "\
{signature:4/4}
|----------2---5-|
|------4---------|
|--2-------------|
!4---------------|
|----2---4-------|
|2-5-------------|
",
        )
        .unwrap();
        let (bytes, warnings) = generate(&tab);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let tracks = tracks(&bytes);
        assert_eq!(tracks.len(), 3);
        let upper = &tracks[1];
        let lower = &tracks[2];
        assert_eq!(program_change(upper), Some((0, PROGRAM)));
        assert_eq!(program_change(lower), Some((1, PROGRAM)));
        // The treble voice: a quaver, three pairs of tied quavers that
        // merge into quarters, and a final quaver.
        assert_eq!(
            note_ons(upper),
            vec![
                (0, 0, 54),
                (240, 0, 57),
                (720, 0, 63),
                (1200, 0, 66),
                (1680, 0, 69)
            ]
        );
        assert_eq!(
            note_offs(upper),
            vec![
                (240, 0, 54),
                (720, 0, 57),
                (1200, 0, 63),
                (1680, 0, 66),
                (1920, 0, 69)
            ]
        );
        // The bass voice: two quavers, a crotchet and a minim.
        assert_eq!(
            note_ons(lower),
            vec![(0, 1, 42), (240, 1, 45), (480, 1, 47), (960, 1, 49)]
        );
        assert_eq!(
            note_offs(lower),
            vec![(240, 1, 42), (480, 1, 45), (960, 1, 47), (1920, 1, 49)]
        );
    }

    #[test]
    fn inexact_grid_is_rounded_with_a_warning() {
        // A seven column bar in 4/4 does not map onto whole ticks.
        let tab = parse(
            "\
{signature:4/4}
|2--2---|
|-------|
|-------|
|-------|
|-------|
|-------|
",
        )
        .unwrap();
        let (bytes, warnings) = generate(&tab);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("rounding"), "warnings: {warnings:?}");
        let guitar = &tracks(&bytes)[1];
        // 3 columns of 4/7 quarters is 822.86 ticks, rounded to 823; the
        // bar still ends exactly at 1920.
        assert_eq!(note_ons(guitar), vec![(0, 0, 66), (823, 0, 66)]);
        assert_eq!(note_offs(guitar), vec![(823, 0, 66), (1920, 0, 66)]);
    }
}
