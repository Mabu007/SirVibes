---
name: captions
description: Produce accurate, readable captions as a HyperFrames overlay and composite them onto the video.
when_to_use: The user wants subtitles, captions, or burned-in text from speech.
---

# Captions

## Purpose

Give a video accurate, legible captions that are comfortable to read at the
pace they appear — rendered as a designed layer, not stamped on by a subtitle
filter.

## When to use

- "Add captions to this."
- "Generate an SRT for this interview."
- Any deliverable for a feed, where most viewers watch muted.

## How captions are made here

Captions are a **HyperFrames composition rendered transparent and composited
onto the footage with ffmpeg**. Read the `hyperframes` skill before you build
one: it carries the composition contract, the render flags that produce a real
alpha channel, and the composite command.

**Never write `.ass` or `.ssa`, and never call ffmpeg with `-vf subtitles=`.**
An `.srt` or `.vtt` sidecar is a legitimate deliverable — a platform may want
one, and it costs nothing to write alongside the render — but it is never how
the words get onto the picture.

## Editorial principles

- **Accuracy first.** Captions that mishear a name or a number are worse than
  no captions. Verify proper nouns and figures against context.
- **Read speed governs everything.** Aim for at most ~17 characters per second
  on screen, and hold every cue at least ~1 second and at most ~7.
- **Two lines maximum, ~42 characters per line.** Break lines at grammatical
  boundaries — after punctuation, before a conjunction, never between an
  article and its noun.
- **Cues follow speech.** A cue starts when the words start, within ~100ms, and
  clears when they stop. Captions that lag or persist read as broken.
- **Verbatim, lightly cleaned.** Keep meaning and voice; drop stammers and
  false starts unless they carry something.
- Identify speakers when more than one person talks and it is not obvious.
- **Design is part of the job.** Because these are real compositions, the type
  is a decision: weight heavy enough to read at phone scale, a stroke or shadow
  strong enough to survive a bright background, and a word-level accent only
  when the piece wants that energy. Ask what the video is for if it is not
  obvious; do not default to novelty.

## Workflow

1. Confirm the audio track exists and is intelligible (`ffprobe`); extract it
   if the file is large — `ffmpeg -i in.mp4 -ac 1 -ar 16000 audio.wav` uploads
   far faster.
2. `transcribe` it. That is what gives you word-level timings, and the cues are
   built from those, not from guesses about pacing.
3. Build cues from the timings: split on sentence boundaries first, then on
   length, honouring the read-speed and line rules above. Keep the per-word
   start times — they are what make word-level animation possible.
4. Write the cue data into the composition as a JSON array and build the lines
   from it in script. That keeps a correction to one line of data rather than a
   hand edit across the DOM.
5. Render transparent and composite, following the `hyperframes` skill. The
   overlay on its own is not the deliverable — carry straight on to the
   composite, and to the music and audio mix if the piece calls for one.
6. Keep captions inside the title-safe area — roughly the middle 90% — and
   clear of the platform UI at the bottom of vertical video.
7. Write `captions.srt` beside the render when the user wants a sidecar too.
8. Spot-check: pull a frame from the **middle** of a cue — not the end, where
   everything has arrived — and `see` it. Confirm the text
   is present, legible against that part of the footage, correctly timed, and
   inside the frame.

## Constraints

- Do not paraphrase into a summary.
- Do not burn captions into a master; write a delivery copy into `out/`.
- Do not use a type size that fails at phone scale. For 1080x1920 that is
  ~54–64px with a strong outline or shadow, and it is a floor, not a target. If
  a line does not fit, break it over two lines — never shrink the type to make
  one long line fit, and never set `white-space: nowrap` to force it.
- Do not reveal words by fading them in where they already sit: the invisible
  ones still take up the line, so the visible word drifts off-centre. Show the
  line and move the emphasis, or let the line grow.
- Do not let a cue outlast its speech to fill a gap.

## Quality criteria

- Every spoken word is captioned.
- No cue exceeds two lines or the read-speed budget.
- Timing drift is imperceptible at the end of the file, not just the start.
- Text remains legible over the brightest part of the footage.
- The composite keeps the original audio and the full length of the source.

## Expected outputs

The composition that made them (kept, so a revision is an edit rather than a
rebuild), the transparent render, the captioned video in `out/`, and
`captions.srt` where a sidecar was asked for.

## Failure conditions

- Timings that drift progressively out of sync.
- Walls of text from unsegmented transcription output.
- Captions clipped by the frame edge or hidden behind platform UI.
- A black box where the overlay should be transparent — see the `hyperframes`
  skill's failure notes.
- Silently dropping inaudible passages instead of flagging them.
