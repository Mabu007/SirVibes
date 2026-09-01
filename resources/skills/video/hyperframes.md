---
name: hyperframes
description: Build a transparent HyperFrames composition — captions, titles, motion graphics — and composite it onto footage with ffmpeg.
when_to_use: Anything that puts words or graphics on top of a video: captions, subtitles burned in, titles, lower thirds, callouts, kinetic type, overlays, animated logos.
---

# HyperFrames overlays

## Purpose

Put words and graphics onto footage as a designed, animated layer, by rendering
an HTML composition with a transparent background and compositing it over the
video. The footage is never re-encoded through a browser and never touched by a
subtitle filter.

## When to use

- "Add captions to this."
- "Put a title on the front."
- "Give it a lower third with my name."
- Any callout, kinetic type, animated logo or graphic that sits over a picture.

## The rule

**Captions and graphics are HyperFrames compositions. They are never `.ass`,
`.ssa`, or `-vf subtitles=`.** Those produce libass text with no design control
worth having, and this app does not use them. An `.srt` or `.vtt` sidecar is
still a fine *deliverable* when the user wants a subtitle file for a platform —
write it as well, alongside the render, never instead of it.

**Render the overlay on its own.** The composition contains the captions and
graphics and nothing else — no source video inside it, no background colour.
Chrome renders 1080p at a few frames a second; putting the footage through it
costs an hour and a generation of quality for nothing.

## Reaching the CLI

`hyperframes` if it is installed — check first, and use it when it is there.

Otherwise `npx -y hyperframes@latest <command>`. The first call downloads it and
a Chrome to render with, which takes a minute; **every later call still pays
about 10 seconds** re-resolving the package, and a caption job calls it twice.
If the user is going to make more than one video, it is worth telling them that
`npm i -g hyperframes` removes that wait for good — say so, and let them decide.

`hyperframes doctor` reports what is missing before a render fails on it.

## Workflow

1. **Probe the source.** The overlay has to match it exactly:

   ```
   ffprobe -v error -select_streams v:0 \
     -show_entries stream=width,height,r_frame_rate -show_entries format=duration \
     -of default=nw=1 source.mp4
   ```

2. **Scaffold**, once per project:

   ```
   hyperframes init overlay --example blank --non-interactive
   ```

   `index.html` and `hyperframes.json` are the whole project. Editing
   `index.html` by hand is normal and expected.

3. **Write the composition.** The contract that matters:

   - `html, body { background: transparent; }` — **never paint a background.**
     Any colour you set is what gets composited over the footage.
   - `html`/`body` width and height, and the root's `data-width`/`data-height`,
     are the source video's exact pixel dimensions.
   - The root carries `data-composition-id` and `data-duration` in seconds —
     the length of the source, so the overlay does not end early.
   - Every timed element gets `class="clip"`, `data-start` and `data-duration`,
     in seconds.
   - Animate on **one paused GSAP timeline** registered as
     `window.__timelines["<composition-id>"]`, with every tween placed at an
     absolute time. The renderer seeks that timeline frame by frame, so
     anything driven by `Date.now()`, CSS animations or a running RAF loop will
     tear or freeze. A tween at an absolute time always resolves to the same
     picture.
   - Keep content inside the middle ~90% of the frame, and clear of the bottom
     ~15% of a vertical video where the platform UI sits.
   - **A word that has not appeared yet must not hold its place.** Laying out
     the whole line and fading words in with `opacity` leaves the invisible ones
     occupying space, so the one visible word sits off to one side of the frame
     and the result reads like a stray default subtitle rather than a designed
     caption. Either show the whole line and animate emphasis onto the current
     word — colour, weight, a small scale — or build the line up so it stays
     centred as it grows. Check a frame mid-cue, not just at the end of one.

4. **Check before rendering.** It is seconds against minutes:

   ```
   hyperframes check
   ```

   Lint, runtime errors, layout and contrast, all in one gate. Fix what it
   reports.

5. **Render transparent.**

   ```
   hyperframes render --format webm -o renders/overlay.webm --fps 30 -q high
   ```

   - `--format webm` is VP9 with an alpha channel — the default choice.
   - `--format mov` is ProRes 4444, much larger, for handing to an editor.
   - `--format png-sequence` writes RGBA frames, for a compositor.
   - `--fps` must match the source frame rate.
   - `-q draft` while iterating; `-q high` for the delivery.

   Confirm the alpha actually survived — this is the one thing that silently
   goes wrong:

   ```
   ffprobe -v error -show_streams renders/overlay.webm | grep ALPHA_MODE
   ```

   `TAG:ALPHA_MODE=1` means there is a real alpha channel. (The stream still
   reports `pix_fmt=yuv420p`; VP9 carries alpha beside it, not in it.)

6. **Composite with ffmpeg.**

   ```
   ffmpeg -i source.mp4 -c:v libvpx-vp9 -i renders/overlay.webm \
     -filter_complex "[0:v][1:v]overlay=0:0:format=auto" \
     -map 0:a? -c:a copy \
     -c:v libx264 -crf 18 -preset medium -pix_fmt yuv420p -movflags +faststart \
     out/captioned.mp4
   ```

   **`-c:v libvpx-vp9` before `-i overlay.webm` is not optional.** It selects
   the decoder that reads VP9's alpha; without it ffmpeg's native decoder drops
   the alpha and the overlay lands as an opaque black rectangle over the
   picture. `-map 0:a? -c:a copy` keeps the original audio untouched, and the
   `?` means a silent source is not an error.

   For a ProRes overlay the same command works without the `-c:v libvpx-vp9`.

   `libx264 -crf 18` is the dependable default. If the capabilities list at the
   top of your instructions names a hardware encoder, it has been tested on this
   machine and is worth using for a long final encode — follow the flags it
   gives, and check the result plays before handing it over.

7. **Look at the result.** An exit code proves nothing about a picture:

   ```
   ffmpeg -ss <a time a caption is on screen> -i out/captioned.mp4 -frames:v 1 out/check.png
   ```

   Then `see` that frame — is the text there, is it legible against the
   footage, is it inside the safe area? Fix and re-render if it is not.

## A composition that works

This is a caption overlay for a 720x1280, 6-second source, rendered and
composited with the commands above. Start from it: change the dimensions, the
duration and the `CUES` data, and design the type for the piece in hand.

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=720, height=1280" />
    <script src="https://cdn.jsdelivr.net/npm/gsap@3.14.2/dist/gsap.min.js"></script>
    <style>
      * { margin: 0; padding: 0; box-sizing: border-box; }
      html, body {
        width: 720px;
        height: 1280px;
        overflow: hidden;
        /* Nothing behind the captions: the footage is composited under this. */
        background: transparent;
      }
      body { font-family: "Inter", "Helvetica Neue", Arial, sans-serif; }

      .cue {
        position: absolute;
        left: 6%;
        right: 6%;
        /* Above the platform UI at the bottom of a vertical frame. */
        bottom: 18%;
        text-align: center;
        font-size: 44px;
        font-weight: 800;
        line-height: 1.22;
        letter-spacing: -0.01em;
        color: #fff;
        text-shadow: 0 4px 18px rgba(0, 0, 0, 0.55);
        -webkit-text-stroke: 3px rgba(0, 0, 0, 0.85);
        paint-order: stroke fill;
      }
      .word { display: inline-block; margin: 0 0.14em; }
    </style>
  </head>
  <body>
    <div
      id="root"
      data-composition-id="captions"
      data-start="0"
      data-duration="6"
      data-width="720"
      data-height="1280"
    ></div>

    <script>
      // Cues built from word-level transcript timings. One object per caption
      // line; `words` carries the start of each word in seconds.
      const CUES = [
        {
          start: 0.30,
          end: 2.10,
          words: [
            { text: "Captions", start: 0.30 },
            { text: "rendered", start: 0.78 },
            { text: "in", start: 1.24 },
            { text: "HyperFrames", start: 1.42 }
          ]
        },
        {
          start: 2.30,
          end: 4.00,
          words: [
            { text: "on", start: 2.30 },
            { text: "a", start: 2.48 },
            { text: "transparent", start: 2.62 },
            { text: "layer", start: 3.30 }
          ]
        },
        {
          start: 4.20,
          end: 5.90,
          words: [
            { text: "composited", start: 4.20 },
            { text: "with", start: 4.86 },
            { text: "ffmpeg", start: 5.16 }
          ]
        }
      ];

      const root = document.getElementById("root");
      const tl = gsap.timeline({ paused: true });

      CUES.forEach((cue, index) => {
        const line = document.createElement("div");
        line.className = "cue clip";
        line.id = "cue-" + index;
        line.dataset.start = cue.start.toFixed(3);
        line.dataset.duration = (cue.end - cue.start).toFixed(3);

        cue.words.forEach((word) => {
          const span = document.createElement("span");
          span.className = "word";
          span.textContent = word.text;
          line.appendChild(span);

          // Each word lands on its own timing. Tweens sit on one paused
          // timeline at absolute times, so any seek resolves to the same frame.
          tl.fromTo(
            span,
            { opacity: 0, yPercent: 22, scale: 0.86 },
            { opacity: 1, yPercent: 0, scale: 1, duration: 0.22, ease: "back.out(2)" },
            word.start
          );
        });

        root.appendChild(line);
      });

      window.__timelines = window.__timelines || {};
      window.__timelines["captions"] = tl;
    </script>
  </body>
</html>
```

## Constraints

- Never write `.ass`/`.ssa`; never call ffmpeg with `-vf subtitles=`.
- Never put the source video inside the composition.
- Never paint a background colour on `html` or `body`.
- Never overwrite the source. Write to `out/`.
- Do not use CSS animations, `setInterval` or a RAF loop for motion — the
  renderer seeks, and only a paused timeline seeks correctly.

## Quality criteria

- The overlay's dimensions and frame rate match the source exactly.
- `ALPHA_MODE=1` on the rendered overlay.
- The footage is fully visible everywhere the graphics are not.
- The original audio is present and in sync.
- A frame pulled from the finished file, looked at, shows what was intended.

## The overlay is never the deliverable

A transparent WebM is an intermediate. It has no footage in it and no audio in
it, and `hasAudio:false` on it is correct rather than a fault. The work is not
finished when the renderer prints `Render complete`; it is finished when the
composite exists, has the original audio, and you have looked at a frame of it.

The order, every time:

```text
cut / source video  +  caption overlay  →  ffmpeg composite  →  music and audio  →  final video
```

If a stage produced its file, say so and go straight on to the next one. Never
hand back the overlay as though it were the video.

## Expected outputs

`overlay/index.html` (the composition, kept — it is how the next revision is
made), `renders/overlay.webm`, and the composite in `out/`.

## Failure conditions

- A black or coloured box over the footage — the alpha was lost, almost always
  a missing `-c:v libvpx-vp9`.
- Graphics that jitter or freeze — motion that is not on the paused timeline.
- An overlay shorter than the source, cutting the end of the video off in the
  composite.
- Re-encoded audio, or audio dropped entirely, when the source had some.
