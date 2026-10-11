tab2lilypond
============

tab2lilypond is a tool that converts guitar tab to lilypond notation. The
guitar tab syntax is a close to "natural" ASCII tab as possible and it's
possible to play directly from the ASCII source. However the syntax is
enriched with subtle additional symbols that mark things like separate
voices and to add extra details on the timings of each note.

Note timings come from the horizontal spacing (couple with the time signature).

Basics
------

Bars are marked with `|` characters, strings with a `-` and numerals for fret
numbers. Horizontal spacing marks the timing of each note and can be any number
of characters wide (providing it can divide nicely into the time signature).
A `#` preceded by whitespace starts a comment that runs to the end of the
line, and a line starting with `#` is a comment too. Comments are ignored
entirely and can appear anywhere, even between the lines of a system. A `#`
inside a `{...}` directive does not start a comment, so values such as
`{title:My #1 Hit}` are safe.

This example shows four identical bars with different widths.

~~~tab
{signature:4/4}
|----|--------|----------------|--------------------------------|
|---6|------6-|------------6---|------------------------6-------|
|--5-|----5---|--------5-------|----------------5---------------|
|-7--|--7-----|----7-----------|--------7-----------------------|
|8---|8-------|8---------------|8-------------------------------|
|----|--------|----------------|--------------------------------|
~~~

Each bar holds the same four crotchets; the wider the bar, the more
characters each crotchet takes.

Numeric figures join together to form a double digit fret number. Thus the
following example shows a bar consisting of four crotchets, each taking two
characters of width.

~~~tab
|--------|
|------11|
|----10--|
|--11----|
|12------|
|--------|
~~~

A wider bar can also hold a ternary grid. In 4/4 a 24 column bar puts a
triplet semiquaver on every column, so notes three columns apart are quavers
while notes four columns apart are triplet crotchets — three in the time of
two:

~~~tab
{signature:4/4}
|------------------------|------------------------|
|------------------------|------------------------|
|------------------------|------------------------|
|------------------------|------------------------|
|------------------------|------------------------|
|1--1--1--1--1--1--1--1--|1---1---1---1---1---1---|
~~~

The first bar is eight quavers. The second is six triplet crotchets, engraved
as two `\tuplet 3/2` groups. A single bar may mix both spacings, which is how
a bar of quavers ending in tripleted crotchets is written.


Header
------

Metadata is written as ChordPro-style `{key:value}` directives before the tab
systems, and no separator is needed between the header and the music. The
recognised keys are `title`, `composer`, `arranger`, `key`, `signature`
(shorthand `s`), `tempo` (beats per minute) and `tuning`.

Voices
------

For example the following is in 4/4 time and a bass voice on the bottom two
strings and the treble voice uses the top four strings (marked by the ! on the
fourth string). The bass voice consisting of two quavers, a crotchet
and a minim and the treble voice is . The treble voice is a quaver, followed by
three pairs of tied quavers and a final quaver:

~~~tab
{signature:4/4}
|----------2---5-|
|------4---------|
|--2-------------|
!4---------------|
|----2---4-------|
|2-5-------------|
~~~

The following example uses the . syntax element to change the fret where the
voices split. A . acts similar to a ! in that it marks the split point for all
new notes that appear horizontally after it. Beyond marking this change for the
future it acts like a -. This example has an alternating base line hitting each
crotchet and a melody line on each quaver which uses the same string used for
parts of the bass voice:

~~~tab
{signature:4/4}
|----------------|
|--------------2-|
!2-4---2---2-4---|
|----2--.4-------|
|------------2---|
|0-------3-------|
~~~

In a multi-bar tablature, the ! is optional on subsequent bars (including those
on subsequent lines). For example this bass voice continues to use the
bottom three strings in bar 2  this example also defaults to a 4/4 time
signature in the key of C):

~~~tab
|0-------1-------|3-----1-3-------|
|--1---3---1---3-|--0-3-----3-0---|
!----0-------0---|--------------0-|
|----2-------2---|----0-------0---|
|3-------3-------|----------------|
|----------------|3-------3-------|
~~~

Finally in a bar where all notes are from the same voice then the other voice
should be notated with invisible rests (like the second bar in this example):

~~~tab
|----------------|----------------|
|1---0-----0-----|1---------------|
!--------2-------|0---------------|
|----0-------0---|2---------------|
|----------------|3---------------|
|3-------3-------!----------------|
~~~

Arpeggios
---------

A `~` immediately before a fret number marks an arpeggiated note. The note
starts on the column of the `~` itself, not on the column of the number,
which only labels the fret. Arpeggiated notes are engraved with an arpeggio
(a wavy line across the chord) in the tablature staff:

~~~tab
{signature:4/4}
|~0--------------|
|~1--------------|
|~2--------------|
|----------------|
|----------------|
|----------------|
~~~

Here the chord starts on the first column even though the fret numbers sit
in the second, so it is engraved as a whole note with an arpeggio.

Repeats
-------

A `|:` in place of a barline starts a repeated section and a `:|` ends one;
a `:|` without a matching `|:` repeats from the previous `:|` (or from the
beginning). The section is engraved as a volta repeat and played twice:

~~~tab
{signature:4/4}
|:0-------0-------|0-------0-------:|
|:----------------|----------------:|
|:----------------|----------------:|
|:----------------|----------------:|
|:----------------|----------------:|
|:----------------|----------------:|
~~~

A bar that sets the voice split can start the repeat too: `!:` is the
voice marker and the repeat colon written as one token, at the start of a
line or in place of a mid-line barline.

~~~tab
{signature:4/4}
|0-------|0-------:|
|--------|--------:|
!:1-------|1-------:|
|--------|--------:|
|--------|--------:|
|--------|--------:|
~~~

The `!:` on the third line starts the repeat at the first bar while also
marking the voice split, exactly as a plain `!` would.

Alternative endings
-------------------

A line made of `+` markers above a system marks alternative endings for a
repeat. Each `+` sits on a barline of the system below; a number after it
starts that ending, which runs to the next `+` (or the end of the line).
The first ending's last bar carries the `:|` that sends the player back to
the start of the repeat, and the later endings are played on the repeat
passes:

~~~tab
{signature:4/4}
                   +1--------+2-------+
|:0---0---|0---0---|0---0---:|0---0---|0-------|
|:--------|--------|--------:|--------|--------|
|:--------|--------|--------:|--------|--------|
|:--------|--------|--------:|--------|--------|
|:--------|--------|--------:|--------|--------|
|:--------|--------|--------:|--------|--------|
~~~

The first time through, the music after the `|:` is played and then the
first ending; the `:|` jumps back to the `|:` and the second ending
replaces the first on the second pass. The endings do not need a system
of their own: they may follow the body bars in the same system, and the
endings line may be indented to bring each `+` onto the barline it marks.

 
## Usage

The tab is read from the file given on the command line (or standard input
when no file is given) and the LilyPond source is written to standard
output.  The score contains a conventional staff (in the octave-transposing
treble clef) above a tablature staff; every note in the tab staff carries a
string number, so the frets shown are exactly the ones in the tab rather
than a fingering guessed by LilyPond.  Warnings about ignored or unsupported
input go to standard error:

    cargo run -- examples/aura_lee.tab > aura_lee.ly
    lilypond aura_lee.ly

With `--midi` the tool writes a standard MIDI file to standard output
instead of LilyPond source: a tempo map track (title, key and time
signatures, tempo) followed by one track per voice, all on the General MIDI
acoustic guitar (steel) program.  Repeats are expanded so every pass is
played with its own alternative ending, and tied notes sound as one
sustained note:

    cargo run -- --midi examples/aura_lee.tab > aura_lee.mid

The fenced example above can be converted directly from this README:

    cargo run -- README.md > example.ly
