---
name: captions
description: Produce accurate, readable captions and burn or attach them correctly.
when_to_use: The user wants subtitles, captions, or burned-in text from speech.
---

# Captions

## Purpose

Give a video accurate, legible captions that are comfortable to read at the pace
they appear.

## When to use

- "Add captions to this."
- "Generate an SRT for this interview."
- Any deliverable for a feed, where most viewers watch muted.

## Editorial principles

- **Accuracy first.** Captions that mishear a name or a number are worse than no
  captions. Verify proper nouns and figures against context.
- **Read speed governs everything.** Aim for at most ~17 characters per second
  on screen, and hold every cue at least ~1 second and at most ~7.
- **Two lines maximum, ~42 characters per line.** Break lines at grammatical
  boundaries — after punctuation, before a conjunction, never between an article
  and its noun.
- **Cues follow speech.** A cue starts when the words start, within ~100ms, and
  clears when they stop. Captions that lag or persist read as broken.
- **Verbatim, lightly cleaned.** Keep meaning and voice; drop stammers and false
  starts unless they carry something.
- Identify speakers when more than one person talks and it is not obvious.

## Workflow

1. Confirm the audio track exists and is intelligible (`ffprobe`); extract it if
   transcription needs a separate file.
2. Transcribe with word or segment timings.
3. Build cues from the timings: split on sentence boundaries first, then on
   length, honouring the read-speed and line rules above.
4. Write `captions.srt` (or `.vtt` for web, `.ass` when styling is required).
5. Deliver either as a sidecar file, a soft-muxed track, or burned in — ask
   which if the user has not said.
   - Soft mux: `-c copy -c:s mov_text` for MP4.
   - Burn in: `-vf subtitles=captions.srt` (use `.ass` for control of font,
     size, outline and position).
6. When burning in, keep captions inside the title-safe area — roughly the
   middle 90% — and clear of platform UI at the bottom of vertical video.
7. Spot-check the result: extract a frame at a known cue time and confirm the
   text is present, legible and correctly timed.

## Constraints

- Do not paraphrase into a summary.
- Do not burn captions into a master; burn into a delivery copy.
- Do not use a font size that fails at phone scale — for 1080x1920, ~54–64px
  with a strong outline or shadow.

## Quality criteria

- Every spoken word is captioned.
- No cue exceeds two lines or the read-speed budget.
- Timing drift is imperceptible at the end of the file, not just the start.
- Text remains legible over the brightest part of the footage.

## Expected outputs

`captions.srt` (or `.vtt`/`.ass`), and where requested a captioned render in
`out/`.

## Failure conditions

- Timings that drift progressively out of sync.
- Walls of text from unsegmented transcription output.
- Captions clipped by the frame edge or hidden behind platform UI.
- Silently dropping inaudible passages instead of flagging them.
