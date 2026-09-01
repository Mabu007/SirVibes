//! The whole pipeline, end to end, on real footage.
//!
//! Everything here runs through the code the app runs: commands go through
//! `tools_shell::run_core` with a real `Jobs` entry, the composition is written
//! with the real `fs_write` tool, the transcript comes from the real Deepgram
//! call, and the finished video is checked by the real `see`. Nothing is
//! stubbed, no completion is simulated, and no artifact is assumed to exist.
//!
//! It is `#[ignore]`d because it spends the user's Deepgram and OpenRouter
//! credit and takes minutes of real rendering:
//!
//! ```text
//! SIRVIBE_E2E_SETTINGS=~/.config/com.sirvibe.agent/settings.json \
//! SIRVIBE_E2E_WORKSPACE=~/SirVibe-e2e/pipeline-run \
//!   cargo test -- --ignored --nocapture real_pipeline
//! ```

use crate::jobs::Jobs;
use crate::settings::Settings;
use crate::tools_shell::{run_core, OutputSink};
use crate::workspace::Workspace;
use crate::{deepgram, tools_fs, vision};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

const DEFAULT_SOURCE: &str = "/home/gift/Videos/African-Video/Africa.mp4";
const DEFAULT_MUSIC: &str =
    "/home/gift/Music/modern melodic trap instrumental with South African hip-hop influence.mp3";
/// A slice with continuous speech in it, so the captions have something to say.
const CUT_FROM: f64 = 24.5;
const CUT_LENGTH: f64 = 10.0;
const FPS: u32 = 25;
const WIDTH: u32 = 720;
const HEIGHT: u32 = 1280;

/// The stages this pipeline moves through. A stage is only left when the work
/// that defines it has actually produced its artifact.
#[derive(Debug, PartialEq)]
enum Stage {
    Cutting,
    Transcribing,
    CaptionsAuthoring,
    CaptionsRendering,
    Compositing,
    MusicMixing,
    Validating,
    Completed,
}

struct Run {
    ws: Workspace,
    jobs: Arc<Jobs>,
    stage: Stage,
    call: usize,
}

impl Run {
    fn advance(&mut self, next: Stage) {
        println!("[pipeline] {:?} → {:?}", self.stage, next);
        self.stage = next;
    }

    /// One command, through the runner the app uses, with the completion
    /// contract checked rather than assumed.
    async fn shell(&mut self, command: &str) -> Value {
        self.call += 1;
        let call_id = format!("e2e-{}", self.call);
        let (job, guard) = self.jobs.start(&call_id);
        let sink: OutputSink = Arc::new(|_, _| {});
        let result = run_core(&self.ws, command, 1800, sink, &job, &call_id)
            .await
            .expect("the runner must return a result, never an error");
        drop(guard);

        assert_eq!(
            result["status"], "completed",
            "stage {:?} did not complete: {}",
            self.stage, result
        );
        assert_eq!(
            result["exit_code"], 0,
            "stage {:?} failed: {}",
            self.stage,
            result["stderr"].as_str().unwrap_or_default()
        );
        assert!(
            !self.jobs.is_running(&call_id),
            "a finished command must not be left registered as running"
        );
        result
    }

    fn require(&self, rel: &str) -> u64 {
        let path = self.ws.resolve(rel);
        let size = std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("{} was never produced: {}", rel, e))
            .len();
        assert!(size > 0, "{} is empty", rel);
        size
    }
}

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

/// Installed, or reached through npx exactly as the skill tells the agent to.
fn hyperframes() -> String {
    if let Ok(explicit) = std::env::var("SIRVIBE_E2E_HYPERFRAMES") {
        return explicit;
    }
    let on_path = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("hyperframes").is_file())
        })
        .unwrap_or(false);
    if on_path {
        "hyperframes".into()
    } else {
        "npx -y hyperframes@latest".into()
    }
}

#[tokio::test]
#[ignore]
async fn real_pipeline_cuts_captions_composites_and_scores_a_finished_video() {
    let settings_path = PathBuf::from(env_or(
        "SIRVIBE_E2E_SETTINGS",
        "/home/gift/.config/com.sirvibe.agent/settings.json",
    ));
    let settings = Settings::load(&settings_path);
    assert!(
        !settings.deepgram_api_key.trim().is_empty(),
        "this exercises the real transcription; a Deepgram key must be configured"
    );

    let source = env_or("SIRVIBE_E2E_SOURCE", DEFAULT_SOURCE);
    let music = env_or("SIRVIBE_E2E_MUSIC", DEFAULT_MUSIC);
    assert!(PathBuf::from(&source).exists(), "no source footage at {}", source);
    assert!(PathBuf::from(&music).exists(), "no music at {}", music);

    let root = PathBuf::from(env_or(
        "SIRVIBE_E2E_WORKSPACE",
        &std::env::temp_dir().join("sirvibe-pipeline-e2e").to_string_lossy(),
    ));
    std::fs::create_dir_all(&root).unwrap();
    let ws = Workspace::open(&root.to_string_lossy()).unwrap();
    println!("[pipeline] workspace {}", ws.root.display());

    let mut run = Run {
        ws,
        jobs: Arc::new(Jobs::new()),
        stage: Stage::Cutting,
        call: 0,
    };

    // ---------------------------------------------------------------- cut
    // A real edit: a chosen in-point, a real duration, reframed to vertical.
    println!("[pipeline] stage {:?}", run.stage);
    run.shell(&format!(
        "mkdir -p out work && ffmpeg -hide_banner -loglevel error -ss {} -t {} -i '{}' \
         -vf \"scale={}:{}:force_original_aspect_ratio=increase,crop={}:{},fps={}\" \
         -c:v libx264 -crf 20 -preset veryfast -pix_fmt yuv420p \
         -c:a aac -b:a 192k -movflags +faststart -y work/cut.mp4",
        CUT_FROM, CUT_LENGTH, source, WIDTH, HEIGHT, WIDTH, HEIGHT, FPS
    ))
    .await;
    run.require("work/cut.mp4");

    let probe = run
        .shell("ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 work/cut.mp4")
        .await;
    let cut_seconds: f64 = probe["stdout"].as_str().unwrap().trim().parse().unwrap();
    assert!(
        (cut_seconds - CUT_LENGTH).abs() < 0.5,
        "the cut is {}s, not the {}s that was asked for",
        cut_seconds,
        CUT_LENGTH
    );

    // -------------------------------------------------------- transcribe
    run.advance(Stage::Transcribing);
    run.shell(
        "ffmpeg -hide_banner -loglevel error -i work/cut.mp4 -ac 1 -ar 16000 -y work/cut.wav",
    )
    .await;
    let transcript = deepgram::transcribe(
        &run.ws,
        &settings.deepgram_api_key,
        &json!({ "path": "work/cut.wav", "save_as": "work/cut-transcript" }),
    )
    .await
    .expect("real transcription");
    println!(
        "[transcribe] {} utterance(s) over {}s",
        transcript["utterance_count"], transcript["duration_seconds"]
    );

    let raw = tools_fs::read(&run.ws, &json!({ "path": "work/cut-transcript.json" }))
        .expect("the word-level transcript is written to disk");
    let parsed: Value = serde_json::from_str(raw["content"].as_str().unwrap()).unwrap();
    let words = parsed
        .pointer("/results/channels/0/alternatives/0/words")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        words.len() > 5,
        "real speech should give real words, got {}",
        words.len()
    );

    // ---------------------------------------------------- author captions
    run.advance(Stage::CaptionsAuthoring);
    let cues = cues_from(&words, cut_seconds);
    assert!(!cues.is_empty(), "words must become cues");
    let first_cue_at = cues[0].0;
    let sample_at = cues[0].2 + 0.35;
    let spoken: String = cues[0]
        .3
        .iter()
        .map(|(w, _)| w.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "[captions] {} cue(s); first at {:.2}s: {:?}",
        cues.len(),
        first_cue_at,
        spoken
    );

    tools_fs::write(
        &run.ws,
        &json!({
            "path": "captions/index.html",
            "content": composition(&cues, cut_seconds),
        }),
    )
    .expect("the composition is written with the real fs tool");
    tools_fs::write(
        &run.ws,
        &json!({ "path": "captions/hyperframes.json", "content": "{}\n" }),
    )
    .unwrap();

    // ---------------------------------------------------- render captions
    run.advance(Stage::CaptionsRendering);
    let hf = hyperframes();
    let browsers_before = browser_processes();
    run.shell(&format!("cd captions && {} check", hf)).await;
    let render = run
        .shell(&format!(
            "cd captions && {} render --format webm -o ../out/captions.webm --fps {} -q high",
            hf, FPS
        ))
        .await;

    // The stage's completion contract, checked rather than inferred from what
    // the renderer printed. `shell` has already asserted status and exit code.
    let digest = render["stdout"].as_str().unwrap_or_default();
    println!(
        "[captions] digest {} bytes, {} progress updates, reading: {}",
        digest.len(),
        render["progress"]["updates"],
        render["progress"]["summary"]
    );
    assert!(
        digest.len() < 8_000,
        "the model was handed {} bytes of render log; progress should have been collapsed",
        digest.len()
    );
    assert!(!digest.contains('\u{fffd}'), "a character was broken in the capture");
    assert!(
        digest.contains("[progress]"),
        "the collapsed progress should still be in the log: {}",
        digest
    );
    let updates = render["progress"]["updates"].as_u64().unwrap_or(0);
    assert!(updates > 50, "a real render redraws far more than {} times", updates);
    assert_eq!(render["progress"]["percent"], 100, "it should have finished at 100%");
    if let Some(log) = render["raw_log"].as_str() {
        println!("[captions] full log kept at {}", log);
    }

    // The artifact, not the log line, is what says the stage is done.
    let overlay_bytes = run.require("out/captions.webm");
    println!("[captions] overlay {} bytes", overlay_bytes);

    // Nothing the renderer started may outlive it.
    let strays: Vec<u32> = browser_processes()
        .into_iter()
        .filter(|pid| !browsers_before.contains(pid))
        .collect();
    assert!(strays.is_empty(), "the render left browsers behind: {:?}", strays);

    let alpha = run
        .shell("ffprobe -v error -show_streams out/captions.webm | grep -i alpha_mode || true")
        .await;
    assert!(
        alpha["stdout"].as_str().unwrap().to_lowercase().contains("alpha_mode=1"),
        "the overlay has no alpha channel; compositing it would paint a black box"
    );

    // ------------------------------------------------------- composite
    run.advance(Stage::Compositing);
    run.shell(
        "ffmpeg -hide_banner -loglevel error -i work/cut.mp4 -c:v libvpx-vp9 -i out/captions.webm \
         -filter_complex '[0:v][1:v]overlay=0:0:format=auto' \
         -map 0:a? -c:a copy -c:v libx264 -crf 18 -preset veryfast -pix_fmt yuv420p \
         -movflags +faststart -y work/captioned.mp4",
    )
    .await;
    run.require("work/captioned.mp4");
    let carried = run
        .shell(
            "ffprobe -v error -select_streams a -show_entries stream=codec_type              -of default=nw=1:nk=1 work/captioned.mp4",
        )
        .await;
    assert_eq!(
        carried["stdout"].as_str().unwrap().trim(),
        "audio",
        "compositing dropped the voice track"
    );

    // ----------------------------------------------------------- music
    run.advance(Stage::MusicMixing);
    run.shell(&format!(
        "ffmpeg -hide_banner -loglevel error -i work/captioned.mp4 -stream_loop -1 -i '{}' \
         -filter_complex \"[1:a]volume=0.25,atrim=0:{dur},asetpts=N/SR/TB,\
         afade=t=in:st=0:d=0.6,afade=t=out:st={fade}:d=1.2[bed];\
         [bed][0:a]sidechaincompress=threshold=0.03:ratio=9:attack=15:release=350[duck];\
         [0:a][duck]amix=inputs=2:normalize=0:duration=first[a]\" \
         -map 0:v -map '[a]' -c:v copy -c:a aac -b:a 192k -movflags +faststart -y out/final.mp4",
        music,
        dur = format!("{:.2}", cut_seconds),
        fade = format!("{:.2}", (cut_seconds - 1.2).max(0.1)),
    ))
    .await;
    run.require("out/final.mp4");

    // -------------------------------------------------------- validate
    run.advance(Stage::Validating);
    let streams = run
        .shell(
            "ffprobe -v error -show_entries stream=codec_type,codec_name,width,height \
             -show_entries format=duration -of default=nw=1 out/final.mp4",
        )
        .await;
    let report = streams["stdout"].as_str().unwrap();
    println!("[validate] {}", report.replace('\n', " · "));
    assert!(report.contains("codec_type=video"), "no video stream: {}", report);
    assert!(report.contains("codec_type=audio"), "no audio stream: {}", report);
    assert!(report.contains(&format!("width={}", WIDTH)));
    let final_seconds: f64 = report
        .lines()
        .find_map(|l| l.strip_prefix("duration="))
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        (final_seconds - cut_seconds).abs() < 0.6,
        "the final video is {}s but the cut was {}s",
        final_seconds,
        cut_seconds
    );

    // Every frame must decode: a file that plays for a second and then breaks
    // is not a finished video.
    let decode = run
        .shell("ffmpeg -hide_banner -v error -i out/final.mp4 -f null - 2>&1")
        .await;
    assert!(
        decode["stderr"].as_str().unwrap_or_default().trim().is_empty()
            && decode["stdout"].as_str().unwrap_or_default().trim().is_empty(),
        "decoding the final video reported errors: {}{}",
        decode["stdout"].as_str().unwrap_or_default(),
        decode["stderr"].as_str().unwrap_or_default()
    );

    // The mix has to be audible, and it has to still be a mix.
    let loudness = run
        .shell("ffmpeg -hide_banner -i out/final.mp4 -af volumedetect -f null - 2>&1")
        .await;
    let heard = format!(
        "{}{}",
        loudness["stdout"].as_str().unwrap_or_default(),
        loudness["stderr"].as_str().unwrap_or_default()
    );
    let mean: f64 = heard
        .lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .and_then(|v| v.trim().split(' ').next().unwrap_or("").parse().ok())
        .expect("volumedetect should report a mean volume");
    println!("[validate] mean volume {} dB", mean);
    assert!(mean > -50.0, "the final audio is effectively silent ({} dB)", mean);

    // And the captions have to be on the picture. Looked at, not assumed.
    run.shell(&format!(
        "ffmpeg -hide_banner -loglevel error -ss {:.2} -i out/final.mp4 -frames:v 1 -y out/final-frame.png",
        sample_at
    ))
    .await;
    run.require("out/final-frame.png");

    if settings.api_key.trim().is_empty() {
        println!("[validate] no OpenRouter key configured — skipping the visual check");
    } else {
        let seen = vision::see(
            &run.ws,
            &settings.api_key,
            &settings.vision_model,
            &json!({
                "path": "out/final-frame.png",
                "question": "Quote every word of caption text burned onto this frame, exactly as written. \
                             Then judge the overlay: is the footage visible behind and around the caption text, \
                             or does the text sit on an opaque rectangle? \
                             Finish with one final line, exactly one of these two and nothing else: \
                             'VERDICT: TRANSPARENT' or 'VERDICT: OPAQUE_BOX'."
            }),
        )
        .await
        .expect("the real vision path");
        let answer = seen["answer"].as_str().unwrap_or_default().to_string();
        println!("[validate] {}", answer);

        let lowered = answer.to_lowercase();
        let matched = spoken
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .any(|w| lowered.contains(&w.to_lowercase()));
        assert!(
            matched,
            "no caption word from {:?} was legible in the frame. It said: {}",
            spoken, answer
        );
        // A verdict token, not a phrase search: "there is no black box" and
        // "there is a black box" both contain the words.
        assert!(
            answer.contains("VERDICT: TRANSPARENT"),
            "the overlay did not read as transparent over the footage: {}",
            answer
        );
        assert!(
            !answer.contains("VERDICT: OPAQUE_BOX"),
            "the overlay lost its transparency: {}",
            answer
        );
    }

    run.advance(Stage::Completed);
    assert_eq!(run.stage, Stage::Completed);
    println!(
        "[pipeline] final video: {}",
        run.ws.resolve("out/final.mp4").display()
    );
}

/// Word timings → caption cues, the way the captions skill describes: break on
/// a gap or a full stop first, then on length, and keep every word's own start
/// so the animation can land on it.
type Cue = (f64, f64, f64, Vec<(String, f64)>);

fn cues_from(words: &[Value], limit: f64) -> Vec<Cue> {
    const MAX_WORDS: usize = 5;
    const MAX_SECONDS: f64 = 2.8;

    let mut cues: Vec<Cue> = Vec::new();
    let mut current: Vec<(String, f64)> = Vec::new();
    let mut start = 0.0;
    let mut end = 0.0;

    for word in words {
        let text = word
            .get("punctuated_word")
            .or_else(|| word.get("word"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let at = word.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        let until = word.get("end").and_then(Value::as_f64).unwrap_or(at);
        if text.is_empty() || at >= limit {
            continue;
        }

        if current.is_empty() {
            start = at;
        }
        let sentence_ended = text.ends_with('.') || text.ends_with('?') || text.ends_with('!');
        current.push((text, at));
        end = until.min(limit);

        if current.len() >= MAX_WORDS || end - start >= MAX_SECONDS || sentence_ended {
            cues.push((start, end, start, std::mem::take(&mut current)));
        }
    }
    if !current.is_empty() {
        cues.push((start, end, start, current));
    }

    // A cue holds until the next one starts, so there is no flicker between
    // them, and never past the end of the clip.
    for index in 0..cues.len() {
        let next_start = cues.get(index + 1).map(|c| c.0).unwrap_or(limit);
        cues[index].1 = cues[index].1.max(next_start - 0.05).min(limit);
    }
    cues
}

/// The transparent caption composition: the same contract the `hyperframes`
/// skill sets out, built from real timings.
fn composition(cues: &[Cue], duration: f64) -> String {
    let data: Vec<Value> = cues
        .iter()
        .map(|(start, end, _, words)| {
            json!({
                "start": start,
                "end": end,
                "words": words.iter().map(|(text, at)| json!({ "text": text, "start": at }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width={width}, height={height}" />
    <script src="https://cdn.jsdelivr.net/npm/gsap@3.14.2/dist/gsap.min.js"></script>
    <style>
      * {{ margin: 0; padding: 0; box-sizing: border-box; }}
      html, body {{
        width: {width}px;
        height: {height}px;
        overflow: hidden;
        background: transparent;
      }}
      body {{ font-family: "Inter", "Helvetica Neue", Arial, sans-serif; }}
      .cue {{
        position: absolute;
        left: 7%;
        right: 7%;
        bottom: 19%;
        text-align: center;
        font-size: 58px;
        font-weight: 800;
        line-height: 1.2;
        letter-spacing: -0.01em;
        color: #fff;
        text-shadow: 0 4px 18px rgba(0, 0, 0, 0.6);
        -webkit-text-stroke: 3px rgba(0, 0, 0, 0.85);
        paint-order: stroke fill;
      }}
      /* The whole line is present for the whole cue, so it stays centred and
         reads as one caption. The emphasis is what moves. */
      .word {{ display: inline-block; margin: 0 0.14em; }}
      .word.hot {{ color: #ffd34e; }}
    </style>
  </head>
  <body>
    <div id="root" data-composition-id="captions" data-start="0" data-duration="{duration:.3}"
         data-width="{width}" data-height="{height}"></div>
    <script>
      const CUES = {data};
      const root = document.getElementById("root");
      const tl = gsap.timeline({{ paused: true }});
      CUES.forEach((cue, index) => {{
        const line = document.createElement("div");
        line.className = "cue clip";
        line.id = "cue-" + index;
        line.dataset.start = cue.start.toFixed(3);
        line.dataset.duration = Math.max(0.1, cue.end - cue.start).toFixed(3);
        // The line arrives whole, then each word takes the emphasis as it is
        // spoken. Fading words in where they already sit would leave the
        // invisible ones holding space and push the visible word off-centre.
        tl.fromTo(line,
          {{ opacity: 0, yPercent: 12 }},
          {{ opacity: 1, yPercent: 0, duration: 0.18, ease: "power2.out" }},
          cue.start);
        cue.words.forEach((word, w) => {{
          const span = document.createElement("span");
          span.className = "word";
          span.textContent = word.text;
          line.appendChild(span);
          const next = cue.words[w + 1];
          tl.to(span, {{ color: '#ffd34e', scale: 1.06, duration: 0.12 }}, word.start);
          tl.to(span, {{ color: '#ffffff', scale: 1, duration: 0.12 }},
            next ? next.start : cue.end);
        }});
        root.appendChild(line);
      }});
      window.__timelines = window.__timelines || {{}};
      window.__timelines["captions"] = tl;
    </script>
  </body>
</html>
"#,
        width = WIDTH,
        height = HEIGHT,
        duration = duration,
        data = serde_json::to_string(&data).unwrap(),
    )
}

/// Stop, against a real render.
///
/// ```text
/// SIRVIBE_E2E_WORKSPACE=/tmp/sirvibe-cancel cargo test -- --ignored --nocapture real_stop
/// ```
#[tokio::test]
#[ignore]
async fn real_stop_kills_the_render_tree_and_leaves_nothing_behind() {
    let root = PathBuf::from(env_or(
        "SIRVIBE_E2E_CANCEL_WORKSPACE",
        &std::env::temp_dir().join("sirvibe-cancel-e2e").to_string_lossy(),
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("captions")).unwrap();
    let ws = Workspace::open(&root.to_string_lossy()).unwrap();

    // A composition long enough that there is a render to interrupt.
    let cues: Vec<Cue> = (0..12)
        .map(|i| {
            let start = i as f64 * 2.0;
            (
                start,
                start + 1.8,
                start,
                vec![(format!("frame {}", i), start)],
            )
        })
        .collect();
    tools_fs::write(
        &ws,
        &json!({ "path": "captions/index.html", "content": composition(&cues, 24.0) }),
    )
    .unwrap();
    tools_fs::write(&ws, &json!({ "path": "captions/hyperframes.json", "content": "{}\n" })).unwrap();

    let before = browser_processes();
    let jobs = Arc::new(Jobs::new());
    let running = jobs.clone();
    let ws_for_run = ws.clone();
    let command = format!(
        "cd captions && {} render --format webm -o ../out/cancelled.webm --fps 30 -q high",
        hyperframes()
    );

    let started = std::time::Instant::now();
    let render = tokio::spawn(async move {
        let (job, _guard) = running.start("stop-me");
        let sink: OutputSink = Arc::new(|_, _| {});
        run_core(&ws_for_run, &command, 1800, sink, &job, "stop-me")
            .await
            .expect("the runner must return a result")
    });

    // Wait until the render is genuinely under way. Not "npx has been spawned"
    // — the browser it eventually launches is the process that matters, and the
    // one that used to be left behind.
    let mut browsers: Vec<u32> = Vec::new();
    for _ in 0..1200 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        browsers = browser_processes()
            .into_iter()
            .filter(|pid| !before.contains(pid))
            .collect();
        if !browsers.is_empty() {
            break;
        }
    }
    assert!(
        !browsers.is_empty(),
        "the render never got as far as launching a browser, so the hard case went untested"
    );
    // Give it a moment to be genuinely mid-render rather than mid-startup.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let tree = crate::tools_shell::descendants(Some(std::process::id()));
    let later: Vec<u32> = browser_processes()
        .into_iter()
        .filter(|pid| !before.contains(pid) && !browsers.contains(pid))
        .collect();
    browsers.extend(later);
    let named = describe(&tree);
    println!(
        "[stop] render under way after {:.1}s · {} descendant process(es), {} browser process(es)",
        started.elapsed().as_secs_f64(),
        tree.len(),
        browsers.len()
    );
    for (pid, name) in &named {
        println!("[stop]   pid {} · {}", pid, name);
    }
    // The tree is what a single-pid kill would have missed: the launcher, the
    // runtime under it, and the browser under that.
    let names: String = named.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(" ");
    assert!(names.contains("node") || names.contains("npm"), "no runtime in the tree: {}", names);
    assert!(
        names.contains("chrome") || !browsers.is_empty(),
        "no browser to leave behind: {}",
        names
    );

    // Exactly what the Stop button reaches: cancel_tool → jobs.cancel.
    let pressed = std::time::Instant::now();
    assert!(jobs.cancel("stop-me"), "Stop must find the running call");

    let result = tokio::time::timeout(std::time::Duration::from_secs(30), render)
        .await
        .expect("Stop must return control to the UI, not hang it")
        .unwrap();
    let took = pressed.elapsed();
    println!("[stop] returned in {:.2}s · {}", took.as_secs_f64(), result["status"]);

    assert_eq!(result["status"], "cancelled");
    assert_eq!(result["cancelled"], true);
    assert!(
        took.as_secs() < 20,
        "Stop took {:?}, which is the UI hanging, not stopping",
        took
    );
    assert!(!jobs.is_running("stop-me"), "the job must not stay registered");

    // Nothing may be left running: not the tree we knew about, and not a
    // browser that gave itself a session of its own.
    let watched: Vec<u32> = tree.iter().chain(browsers.iter()).copied().collect();
    let mut leftovers = Vec::new();
    for _ in 0..40 {
        leftovers = crate::tools_shell::still_alive(&watched);
        let strays: Vec<u32> = browser_processes()
            .into_iter()
            .filter(|pid| !before.contains(pid))
            .collect();
        let fresh: Vec<u32> = strays
            .into_iter()
            .filter(|p| !leftovers.contains(p))
            .collect();
        leftovers.extend(fresh);
        if leftovers.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert!(
        leftovers.is_empty(),
        "orphan processes survived the stop: {:?}",
        leftovers
    );
    println!(
        "[stop] all {} process(es) gone, no orphan browser left behind",
        watched.len()
    );

    // And the app must still be usable afterwards.
    let (job, _guard) = jobs.start("after");
    let sink: OutputSink = Arc::new(|_, _| {});
    let after = run_core(&ws, "echo still working", 60, sink, &job, "after")
        .await
        .unwrap();
    assert_eq!(after["status"], "completed");
    assert_eq!(after["stdout"].as_str().unwrap().trim(), "still working");
    println!("[stop] the next command ran normally");
}

/// The discovery failure that started all of this, against the real service.
///
/// In a real session the agent asked Composio for "followers posts media
/// insights" on a connected Instagram account and was told nothing matched. It
/// concluded — reasonably — that the capability did not exist, and said so. The
/// account has sixteen actions, two of which are exactly what was wanted.
///
/// ```text
/// cargo test -- --ignored --nocapture real_app_discovery
/// ```
#[tokio::test]
#[ignore]
async fn real_app_discovery_never_reports_a_capability_as_missing() {
    use crate::composio::Composio;
    use crate::secrets::SecretStore;

    let config = PathBuf::from(env_or(
        "SIRVIBE_E2E_CONFIG",
        "/home/gift/.config/com.sirvibe.agent",
    ));
    let secrets = SecretStore::new(&config);
    let Ok(client) = Composio::from_secrets(&secrets) else {
        println!("[apps] no Composio key configured — skipping");
        return;
    };
    let app = env_or("SIRVIBE_E2E_APP", "instagram");

    // The query the agent actually wrote, against the real service.
    let searched = client
        .search_tools(&[app.clone()], Some("followers posts media insights"), 10)
        .await
        .expect("the search itself should work");
    println!("[apps] the agent's own words matched {} tool(s)", searched.len());

    // What the app can really do — the fallback the runtime now takes.
    let inventory = client
        .search_tools(&[app.clone()], None, 25)
        .await
        .expect("listing a toolkit should work");
    println!(
        "[apps] {} has {} action(s): {:?}",
        app,
        inventory.len(),
        inventory.iter().map(|t| t.slug.as_str()).take(6).collect::<Vec<_>>()
    );

    assert!(
        !inventory.is_empty(),
        "a connected app with no actions at all would make the fallback pointless"
    );
    assert!(
        inventory.len() >= searched.len(),
        "the full list can never be smaller than a search of it"
    );

    // The point of the whole change: whatever the words were, the capability is
    // reachable. Every action carries a slug and a description to choose from.
    for t in &inventory {
        assert!(!t.slug.is_empty(), "an action with no slug cannot be run");
    }
    let described = inventory.iter().filter(|t| !t.description.is_empty()).count();
    println!("[apps] {}/{} actions carry a description", described, inventory.len());
    assert!(
        described * 2 >= inventory.len(),
        "most actions should describe themselves, or the agent cannot choose between them"
    );

    if searched.is_empty() {
        println!(
            "[apps] confirmed: search said nothing, the inventory says {} — this is exactly the \
             false negative the fallback exists for",
            inventory.len()
        );
    }
}

/// A real reference, watched where it lives.
///
/// Runs the production `analyze_reference` path against a real YouTube link
/// with the user's own key: caption-only scope, a section, the whole editing
/// grammar, and a reference that cannot be watched at all.
///
/// ```text
/// cargo test -- --ignored --nocapture real_reference
/// ```
#[tokio::test]
#[ignore]
async fn real_reference_is_watched_remotely_and_never_downloaded() {
    use crate::reference;

    let settings_path = PathBuf::from(env_or(
        "SIRVIBE_E2E_SETTINGS",
        "/home/gift/.config/com.sirvibe.agent/settings.json",
    ));
    let settings = Settings::load(&settings_path);
    assert!(!settings.api_key.trim().is_empty(), "an OpenRouter key is needed");

    let url = env_or("SIRVIBE_E2E_REFERENCE", "https://youtu.be/aqz-KE-bpKQ");
    let root = PathBuf::from(env_or(
        "SIRVIBE_E2E_REFERENCE_WORKSPACE",
        &std::env::temp_dir().join("sirvibe-reference-e2e").to_string_lossy(),
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let ws = Workspace::open(&root.to_string_lossy()).unwrap();
    let before: Vec<PathBuf> = walk(&ws.root);

    // ---- A: the user asked about captions, so only captions are studied.
    let captions = reference::analyze(
        &ws,
        &settings.api_key,
        &settings.reference_model,
        &json!({
            "url": url,
            "scope": "captions",
            "instruction": "Make my captions look like this"
        }),
    )
    .await
    .expect("a caption reference should be watchable");
    println!(
        "[A captions] model={} confidence={} saved={}",
        captions["model"], captions["confidence"], captions["saved"]
    );
    println!("[A captions] {}", serde_json::to_string_pretty(&captions["analysis"]).unwrap());
    assert_eq!(captions["scope"], "captions");
    let analysis = &captions["analysis"];
    assert!(
        analysis.get("captions").is_some(),
        "a caption analysis must report on captions: {}",
        analysis
    );
    assert!(
        analysis.get("confidence").and_then(Value::as_f64).is_some(),
        "every analysis states how sure it is"
    );
    // Scoped means scoped: the pacing and camera blocks belong to other asks.
    for unrelated in ["pacing", "camera", "transitions"] {
        assert!(
            analysis.get(unrelated).is_none(),
            "a caption question came back with {}: {}",
            unrelated,
            analysis
        );
    }

    // ---- B: only a section of the reference.
    let section = reference::analyze(
        &ws,
        &settings.api_key,
        &settings.reference_model,
        &json!({
            "url": url,
            "scope": "pacing",
            "start_seconds": 30.0,
            "end_seconds": 45.0,
            "instruction": "Match the pacing of this bit"
        }),
    )
    .await
    .expect("a section should be watchable");
    println!(
        "[B section] window={} confidence={}",
        section["window"], section["confidence"]
    );
    println!("[B section] {}", serde_json::to_string_pretty(&section["analysis"]).unwrap());
    assert_eq!(section["window"], "the section from 0:30 to 0:45");
    assert!(section["analysis"].get("pacing").is_some(), "{}", section["analysis"]);

    // ---- C: the whole editing grammar.
    let full = reference::analyze(
        &ws,
        &settings.api_key,
        &settings.reference_model,
        &json!({ "url": url, "scope": "full" }),
    )
    .await
    .expect("a full reference should be watchable");
    println!("[C full] {}", serde_json::to_string_pretty(&full["analysis"]).unwrap());
    let grammar = &full["analysis"];
    let blocks = ["pacing", "captions", "camera", "color"]
        .iter()
        .filter(|b| grammar.get(**b).is_some())
        .count();
    assert!(
        blocks >= 3,
        "a full recipe should cover the editing grammar, got {}: {}",
        blocks,
        grammar
    );

    // ---- E: a reference that cannot be watched fails honestly.
    for unwatchable in [
        "https://www.instagram.com/reel/C2Xm1YQr8kM/",
        "https://cdn.example.com/someones-clip.mp4",
    ] {
        let refused = reference::analyze(
            &ws,
            &settings.api_key,
            &settings.reference_model,
            &json!({ "url": unwatchable, "scope": "captions" }),
        )
        .await
        .expect_err("this cannot be watched and must not be guessed at");
        println!("[E refused] {}", refused);
        assert!(refused.contains("see"), "it should offer the next step: {}", refused);
        assert!(!refused.to_lowercase().contains("yt-dlp"));
        assert!(!refused.to_lowercase().contains("download it"));
    }

    // ---- and nothing was fetched onto the disk.
    let after = walk(&ws.root);
    let added: Vec<&PathBuf> = after.iter().filter(|p| !before.contains(p)).collect();
    println!("[files] the reference work wrote: {:?}", added);
    for path in &added {
        let name = path.to_string_lossy().to_lowercase();
        assert!(
            name.ends_with(".json"),
            "only the analysis should be written, found {}",
            name
        );
        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        assert!(bytes < 200_000, "{} is {} bytes — that is media, not analysis", name, bytes);
    }
    assert!(
        added.iter().any(|p| p.to_string_lossy().contains("references/")),
        "the analysis should be filed in references/"
    );
}

fn walk(root: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

/// What the user is shown after a real video run.
///
/// The artifact list comes from the real scanner, and the filtering comes from
/// the real presentation module the app imports — compiled from the same file
/// and run as it runs, rather than restated here.
///
/// ```text
/// SIRVIBE_E2E_WORKSPACE=~/SirVibe-e2e/pipeline-run \
///   cargo test -- --ignored --nocapture artifact_presentation
/// ```
#[tokio::test]
#[ignore]
async fn artifact_presentation_shows_the_finished_video_and_nothing_else() {
    let root = PathBuf::from(env_or(
        "SIRVIBE_E2E_WORKSPACE",
        &std::env::temp_dir().join("sirvibe-pipeline-e2e").to_string_lossy(),
    ));
    let ws = Workspace::open(&root.to_string_lossy()).expect("run the pipeline test first");

    // Everything the run wrote, from the scanner the app uses.
    let found = crate::artifacts::scan(&ws, 0);
    assert!(!found.is_empty(), "nothing in {}", ws.root.display());
    println!("[artifacts] the run wrote {} file(s):", found.len());
    for a in &found {
        println!("[artifacts]   {} ({})", a.path, a.kind);
    }
    assert!(
        found.iter().any(|a| a.path.ends_with("captions.webm")),
        "the overlay should be on disk — it is the hiding that is being tested"
    );

    // Compile the real presentation module and run it on that list.
    let out = std::env::temp_dir().join("sirvibe-deliverables-check");
    let _ = std::fs::remove_dir_all(&out);
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let compile = std::process::Command::new("npx")
        .current_dir(&project)
        .args([
            "tsc",
            "src/lib/deliverables.ts",
            "--outDir",
            &out.to_string_lossy(),
            "--module",
            "es2022",
            "--target",
            "es2022",
            "--moduleResolution",
            "bundler",
        ])
        .output()
        .expect("tsc");
    assert!(
        compile.status.success(),
        "could not compile the presentation module: {}",
        String::from_utf8_lossy(&compile.stdout)
    );

    let payload = serde_json::to_string(&found).unwrap();
    let script = format!(
        "import {{ deliverables }} from '{}/deliverables.js';         const shown = deliverables({});         console.log(JSON.stringify(shown.map((a) => a.path)));",
        out.to_string_lossy(),
        payload
    );
    let script_path = out.join("check.mjs");
    std::fs::write(&script_path, script).unwrap();
    let ran = std::process::Command::new("node")
        .arg(&script_path)
        .output()
        .expect("node");
    assert!(
        ran.status.success(),
        "the presentation module failed: {}",
        String::from_utf8_lossy(&ran.stderr)
    );

    let shown: Vec<String> = serde_json::from_slice(&ran.stdout).expect("a list of paths");
    println!("[artifacts] the user is shown: {:?}", shown);

    // A run can legitimately produce several finished videos — three Shorts is
    // three deliverables. What must never happen is an intermediate reaching
    // the user, or a finished video being hidden behind one.
    assert!(!shown.is_empty(), "a video was made and none was shown");
    for path in &shown {
        assert!(path.ends_with(".mp4"), "{} is not a finished video: {:?}", path, shown);
        assert!(!path.contains("/work/"), "an intermediate was shown: {:?}", shown);
    }
    assert!(
        shown.iter().any(|p| p.ends_with("final.mp4")),
        "the pipeline's own output should be among them: {:?}",
        shown
    );
    for hidden in ["captions.webm", "final-frame.png", "cut.mp4", ".srt", ".ass"] {
        assert!(
            !shown.iter().any(|p| p.contains(hidden)),
            "{} should have stayed internal: {:?}",
            hidden,
            shown
        );
    }
}

/// What each of these processes actually is, for the record.
fn describe(pids: &[u32]) -> Vec<(u32, String)> {
    pids.iter()
        .filter_map(|pid| {
            let raw = std::fs::read_to_string(format!("/proc/{}/cmdline", pid)).ok()?;
            let name = raw.replace('\0', " ").trim().to_string();
            Some((*pid, name.chars().take(70).collect()))
        })
        .collect()
}

/// Every headless browser on the machine right now, by pid. Used to tell a
/// render's own browser apart from the user's, and to prove none is left.
fn browser_processes() -> Vec<u32> {
    let Ok(out) = std::process::Command::new("ps").args(["-eo", "pid=,args="]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.contains("chrome-headless-shell"))
        .filter_map(|line| line.split_whitespace().next()?.parse::<u32>().ok())
        .collect()
}
