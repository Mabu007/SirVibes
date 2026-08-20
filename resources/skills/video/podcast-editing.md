---
name: podcast-editing
description: Turn raw recorded conversation into a clean, listenable episode.
when_to_use: The user has a recorded interview or conversation to assemble, tighten, or clean up.
---

# Podcast Editing

## Purpose

Take raw conversation recordings and produce an episode that sounds
intentional: level, clean, and paced.

## When to use

- "Clean up this interview."
- "Assemble these tracks into an episode."
- Multi-track remote recordings that need syncing and mixing.

## Editorial principles

- **Serve the conversation.** Cut what wastes the listener's time — dead air,
  restarts, technical fumbling, the ten minutes before anyone says anything.
  Keep what makes people sound human.
- **Breathing room is content.** Do not remove every pause; conversation without
  pauses is exhausting. Trim silences over roughly 1.5 seconds rather than
  eliminating them.
- **Consistent loudness.** Target −16 LUFS integrated for stereo podcast
  distribution (−19 for mono), with true peak at or below −1 dBTP.
- **Fix at the source of the problem.** Hum is a filter problem, uneven levels
  are a normalisation problem, and a bad take is an editing problem. Do not
  reach for compression to solve all three.
- Preserve each speaker's own track where you have one; process per speaker
  before mixing.

## Workflow

1. Probe every file: duration, sample rate, channels, and whether tracks are
   the same length (`video-analysis` applies here too).
2. Sync multi-track recordings. Use a common reference — a clap, or cross
   correlation of the tracks — and confirm the offset by checking alignment at
   both the start and the end, since clock drift accumulates.
3. Per speaker: high-pass around 80Hz, remove hum if present, then gentle
   compression, then normalise.
4. Assemble the edit. Write the cut list to `edit-plan.md` with timestamps and
   reasons before making it, so the user can review the shape of the episode.
5. Trim long silences and remove the obvious dross.
6. Mix to the loudness target — measure with `loudnorm` in analysis mode, then
   apply with the measured values (two-pass) rather than guessing.
7. Add intro/outro if supplied, with a short crossfade rather than a hard butt.
8. Verify the master: measure loudness and peak on the finished file and confirm
   the numbers, and confirm the duration matches the edit plan.

## Constraints

- Never edit the source recordings in place.
- Do not apply noise reduction aggressively enough to make voices sound
  underwater; a little hiss beats artefacts.
- Do not gate speech; gates clip the ends of quiet words.
- Keep the video in sync if there is video — any audio edit must be mirrored.

## Quality criteria

- Integrated loudness within ±1 LU of target, true peak ≤ −1 dBTP.
- No audible clicks at edit points.
- Speakers sit at comparable levels; no one makes the listener reach for the
  volume.
- Cuts are inaudible unless they are meant to be heard.

## Example

Two-pass loudness normalisation, measuring first and then applying:

```
ffmpeg -i mix.wav -af loudnorm=I=-16:TP=-1:LRA=11:print_format=json -f null -
ffmpeg -i mix.wav -af loudnorm=I=-16:TP=-1:LRA=11:measured_I=…:measured_TP=…:\
measured_LRA=…:measured_thresh=…:offset=…:linear=true -c:a aac -b:a 192k \
  out/episode-042.m4a
```

## Expected outputs

`edit-plan.md`, and a mastered episode in `out/`.

## Failure conditions

- Delivering an episode whose loudness you never measured.
- Sync that is correct at the start and drifting by the end.
- Cutting so tightly that the conversation loses its rhythm.
- Processing applied to the mix that should have been applied per track.
