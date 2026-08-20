---
name: shorts
description: Cut short-form vertical clips out of long-form footage, selected for standalone impact.
when_to_use: The user wants Shorts, Reels, TikToks, or short clips extracted from a longer video.
---

# Shorts

## Purpose

Find the moments in a long video that work on their own, and cut them into
vertical clips that hold attention from the first frame.

## When to use

- "Turn this podcast into three Shorts."
- "Pull the best moments out of this talk."

## Editorial principles

- **A Short is a complete thought.** It needs a beginning, a turn, and a
  resolution. A clip that starts mid-sentence or ends before the payoff fails,
  no matter how good the moment was in context.
- **The first two seconds decide everything.** Open on the strongest line, not
  on setup, throat-clearing, or "so, anyway". Cut the run-up.
- **One idea per clip.** If you can't say what the clip is about in one line, it
  is two clips or none.
- **Tension, not just information.** A claim someone might disagree with, a
  surprising number, a story turn, a genuine laugh. Neutral exposition does not
  travel.
- **Density.** 30–60 seconds is the working range. Under 15s rarely lands an
  idea; over 90s needs a very strong reason.
- Leave a breath — roughly 200–400ms — before the first word and after the last.
  Hard cuts on the consonant sound like mistakes.

## Workflow

1. Read the `video-analysis` skill's output or probe the source yourself.
2. Get the words. Use an existing transcript if there is one; otherwise
   transcribe with whatever is available on the machine.
3. Read the transcript for moments, not keywords. Mark candidate in/out points
   with timestamps and write them to `shorts-plan.md` with a one-line reason for
   each, so the user can see your reasoning and correct it.
4. Prefer more candidates than requested, then choose the strongest.
5. Cut each clip frame-accurately (re-encode; do not stream-copy an arbitrary
   in-point).
6. Reframe to vertical 1080x1920. Keep the speaker's eyeline in the upper third;
   crop rather than pillarbox when a subject is clearly framed.
7. Verify every output: it exists, its duration matches the plan, it has audio.
8. Name outputs `short-01-<slug>.mp4` where the slug says what the moment is.

## Constraints

- Never cut mid-word.
- Do not speed-ramp or add music unless asked.
- Do not stretch a clip to hit a target length by including filler.
- Sources are read-only; deliverables go to `out/`.

## Quality criteria

- Each clip is intelligible to someone who has not seen the source.
- The hook is in the first two seconds.
- Audio is continuous and clean across the cut points.
- Vertical framing keeps the subject centred and un-cropped at the head.

## Example

Source is a 118-minute interview. `shorts-plan.md` records:

```
short-01-funding-myth   00:14:22.400 → 00:15:03.100  (61s)
  Hook: "Most founders raise too early and it costs them the company."
  Complete argument, ends on the consequence. Strong disagreement bait.
```

Then, frame-accurate cut and vertical reframe:

```
ffmpeg -i source.mp4 -ss 00:14:22.400 -to 00:15:03.100 \
  -vf "crop=ih*9/16:ih,scale=1080:1920" \
  -c:v libx264 -crf 20 -preset medium -c:a aac -b:a 160k \
  out/short-01-funding-myth.mp4
```

## Expected outputs

`shorts-plan.md` with timestamps and reasoning, plus one `out/short-NN-slug.mp4`
per clip.

## Failure conditions

- Clips that start or end mid-sentence.
- Slicing on a fixed interval instead of on meaning.
- Delivering clips you never verified.
- Choosing moments by keyword frequency rather than by whether they land.
