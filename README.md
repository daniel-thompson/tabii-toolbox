tab2lilypond
============

tab2lilypond is a tool that converts guitar tab to lilypond notation. The
guitar tab syntax is a close to "natural" ASCII tab as possible and it's
possible to play directly from the ASCII source. However the syntax is
enriched with subtle additional symbols that mark things like separate
voices and to add extra details on the timings of each note.

Note timings come from the horizontal spacing (couple with the time signature).

Voices
------

For example the following is in 4/4 time and a bass voice on the bottom two
strings and the treble voice uses the top four strings (marked by the ! on the
fourth string). The bass voice consisting of two quavers, a crotchet
and a minim and the treble voice is . The treble voice is a quaver, followed by
three pairs of tied quavers and a final quaver:

~~~tab
s:4/4
--
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
s:4/4
--
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


## Usage

The tab is read from the file given on the command line (or standard input
when no file is given) and the LilyPond source is written to standard
output.  The score contains a conventional staff (in the octave-transposing
treble clef) above a tablature staff; every note in the tab staff carries a
string number, so the frets shown are exactly the ones in the tab rather
than a fingering guessed by LilyPond.  Warnings about ignored or unsupported
input go to standard error:

    cargo run -- aura_lee.tab > aura_lee.ly
    lilypond aura_lee.ly

The fenced example above can be converted directly from this README:

    cargo run -- README.md > example.ly
