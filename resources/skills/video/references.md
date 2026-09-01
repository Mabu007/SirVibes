---
name: references
description: Learn a look from a video someone else made, without taking a copy of it.
when_to_use: The user points at a video — a link or a clip — and wants theirs to be like it.
---

# References

## Purpose

Turn "make it look like this" into instructions specific enough to build from.

## When to use

- "Make my captions look like this." (a YouTube link)
- "Use the editing style from this Reel."
- "Analyse the pacing of this Short."
- "Copy the transition at 00:12."

## The rule

**A reference is watched, never taken.** `analyze_reference` sends the link to a
model that opens the video where it lives. Nothing is downloaded, nothing is
kept, and there is no copy in the workspace.

**Never use `yt-dlp`, `curl` or anything else to fetch a reference.** If it
cannot be watched remotely, that is an answer — say so and ask for the clip.
Downloading someone's video to look at it is not a fallback, it is a different
thing that the user did not ask for.

## What can be watched

| The user gives you | What to do |
|---|---|
| A YouTube link (`youtube.com`, `youtu.be`, `/shorts/`) | `analyze_reference` |
| An Instagram link | Cannot be opened remotely. Ask them to add the clip to the chat, then `see` it. |
| A direct file link (`…/clip.mp4`) | The provider will not fetch it. Ask for the file itself. |
| A file they added to the chat | `see` it — that is the local path, and it works today |

When a link cannot be watched, say exactly that and offer the next step:

> I couldn't open that link with the video viewer available here. If you drop
> the clip into the chat, I can look at it directly.

Never describe a reference you could not see. If the tool refuses, you have not
seen it, and there is nothing to report.

## Ask for what you need, not for everything

`scope` is the difference between an answer and an essay, and between cents and
dollars. Match it to what was actually asked:

| They said | `scope` |
|---|---|
| "make my captions look like this" | `captions` |
| "copy these transitions" | `transitions` |
| "match the pacing" / "the cuts" | `pacing` |
| "the punch-ins", "how it moves" | `camera` |
| "the colour", "the grade" | `color` |
| "the whole style", "edit it like this" | `full` |

`start_seconds` / `end_seconds` narrow what comes back to a section — "the
transition at 00:12", "the first 15 seconds". Be honest about what that does:
the provider still watches the whole video, so the range focuses the answer, not
the cost.

Pass `instruction` — the user's own words. It decides what matters when the
reference does several things at once.

## Workflow

1. `analyze_reference` with the narrowest `scope` that answers the request.
2. Read what came back. It is JSON, saved in `references/`, and it is the brief
   for what you build — sizes relative to frame height, hex colours, words per
   phrase, entrance timings.
3. Build from it with the existing pipeline: captions are a HyperFrames
   composition (the `captions` and `hyperframes` skills), cuts are ffmpeg. The
   analysis says *what*; those skills say *how*.
4. If the analysis reports low confidence, or two conflicting looks, ask the
   user which one they meant with `ask_user` — one question, in their words.
5. Tell the user what you took from the reference, in a sentence. "Big bold
   captions, four words at a time, active word in yellow" — not a JSON dump.

## Constraints

- Never download a reference.
- Never invent what a reference looks like. If it could not be watched, say so.
- Do not analyse `full` when the user asked about one thing.
- Do not copy a reference's *content* — the words, the footage, the music. The
  look and the structure are what is being learned.
- Music: a reference's track is somebody's licensed music. Match the mood with
  something the user owns or something royalty-free; never lift the audio.

## Quality criteria

- The scope matches what was asked.
- Every claim about the reference came from the analysis, not from the link,
  the title, or what videos like that usually do.
- The finished piece visibly shares the quality the user pointed at.
- The user is told what was borrowed in plain language.

## Failure conditions

- A reference downloaded to disk.
- A confident description of a video that was never opened.
- A caption request answered with a full editing breakdown.
- Copying the reference's music or footage rather than its style.
