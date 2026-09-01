---
name: music
description: Place a music bed under a cut so the voice still leads, and finish the audio.
when_to_use: The user wants music, a soundtrack, a bed, or "make it feel like something" on a video that already has speech.
---

# Music

## Purpose

Put music under a piece without burying what is being said, and leave the audio
at a level that travels — a phone speaker, a laptop, a feed that normalises
loudness.

## When to use

- "Add music to this."
- "It feels flat / needs energy."
- Any short or reel going to a feed, where silence under the voice reads as
  unfinished.

## Editorial principles

- **The voice leads.** Music exists to carry the cut between sentences. If a
  listener has to concentrate to follow the words, the bed is too loud whatever
  the meter says.
- **Roughly 12–18 dB under the voice** while someone is speaking, and let it
  come up in the gaps. That difference is what ducking is for; a single fixed
  level either buries the voice or disappears.
- **Start and end deliberately.** A bed that snaps on at full level, or stops
  dead on the last frame, sounds like an accident. Fade in over ~0.5s and out
  over 1–2s.
- **Match the cut, not the mood board.** Tempo against the pace of the edit
  matters more than genre.
- **One bed.** Layering two tracks under speech is noise.
- Respect what the user owns. Use a file they gave you or something already in
  the workspace; if there is nothing, say so and ask rather than reaching for
  whatever is to hand.

## Workflow

1. Probe both the video and the music (`ffprobe`) — you need the video's exact
   duration and the music's, and whether the video actually has an audio track.
2. If the music is shorter than the video, loop it (`-stream_loop -1`) and trim
   to length. Never let it run out mid-piece.
3. Duck the bed under the voice and mix:

   ```
   ffmpeg -i captioned.mp4 -stream_loop -1 -i music.mp3 \
     -filter_complex "[1:a]volume=0.25,atrim=0:DURATION,asetpts=N/SR/TB,\
   afade=t=in:st=0:d=0.6,afade=t=out:st=FADE_START:d=1.2[bed];\
   [bed][0:a]sidechaincompress=threshold=0.03:ratio=9:attack=15:release=350[duck];\
   [0:a][duck]amix=inputs=2:normalize=0:duration=first[a]" \
     -map 0:v -map "[a]" -c:v copy -c:a aac -b:a 192k -movflags +faststart out/final.mp4
   ```

   `[bed][0:a]sidechaincompress` is the order that matters: the first input is
   what gets pushed down, the second is what pushes it. `normalize=0` on the
   mix stops ffmpeg quietly halving both inputs. `-c:v copy` means the picture
   is never re-encoded for the sake of the audio.
4. Check the level, don't assume it:

   ```
   ffmpeg -i out/final.mp4 -af volumedetect -f null -
   ```

   A mean around -20 dB with peaks near -1 dB is a healthy short. Silence, or a
   mean under about -40 dB, means the mix did not happen.
5. Listen to the balance the only way available to you: run `volumedetect` over
   the voice-only cut and over the final, and confirm the voice did not lose
   level in the mix.
6. For a longer piece, normalise the finished mix to -16 LUFS for a feed
   (`loudnorm=I=-16:TP=-1:LRA=11`), measured then applied — see the
   `podcast-editing` skill for the two-pass form.

## Constraints

- Never replace the original audio with music. Mix; do not overwrite.
- Never re-encode the video to add audio.
- Do not add music to a piece the user did not ask to have music.
- Do not normalise a bed to the same loudness as speech.

## Quality criteria

- Every word remains intelligible with the bed running.
- The bed rises in the gaps and sits back under the voice.
- No clipping: peaks below 0 dB.
- The music starts and ends on purpose.
- The final file has both streams, the full duration, and plays through.

## Expected outputs

The final video in `out/`, with the music mixed in, plus whatever intermediate
the composite produced.

## Failure conditions

- A final file whose audio is silent, or which lost the voice entirely.
- Music that stops before the video does, or loops audibly mid-phrase.
- A bed so loud the captions are the only way to follow the speech.
- Declaring the job finished on the intermediate rather than the mixed video.
