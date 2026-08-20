---
name: video-analysis
description: Survey and characterise the media in a workspace before any editing decision is made.
when_to_use: The user asks what footage they have, or any time you are about to cut, convert, or render material you have not inspected yet.
---

# Video Analysis

## Purpose

Turn a folder of opaque media files into an accurate picture of what the user
actually has, so that every later decision is grounded in facts rather than
assumptions.

## When to use

- "What do I have in this folder?"
- "Is this footage usable for X?"
- Before any cut, concat, transcode, or render on unfamiliar material.

## Editorial principles

- Probe, never guess. Duration, resolution, frame rate, codec, pixel format,
  audio channel layout and sample rate all change what commands are correct.
- Mixed properties are the story. Footage that is 24fps in one file and 30fps in
  another, or 48kHz stereo next to 44.1kHz mono, will break a naive concat. Say
  so before it bites.
- Report what matters to the user's goal, not everything ffprobe printed.

## Workflow

1. List the workspace and identify media by extension.
2. For each file, run a single structured probe:
   `ffprobe -v error -print_format json -show_format -show_streams FILE`
3. Extract per file: duration, container, video codec, resolution, frame rate,
   bitrate, audio codec, channels, sample rate.
4. Compare across files and flag inconsistencies explicitly.
5. Note anything that will need handling: variable frame rate, no audio track,
   rotation metadata, very high bitrate, unusual pixel format, corrupt files.
6. Summarise as a short table plus a plain-language read on what the material
   is good for.

## Constraints

- Do not transcode anything during analysis.
- Do not open or modify source files.
- For large folders, probe every file but summarise by group.

## Quality criteria

- Every claim traceable to a probe you actually ran.
- Inconsistencies between files are stated, not averaged away.
- The user learns something actionable, not a metadata dump.

## Expected outputs

A concise summary in the conversation. For more than a handful of files, also
write `media-inventory.md` in the workspace.

## Failure conditions

- Reporting duration or resolution you did not probe.
- Ignoring a file that failed to probe — say it is unreadable instead.
- Burying the finding that matters under raw ffprobe output.
