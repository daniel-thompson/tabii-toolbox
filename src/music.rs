//! Musical building blocks shared by the parser and the LilyPond backend:
//! exact duration arithmetic, the timing model, pitch spelling and chord
//! symbols.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, Div, Mul, Sub};

/// Exact rational number.  All duration arithmetic uses these so note values
/// never suffer from floating point rounding.
#[derive(Clone, Copy, Debug)]
pub struct Rational {
    num: i64,
    den: i64,
}

impl Rational {
    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "rational with zero denominator");
        let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
        let g = gcd(num.abs(), den).max(1);
        Rational {
            num: num / g,
            den: den / g,
        }
    }

    pub fn integer(value: i64) -> Self {
        Rational::new(value, 1)
    }

    pub fn is_positive(self) -> bool {
        self.num > 0
    }
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl PartialEq for Rational {
    fn eq(&self, other: &Self) -> bool {
        self.num == other.num && self.den == other.den
    }
}

impl Eq for Rational {}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.num * other.den).cmp(&(other.num * self.den))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Add for Rational {
    type Output = Rational;

    fn add(self, rhs: Rational) -> Rational {
        Rational::new(self.num * rhs.den + rhs.num * self.den, self.den * rhs.den)
    }
}

impl Sub for Rational {
    type Output = Rational;

    fn sub(self, rhs: Rational) -> Rational {
        Rational::new(self.num * rhs.den - rhs.num * self.den, self.den * rhs.den)
    }
}

impl Mul for Rational {
    type Output = Rational;

    fn mul(self, rhs: Rational) -> Rational {
        Rational::new(self.num * rhs.num, self.den * rhs.den)
    }
}

impl Div for Rational {
    type Output = Rational;

    fn div(self, rhs: Rational) -> Rational {
        assert!(rhs.num != 0, "division by zero rational");
        Rational::new(self.num * rhs.den, self.den * rhs.num)
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Time signature: `num` occurrences of a `den` note per bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSig {
    pub num: u32,
    pub den: u32,
}

impl TimeSig {
    /// Length of one bar in quarter notes.
    pub fn bar_len(self) -> Rational {
        Rational::new(4 * self.num as i64, self.den as i64)
    }
}

/// Key: root pitch plus mode; enough to derive the key signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub letter: usize,
    pub acc: i32,
    pub minor: bool,
}

impl Key {
    /// Position on the circle of fifths (positive = sharps).
    pub fn fifths(self) -> i32 {
        let pc = (NATURAL_PC[self.letter] + self.acc).rem_euclid(12);
        let (letter, acc) = if self.minor {
            // The relative major lies three semitones above the tonic.
            let rel_pc = (pc + 3) % 12;
            let letter = (self.letter + 2) % 7;
            let diff = (rel_pc - NATURAL_PC[letter]).rem_euclid(12);
            let acc = if diff <= 6 { diff } else { diff - 12 };
            (letter, acc)
        } else {
            (self.letter, self.acc)
        };
        let base = match letter {
            0 => 0,  // C
            1 => 2,  // D
            2 => 4,  // E
            3 => -1, // F
            4 => 1,  // G
            5 => 3,  // A
            _ => 5,  // B
        };
        base + acc * 7
    }
}

/// Guitar tuning as the MIDI note of each open string, string 1 (high E)
/// first.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    pub open_midi: [i32; 6],
}

impl Tuning {
    pub fn standard() -> Self {
        Tuning {
            open_midi: [64, 59, 55, 50, 45, 40],
        }
    }
}

/// A notated duration: LilyPond base length with optional augmentation dots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dur {
    pub base: u32,
    pub dots: u32,
}

impl Dur {
    /// Length in quarter notes.
    pub fn len(self) -> Rational {
        let base = Rational::new(4, self.base as i64);
        match self.dots {
            0 => base,
            1 => base * Rational::new(3, 2),
            _ => base * Rational::new(7, 4),
        }
    }

    pub fn to_lily(self) -> String {
        let mut s = self.base.to_string();
        for _ in 0..self.dots {
            s.push('.');
        }
        s
    }
}

const BASES: [u32; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

/// The single notated duration (at most one dot) equal to `len`, if any.
pub fn single_dur(len: Rational) -> Option<Dur> {
    for base in BASES {
        for dots in 0..=1 {
            let dur = Dur { base, dots };
            if dur.len() == len {
                return Some(dur);
            }
        }
    }
    None
}

/// Decompose `len` into a sequence of tied base durations, largest first.
pub fn split_dur(len: Rational) -> Vec<Dur> {
    let mut parts = Vec::new();
    let mut rem = len;
    while rem.is_positive() && parts.len() < 64 {
        match BASES
            .iter()
            .copied()
            .find(|&base| Rational::new(4, base as i64) <= rem)
        {
            Some(base) => {
                parts.push(Dur { base, dots: 0 });
                rem = rem - Rational::new(4, base as i64);
            }
            None => break, // remainder finer than a 128th; the grid was warned about
        }
    }
    parts
}

/// Durations for one note event: `dur_cols` grid columns starting at
/// `start_col` (1-based within the bar).
///
/// A note that begins on a beat and matches a single (possibly dotted)
/// duration is written as one note.  Anything else is split at the beat
/// boundaries it crosses and written as tied notes; this is what turns the
/// off-beat crotchets in the README example into pairs of tied quavers.
pub fn note_durs(start_col: usize, dur_cols: usize, beat_cols: usize, unit: Rational) -> Vec<Dur> {
    let len = unit * Rational::integer(dur_cols as i64);
    if beat_cols > 0
        && (start_col - 1).is_multiple_of(beat_cols)
        && let Some(dur) = single_dur(len)
    {
        return vec![dur];
    }
    let mut segs = Vec::new();
    let (mut col, mut rem) = (start_col, dur_cols);
    while rem > 0 {
        let Some(next) = next_beat_boundary(col, beat_cols) else {
            segs.push(rem);
            break;
        };
        let take = rem.min(next - col);
        segs.push(take);
        col += take;
        rem -= take;
    }
    let mut durs = Vec::new();
    for seg in segs {
        let seg_len = unit * Rational::integer(seg as i64);
        match single_dur(seg_len) {
            Some(dur) => durs.push(dur),
            None => durs.extend(split_dur(seg_len)),
        }
    }
    durs
}

/// The first beat boundary after `col` (exclusive), or `None` when the bar
/// has no beat grid.
fn next_beat_boundary(col: usize, beat_cols: usize) -> Option<usize> {
    (col - 1)
        .checked_div(beat_cols)
        .map(|k| 1 + (k + 1) * beat_cols)
}

/// Semitone offset of the natural notes c d e f g a b.
pub const NATURAL_PC: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const LETTERS: [&str; 7] = ["c", "d", "e", "f", "g", "a", "b"];

/// A pitch class spelled as a letter plus accidental.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spelling {
    pub letter: usize,
    pub acc: i32,
}

/// Pitch class to spelling map; `None` entries fall through to the next map.
pub type SpellingMap = [Option<Spelling>; 12];

/// Spell a pitch class with sharps; the fallback without key or chord
/// context.
pub fn sharp_spelling(pc: i32) -> Spelling {
    let pc = pc.rem_euclid(12);
    if let Some(letter) = NATURAL_PC.iter().position(|&p| p == pc) {
        Spelling { letter, acc: 0 }
    } else {
        let letter = (0..7)
            .rev()
            .find(|&l| NATURAL_PC[l] < pc)
            .expect("chromatic pitch class has a lower natural note");
        Spelling { letter, acc: 1 }
    }
}

/// Spelling implied by a key signature.
pub fn key_spelling_map(fifths: i32) -> SpellingMap {
    // Accidentals are added in the order of the circle of fifths.
    const SHARP_ORDER: [usize; 7] = [3, 0, 4, 1, 5, 2, 6]; // f c g d a e b
    const FLAT_ORDER: [usize; 7] = [6, 2, 5, 1, 4, 0, 3]; // b e a d g c f
    let order = if fifths > 0 { SHARP_ORDER } else { FLAT_ORDER };
    let mut acc = [0i32; 7];
    for i in 0..fifths.unsigned_abs().min(7) as usize {
        acc[order[i]] += if fifths > 0 { 1 } else { -1 };
    }
    let mut map = [None; 12];
    for letter in 0..7 {
        let pc = (NATURAL_PC[letter] + acc[letter]).rem_euclid(12);
        map[pc as usize] = Some(Spelling {
            letter,
            acc: acc[letter],
        });
    }
    map
}

/// Spelling implied by the tones of a chord.
pub fn chord_spelling_map(root: Spelling, qual: Qual) -> SpellingMap {
    let mut map = [None; 12];
    let root_pc = (NATURAL_PC[root.letter] + root.acc).rem_euclid(12);
    for &(semitones, letter_offset) in qual.intervals() {
        let letter = (root.letter + letter_offset as usize) % 7;
        let pc = (root_pc + semitones).rem_euclid(12);
        let diff = (pc - NATURAL_PC[letter]).rem_euclid(12);
        let acc = if diff <= 6 { diff } else { diff - 12 };
        map[pc as usize] = Some(Spelling { letter, acc });
    }
    map
}

/// Spell `midi` with the first map that covers its pitch class.
pub fn spell(midi: i32, chord: Option<&SpellingMap>, key: Option<&SpellingMap>) -> Spelling {
    let pc = midi.rem_euclid(12) as usize;
    if let Some(map) = chord
        && let Some(sp) = map[pc]
    {
        return sp;
    }
    if let Some(map) = key
        && let Some(sp) = map[pc]
    {
        return sp;
    }
    sharp_spelling(pc as i32)
}

/// LilyPond note name for `midi` spelled as `sp`, including octave marks.
pub fn lily_note_name(midi: i32, sp: Spelling) -> String {
    let octave = (midi - sp.acc - NATURAL_PC[sp.letter]) / 12 - 1;
    let mut name = acc_name(sp.letter, sp.acc);
    if octave > 3 {
        name.push_str(&"'".repeat((octave - 3) as usize));
    } else if octave < 3 {
        name.push_str(&",".repeat((3 - octave) as usize));
    }
    name
}

/// LilyPond pitch class name (no octave marks), used for chord symbols.
pub fn pitch_class_name(sp: Spelling) -> String {
    acc_name(sp.letter, sp.acc)
}

/// LilyPond name of a letter with an accidental, e.g. `fis`, `es` or `bes`.
fn acc_name(letter: usize, acc: i32) -> String {
    let letter = LETTERS[letter];
    if acc > 0 {
        return format!("{letter}{}", "is".repeat(acc as usize));
    }
    if acc < 0 {
        // Flats contract on e and a (ees -> es, aes -> as).
        let flats = (-acc) as usize;
        let suffix = if flats == 1 { "es" } else { "eses" };
        return match letter {
            "e" => suffix.to_string(),
            "a" => format!("a{suffix}"),
            l => format!("{l}{suffix}"),
        };
    }
    letter.to_string()
}

/// Chord quality: the chord tones as (semitones above the root, letter offset
/// from the root letter) pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Qual {
    Maj,
    Min,
    Seven,
    MajSeven,
    MinSeven,
    Aug,
    AugSeven,
    Dim,
    DimSeven,
    Sus2,
    Sus4,
    Six,
    MinSix,
    Nine,
}

impl Qual {
    pub fn intervals(self) -> &'static [(i32, i32)] {
        match self {
            Qual::Maj => &[(0, 0), (4, 2), (7, 4)],
            Qual::Min => &[(0, 0), (3, 2), (7, 4)],
            Qual::Seven => &[(0, 0), (4, 2), (7, 4), (10, 6)],
            Qual::MajSeven => &[(0, 0), (4, 2), (7, 4), (11, 6)],
            Qual::MinSeven => &[(0, 0), (3, 2), (7, 4), (10, 6)],
            Qual::Aug => &[(0, 0), (4, 2), (8, 4)],
            Qual::AugSeven => &[(0, 0), (4, 2), (8, 4), (10, 6)],
            Qual::Dim => &[(0, 0), (3, 2), (6, 4)],
            Qual::DimSeven => &[(0, 0), (3, 2), (6, 4), (9, 6)],
            Qual::Sus2 => &[(0, 0), (2, 1), (7, 4)],
            Qual::Sus4 => &[(0, 0), (5, 3), (7, 4)],
            Qual::Six => &[(0, 0), (4, 2), (7, 4), (9, 5)],
            Qual::MinSix => &[(0, 0), (3, 2), (7, 4), (9, 5)],
            Qual::Nine => &[(0, 0), (4, 2), (7, 4), (10, 6), (14, 1)],
        }
    }

    pub fn lily_suffix(self) -> &'static str {
        match self {
            Qual::Maj => "",
            Qual::Min => "m",
            Qual::Seven => "7",
            Qual::MajSeven => "maj7",
            Qual::MinSeven => "m7",
            Qual::Aug => "aug",
            Qual::AugSeven => "7.5+",
            Qual::Dim => "dim",
            Qual::DimSeven => "dim7",
            Qual::Sus2 => "sus2",
            Qual::Sus4 => "sus4",
            Qual::Six => "6",
            Qual::MinSix => "m6",
            Qual::Nine => "9",
        }
    }
}

/// A parsed chord symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub root: Spelling,
    pub qual: Qual,
    pub bass: Option<Spelling>,
}

impl Chord {
    /// LilyPond chord mode token with `duration` inserted after the root,
    /// e.g. `c1`, `d1/fis` or `a2:7.5+`.
    pub fn lily_with_duration(&self, duration: &str) -> String {
        let mut token = pitch_class_name(self.root);
        token.push_str(duration);
        if self.qual != Qual::Maj {
            token.push(':');
            token.push_str(self.qual.lily_suffix());
        }
        if let Some(bass) = self.bass {
            token.push('/');
            token.push_str(&pitch_class_name(bass));
        }
        token
    }
}

/// Parse a chord symbol such as `C`, `Am`, `Fmaj7`, `Aaug7` or `D/F#`.
pub fn parse_chord(token: &str) -> Option<Chord> {
    let chars: Vec<char> = token.chars().collect();
    let letter = letter_index(*chars.first()?)?;
    let mut i = 1;
    let mut acc = 0i32;
    while i < chars.len() && matches!(chars[i], '#' | 'b') {
        acc += if chars[i] == '#' { 1 } else { -1 };
        i += 1;
    }
    let rest: String = chars[i..].iter().collect();
    let (suffix, bass) = match rest.split_once('/') {
        Some((s, b)) => (s, Some(b)),
        None => (rest.as_str(), None),
    };
    let qual = match suffix.to_ascii_lowercase().as_str() {
        "" => Qual::Maj,
        "m" | "min" | "-" => Qual::Min,
        "7" => Qual::Seven,
        "maj7" => Qual::MajSeven,
        "m7" | "min7" => Qual::MinSeven,
        "aug" | "+" => Qual::Aug,
        "aug7" | "7#5" | "+7" => Qual::AugSeven,
        "dim" => Qual::Dim,
        "dim7" | "o7" => Qual::DimSeven,
        "sus4" | "sus" => Qual::Sus4,
        "sus2" => Qual::Sus2,
        "6" => Qual::Six,
        "m6" | "min6" => Qual::MinSix,
        "9" => Qual::Nine,
        _ => return None,
    };
    let bass = match bass {
        Some(token) => Some(parse_pitch_class(token)?),
        None => None,
    };
    Some(Chord {
        root: Spelling { letter, acc },
        qual,
        bass,
    })
}

fn letter_index(c: char) -> Option<usize> {
    match c.to_ascii_uppercase() {
        'A' => Some(5),
        'B' => Some(6),
        'C' => Some(0),
        'D' => Some(1),
        'E' => Some(2),
        'F' => Some(3),
        'G' => Some(4),
        _ => None,
    }
}

fn parse_pitch_class(token: &str) -> Option<Spelling> {
    let chars: Vec<char> = token.chars().collect();
    let letter = letter_index(*chars.first()?)?;
    let mut acc = 0i32;
    for &c in &chars[1..] {
        match c {
            '#' => acc += 1,
            'b' => acc -= 1,
            _ => return None,
        }
    }
    Some(Spelling { letter, acc })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(num: i64, den: i64) -> Rational {
        Rational::new(num, den)
    }

    #[test]
    fn rational_arithmetic() {
        assert_eq!(r(1, 2) + r(1, 3), r(5, 6));
        assert_eq!(r(1, 2) - r(1, 3), r(1, 6));
        assert_eq!(r(1, 2) * r(2, 3), r(1, 3));
        assert_eq!(r(1, 2) / r(1, 3), r(3, 2));
        assert!(r(1, 2) < r(2, 3));
        assert!(r(2, 3) > r(1, 2));
        assert_eq!(r(2, 4), r(1, 2));
        assert_eq!(r(-1, 2), r(1, -2));
        assert_eq!(r(1, 2).to_string(), "1/2");
        assert_eq!(r(4, 1).to_string(), "4");
    }

    #[test]
    fn single_and_split_durations() {
        assert_eq!(single_dur(r(4, 1)), Some(Dur { base: 1, dots: 0 }));
        assert_eq!(single_dur(r(2, 1)), Some(Dur { base: 2, dots: 0 }));
        assert_eq!(single_dur(r(1, 1)), Some(Dur { base: 4, dots: 0 }));
        assert_eq!(single_dur(r(1, 2)), Some(Dur { base: 8, dots: 0 }));
        assert_eq!(single_dur(r(1, 4)), Some(Dur { base: 16, dots: 0 }));
        assert_eq!(single_dur(r(3, 4)), Some(Dur { base: 8, dots: 1 }));
        assert_eq!(single_dur(r(3, 2)), Some(Dur { base: 4, dots: 1 }));
        assert_eq!(single_dur(r(3, 1)), Some(Dur { base: 2, dots: 1 }));
        assert_eq!(single_dur(r(5, 4)), None);
        assert_eq!(
            split_dur(r(5, 4)),
            vec![Dur { base: 4, dots: 0 }, Dur { base: 16, dots: 0 }]
        );
        assert_eq!(
            split_dur(r(7, 4)),
            vec![
                Dur { base: 4, dots: 0 },
                Dur { base: 8, dots: 0 },
                Dur { base: 16, dots: 0 }
            ]
        );
        assert_eq!(Dur { base: 8, dots: 1 }.to_lily(), "8.");
    }

    #[test]
    fn readme_bass_voice_durations() {
        // 4/4 with one semiquaver per column; the bass voice of the README
        // example is two quavers, a crotchet and a minim.
        let unit = r(1, 4);
        assert_eq!(note_durs(1, 2, 4, unit), vec![Dur { base: 8, dots: 0 }]);
        assert_eq!(note_durs(3, 2, 4, unit), vec![Dur { base: 8, dots: 0 }]);
        assert_eq!(note_durs(5, 4, 4, unit), vec![Dur { base: 4, dots: 0 }]);
        assert_eq!(note_durs(9, 8, 4, unit), vec![Dur { base: 2, dots: 0 }]);
    }

    #[test]
    fn readme_treble_voice_durations() {
        // The treble voice is a quaver, three pairs of tied quavers and a
        // final quaver.
        let unit = r(1, 4);
        let quaver = Dur { base: 8, dots: 0 };
        assert_eq!(note_durs(1, 2, 4, unit), vec![quaver]);
        assert_eq!(note_durs(3, 4, 4, unit), vec![quaver, quaver]);
        assert_eq!(note_durs(7, 4, 4, unit), vec![quaver, quaver]);
        assert_eq!(note_durs(11, 4, 4, unit), vec![quaver, quaver]);
        assert_eq!(note_durs(15, 2, 4, unit), vec![quaver]);
    }

    #[test]
    fn syncopated_durations() {
        let unit = r(1, 4);
        // Five semiquavers starting on the beat: crotchet ~ semiquaver.
        assert_eq!(
            note_durs(1, 5, 4, unit),
            vec![Dur { base: 4, dots: 0 }, Dur { base: 16, dots: 0 }]
        );
        // Five semiquavers starting off the beat: dotted quaver ~ quaver.
        assert_eq!(
            note_durs(2, 5, 4, unit),
            vec![Dur { base: 8, dots: 1 }, Dur { base: 8, dots: 0 }]
        );
        // A whole bar is a single semibreve.
        assert_eq!(note_durs(1, 16, 4, unit), vec![Dur { base: 1, dots: 0 }]);
    }

    #[test]
    fn pitch_names() {
        let sharp = |midi: i32| lily_note_name(midi, sharp_spelling(midi.rem_euclid(12)));
        assert_eq!(sharp(64), "e'"); // high E
        assert_eq!(sharp(60), "c'"); // middle C
        assert_eq!(sharp(48), "c");
        assert_eq!(sharp(40), "e,"); // low E
        assert_eq!(sharp(42), "fis,");
        assert_eq!(sharp(63), "dis'");
        assert_eq!(sharp(45), "a,");
        assert_eq!(lily_note_name(70, sharp_spelling(10)), "ais'");
        assert_eq!(lily_note_name(46, sharp_spelling(10)), "ais,");
        assert_eq!(sharp(59), "b");
    }

    #[test]
    fn key_spelling() {
        // G major spells F sharp.
        let g = key_spelling_map(1);
        assert_eq!(g[6], Some(Spelling { letter: 3, acc: 1 }));
        // F major spells B flat.
        let f = key_spelling_map(-1);
        assert_eq!(f[10], Some(Spelling { letter: 6, acc: -1 }));
        // C major leaves the naturals alone.
        let c = key_spelling_map(0);
        assert_eq!(c[0], Some(Spelling { letter: 0, acc: 0 }));
        assert_eq!(c[1], None);
    }

    #[test]
    fn chord_spelling() {
        // E7 spells G sharp.
        let e7 = chord_spelling_map(Spelling { letter: 2, acc: 0 }, Qual::Seven);
        assert_eq!(e7[8], Some(Spelling { letter: 4, acc: 1 }));
        // C7 spells B flat.
        let c7 = chord_spelling_map(Spelling { letter: 0, acc: 0 }, Qual::Seven);
        assert_eq!(c7[10], Some(Spelling { letter: 6, acc: -1 }));
        // D major spells F sharp.
        let d = chord_spelling_map(Spelling { letter: 1, acc: 0 }, Qual::Maj);
        assert_eq!(d[6], Some(Spelling { letter: 3, acc: 1 }));
    }

    #[test]
    fn chord_parsing() {
        let c = parse_chord("C").unwrap();
        assert_eq!(c.root, Spelling { letter: 0, acc: 0 });
        assert_eq!(c.qual, Qual::Maj);
        assert_eq!(c.bass, None);
        assert_eq!(parse_chord("Am").unwrap().qual, Qual::Min);
        assert_eq!(parse_chord("C7").unwrap().qual, Qual::Seven);
        assert_eq!(parse_chord("Fmaj7").unwrap().qual, Qual::MajSeven);
        assert_eq!(parse_chord("Aaug7").unwrap().qual, Qual::AugSeven);
        assert_eq!(parse_chord("Bbm").unwrap().root.acc, -1);
        assert_eq!(
            parse_chord("D/F#").unwrap().bass,
            Some(Spelling { letter: 3, acc: 1 })
        );
        assert!(parse_chord("X").is_none());
        assert!(parse_chord("C9").is_some());
        assert!(parse_chord("Csus4").is_some());
    }

    #[test]
    fn chord_lily_tokens() {
        assert_eq!(parse_chord("C").unwrap().lily_with_duration("1"), "c1");
        assert_eq!(
            parse_chord("D/F#").unwrap().lily_with_duration("1"),
            "d1/fis"
        );
        assert_eq!(parse_chord("E7").unwrap().lily_with_duration("2"), "e2:7");
        assert_eq!(parse_chord("Am").unwrap().lily_with_duration("2"), "a2:m");
        assert_eq!(
            parse_chord("Fmaj7").unwrap().lily_with_duration("2"),
            "f2:maj7"
        );
        assert_eq!(
            parse_chord("Aaug7").unwrap().lily_with_duration("2"),
            "a2:7.5+"
        );
        assert_eq!(parse_chord("Bb").unwrap().lily_with_duration("1"), "bes1");
    }

    #[test]
    fn key_fifths() {
        assert_eq!(
            Key {
                letter: 0,
                acc: 0,
                minor: false
            }
            .fifths(),
            0
        ); // C
        assert_eq!(
            Key {
                letter: 4,
                acc: 0,
                minor: false
            }
            .fifths(),
            1
        ); // G
        assert_eq!(
            Key {
                letter: 3,
                acc: 0,
                minor: false
            }
            .fifths(),
            -1
        ); // F
        assert_eq!(
            Key {
                letter: 6,
                acc: -1,
                minor: false
            }
            .fifths(),
            -2
        ); // Bb
        assert_eq!(
            Key {
                letter: 5,
                acc: 0,
                minor: true
            }
            .fifths(),
            0
        ); // Am
        assert_eq!(
            Key {
                letter: 2,
                acc: 0,
                minor: true
            }
            .fifths(),
            1
        ); // Em
        assert_eq!(
            Key {
                letter: 3,
                acc: 1,
                minor: true
            }
            .fifths(),
            3
        ); // F#m
        assert_eq!(
            Key {
                letter: 6,
                acc: -1,
                minor: true
            }
            .fifths(),
            -5
        ); // Bbm
    }
}
