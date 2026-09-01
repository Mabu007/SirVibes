//! Turning arbitrary bytes from a subprocess into something an agent, a person
//! and a log file can each use.
//!
//! A command's stdout is not a Rust `String` and must never be assumed to be
//! one. It arrives as bytes in chunks that split wherever the kernel felt like
//! splitting them; it carries progress bars drawn with `█`, spinners rewritten
//! with a carriage return, ANSI colour, CJK, emoji, and sometimes it is not
//! text at all. Every one of those has to end in a readable line and none of
//! them may end in a panic.
//!
//! Two products come out of the same pass:
//!
//! - the **raw** log, everything that was said, for debugging;
//! - the **digest**, which is what the model is shown — the same log with runs
//!   of progress lines collapsed into one statement of where the work got to.
//!
//! `RAW PROCESS OUTPUT ≠ AGENT EVENT STREAM` is the rule this module exists to
//! enforce.

use std::borrow::Cow;

/// A single line longer than this is emitted in pieces. A program that writes
/// megabytes without a newline — `cat` on a video, say — must not be able to
/// grow one unbounded `String` in memory.
const MAX_LINE: usize = 8 * 1024;
/// How much of the raw log is kept. Beyond this the middle is dropped, and the
/// result says how much.
const MAX_RAW: usize = 4 * 1024 * 1024;
/// Bar glyphs, in the order of likelihood. Their presence is what marks a line
/// as a drawing of progress rather than a sentence about it.
const BARS: [char; 8] = ['█', '░', '▓', '▒', '▏', '■', '═', '#'];
/// How many bar glyphs in a row make it a progress bar rather than a hash.
const BAR_RUN: usize = 3;

// ---------------------------------------------------------------- splitting

/// Bytes in, whole lines out.
///
/// Lines end at `\n` or at `\r` — a spinner that redraws itself with a
/// carriage return and never emits a newline is still producing lines, and
/// waiting for a `\n` that is never coming would hold the whole run in a
/// buffer. Decoding happens per line, so a character split across two reads is
/// still one character when it comes out.
#[derive(Default)]
pub struct Splitter {
    buffer: Vec<u8>,
    /// A `\n` immediately after a `\r` is the same break, not an empty line.
    swallow_newline: bool,
}

impl Splitter {
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<String>) {
        for &byte in bytes {
            if self.swallow_newline {
                self.swallow_newline = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\n' => out.push(self.take()),
                b'\r' => {
                    self.swallow_newline = true;
                    out.push(self.take());
                }
                _ => {
                    self.buffer.push(byte);
                    if self.buffer.len() >= MAX_LINE {
                        out.push(self.take());
                    }
                }
            }
        }
    }

    /// Whatever is left when the pipe closes: a last line with no terminator.
    pub fn finish(&mut self) -> Option<String> {
        (!self.buffer.is_empty()).then(|| self.take())
    }

    fn take(&mut self) -> String {
        // Lossy, never fallible: binary on stdout is a thing that happens, and
        // it is not a reason to stop reading or to lose what came after it.
        let line = String::from_utf8_lossy(&self.buffer).into_owned();
        self.buffer.clear();
        line
    }
}

// ------------------------------------------------------------------- ANSI

/// Strip terminal control sequences, keeping the text they were decorating.
/// The UI is a web view and the model is a reader; neither has a cursor to
/// move, and an unstripped `ESC[2K` is noise in both.
pub fn strip_ansi(line: &str) -> Cow<'_, str> {
    if !line.contains('\u{1b}') && !line.chars().any(is_stray_control) {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if !is_stray_control(c) {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI: parameters, then a final byte in @..~
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: runs until BEL or ESC \
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            // Anything else is a two-character sequence; both are consumed.
            _ => {}
        }
    }
    Cow::Owned(out)
}

fn is_stray_control(c: char) -> bool {
    c.is_control() && c != '\t'
}

// --------------------------------------------------------------- progress

/// Where a piece of work has got to, read off a line it drew for a human.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// What the line said it was doing, with the bar and the numbers removed.
    pub label: String,
    pub percent: Option<u8>,
    pub done: Option<u64>,
    pub total: Option<u64>,
}

impl Progress {
    /// One line a person can read, which is what the UI and the digest show
    /// instead of four hundred redraws of a bar.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(percent) = self.percent {
            parts.push(format!("{}%", percent));
        }
        match (self.done, self.total) {
            (Some(done), Some(total)) => parts.push(format!("{}/{}", done, total)),
            (Some(done), None) => parts.push(format!("{}", done)),
            _ => {}
        }
        let label = if self.label.is_empty() { "working" } else { &self.label };
        if parts.is_empty() {
            label.to_string()
        } else {
            format!("{} — {}", label, parts.join(" · "))
        }
    }
}

/// Is this line a redraw of progress rather than something that was said once?
///
/// Deliberately narrow. "We shipped 50% faster" is a sentence and must survive
/// into the log; a bar of blocks with a percentage on it is a redraw and must
/// not appear four hundred times.
pub fn as_progress(line: &str) -> Option<Progress> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    // ffmpeg's status line, which it rewrites continuously.
    if trimmed.starts_with("frame=") {
        let mut progress = Progress {
            label: "encoding".into(),
            ..Default::default()
        };
        progress.done = field(trimmed, "frame=").and_then(|v| v.parse().ok());
        return Some(progress);
    }

    let percent = percentage(trimmed);
    let (done, total) = ratio(trimmed);

    // Either it is drawn as a bar, or it counts itself: a percentage together
    // with an `N/M` that makes sense as a count. One of those is a redraw; a
    // sentence that happens to mention 50% is neither.
    let drawn = longest_bar_run(trimmed) >= BAR_RUN;
    let counted = percent.is_some()
        && matches!((done, total), (Some(done), Some(total)) if total > 1 && done <= total);
    if !drawn && !counted {
        return None;
    }

    Some(Progress {
        label: label_of(trimmed),
        percent,
        done,
        total,
    })
}

/// `frame= 207 fps=25` → the value after a key, however it is spaced.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.split(key).nth(1)?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

fn longest_bar_run(line: &str) -> usize {
    let mut best = 0;
    let mut run = 0;
    for c in line.chars() {
        if BARS.contains(&c) {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

fn percentage(line: &str) -> Option<u8> {
    let bytes = line.as_bytes();
    let at = line.find('%')?;
    let mut start = at;
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    (start < at).then(|| line[start..at].parse::<u32>().ok().unwrap_or(0).min(100) as u8)
}

/// `207/375` anywhere in the line.
fn ratio(line: &str) -> (Option<u64>, Option<u64>) {
    let bytes = line.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        if byte != b'/' || index == 0 {
            continue;
        }
        let mut start = index;
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }
        let mut end = index + 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if start < index && end > index + 1 {
            return (
                line[start..index].parse().ok(),
                line[index + 1..end].parse().ok(),
            );
        }
    }
    (None, None)
}

/// The words on the line, once the drawing is taken away.
fn label_of(line: &str) -> String {
    let words: Vec<&str> = line
        .split_whitespace()
        .filter(|word| {
            !word.chars().any(|c| BARS.contains(&c))
                && !word.ends_with('%')
                && !word.chars().all(|c| c.is_ascii_digit() || c == '/' || c == '.')
        })
        .collect();
    words.join(" ")
}

// ---------------------------------------------------------------- capture

/// What one stream of a command said, in the two forms it is needed in.
#[derive(Default)]
pub struct Capture {
    raw: String,
    raw_dropped: usize,
    digest: String,
    lines: usize,
    run: Option<Run>,
    latest: Option<Progress>,
    updates: usize,
}

/// A run of consecutive progress redraws, waiting to be summarised.
struct Run {
    last: Progress,
    count: usize,
}

/// What a line turned out to be, so the caller knows what to do with it.
pub enum Accepted {
    /// Something that was said once. Show it.
    Text,
    /// A redraw. The progress state moved; there is nothing new to print.
    Progress,
}

impl Capture {
    /// Take one already-decoded line. Returns what kind of line it was.
    pub fn accept(&mut self, line: &str) -> Accepted {
        self.lines += 1;
        self.push_raw(line);

        match as_progress(line) {
            Some(mut progress) => {
                // The last redraw of a render is often "100%  Render complete",
                // which has no frame count on it. Carry the counts forward so
                // the finished reading still says how much work was done.
                if let Some(previous) = &self.latest {
                    progress.done = progress.done.or(previous.done);
                    progress.total = progress.total.or(previous.total);
                }
                self.updates += 1;
                match &mut self.run {
                    Some(run) => {
                        run.count += 1;
                        run.last = progress.clone();
                    }
                    None => {
                        self.run = Some(Run {
                            last: progress.clone(),
                            count: 1,
                        })
                    }
                }
                self.latest = Some(progress);
                Accepted::Progress
            }
            None => {
                self.close_run();
                self.digest.push_str(line);
                self.digest.push('\n');
                Accepted::Text
            }
        }
    }

    /// The most recent progress reading, for the live event.
    pub fn latest(&self) -> Option<&Progress> {
        self.latest.as_ref()
    }

    pub fn updates(&self) -> usize {
        self.updates
    }

    /// The raw log and the digest, finished.
    pub fn finish(mut self) -> Captured {
        self.close_run();
        Captured {
            digest: self.digest,
            raw: self.raw,
            raw_dropped: self.raw_dropped,
            lines: self.lines,
            updates: self.updates,
            last: self.latest,
        }
    }

    fn push_raw(&mut self, line: &str) {
        if self.raw.len() + line.len() + 1 > MAX_RAW {
            self.raw_dropped += line.len() + 1;
            return;
        }
        self.raw.push_str(line);
        self.raw.push('\n');
    }

    fn close_run(&mut self) {
        let Some(run) = self.run.take() else { return };
        self.digest.push_str(&format!(
            "[progress] {} · {} update{}\n",
            run.last.summary(),
            run.count,
            if run.count == 1 { "" } else { "s" }
        ));
    }
}

pub struct Captured {
    /// What the model is shown: everything said, with redraws collapsed.
    pub digest: String,
    /// Everything, for the log file.
    pub raw: String,
    pub raw_dropped: usize,
    pub lines: usize,
    pub updates: usize,
    pub last: Option<Progress>,
}

// -------------------------------------------------------------- truncation

/// Cut a string to a byte budget **without ever splitting a character**.
///
/// This is the whole reason this module exists: `&text[..20_000]` panics the
/// moment byte 20 000 lands inside a `█`, and a progress bar makes that a
/// certainty rather than a possibility.
pub fn head(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The same, from the other end.
pub fn tail(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Keep the beginning and the end of a long capture, and say what went.
pub fn clamp(text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let half = max_bytes / 2;
    let front = head(&text, half);
    let back = tail(&text, half);
    let omitted = text.len() - front.len() - back.len();
    format!("{}\n… [{} bytes omitted] …\n{}", front, omitted, back)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(chunks: &[&[u8]]) -> Vec<String> {
        let mut splitter = Splitter::default();
        let mut out = Vec::new();
        for chunk in chunks {
            splitter.push(chunk, &mut out);
        }
        out.extend(splitter.finish());
        out
    }

    #[test]
    fn a_character_split_across_two_reads_is_still_one_character() {
        // '█' is three bytes; the kernel is free to hand them over separately.
        let bar = "█".as_bytes();
        let lines = split(&[&bar[..1], &bar[1..], b"\n"]);
        assert_eq!(lines, vec!["█"]);
    }

    #[test]
    fn carriage_returns_are_line_breaks_not_a_line_that_never_ends() {
        // A spinner that redraws itself and never writes a newline.
        assert_eq!(
            split(&[b"10%\r20%\r30%\r"]),
            vec!["10%", "20%", "30%"],
            "each redraw is its own line"
        );
        // CRLF is one break, not two.
        assert_eq!(split(&[b"one\r\ntwo\r\n"]), vec!["one", "two"]);
    }

    #[test]
    fn output_that_is_not_text_at_all_does_not_stop_the_reader() {
        let lines = split(&[&[0xff, 0xfe, b'a'], b"\n", b"after\n"]);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains('a'));
        assert_eq!(lines[1], "after", "reading must continue past the garbage");
    }

    #[test]
    fn one_endless_line_cannot_grow_without_limit() {
        let huge = vec![b'x'; MAX_LINE * 3 + 17];
        let lines = split(&[&huge]);
        assert!(lines.len() >= 3, "it should be broken up, got {}", lines.len());
        assert!(lines.iter().all(|l| l.len() <= MAX_LINE));
        assert_eq!(lines.iter().map(|l| l.len()).sum::<usize>(), huge.len());
    }

    #[test]
    fn terminal_control_sequences_are_taken_off_the_text() {
        assert_eq!(strip_ansi("\u{1b}[32mgreen\u{1b}[0m"), "green");
        assert_eq!(strip_ansi("\u{1b}[2K\u{1b}[1Gredraw"), "redraw");
        assert_eq!(strip_ansi("\u{1b}]0;a title\u{7}text"), "text");
        assert_eq!(strip_ansi("plain"), "plain");
        // The bar itself is content, not decoration.
        assert_eq!(strip_ansi("\u{1b}[36m███░░\u{1b}[0m 60%"), "███░░ 60%");
    }

    #[test]
    fn a_hyperframes_progress_line_is_read_not_repeated() {
        let line = "  ███████████████░░░░░░░░░  55%  Streaming frame 207/375";
        let progress = as_progress(line).expect("this is a progress redraw");
        assert_eq!(progress.percent, Some(55));
        assert_eq!(progress.done, Some(207));
        assert_eq!(progress.total, Some(375));
        assert_eq!(progress.label, "Streaming frame");
        assert_eq!(progress.summary(), "Streaming frame — 55% · 207/375");
    }

    #[test]
    fn ffmpeg_status_lines_are_progress_too() {
        let progress = as_progress("frame=  207 fps= 25 q=28.0 size=  1024kB time=00:00:08.28")
            .expect("ffmpeg redraws this line continuously");
        assert_eq!(progress.done, Some(207));
        assert_eq!(progress.label, "encoding");
    }

    #[test]
    fn a_final_redraw_without_a_count_keeps_the_count_it_had() {
        let mut capture = Capture::default();
        capture.accept("  ███████░░  70%  Streaming frame 262/375");
        capture.accept("  ██████████  100%  Render complete");
        let last = capture.finish().last.expect("a reading");
        assert_eq!(last.percent, Some(100));
        assert_eq!(last.done, Some(262), "the count carries forward");
        assert_eq!(last.total, Some(375));
        assert_eq!(last.label, "Render complete");
    }

    #[test]
    fn a_counting_line_is_progress_even_before_the_bar_fills_up() {
        // The first frames of a render draw an almost empty bar.
        let progress = as_progress("  ░░░░░  1%  Streaming frame 3/375").expect("still a redraw");
        assert_eq!(progress.done, Some(3));
        // And a counter with no bar at all still counts.
        let counted = as_progress("Downloading 40% (2/5)").expect("a percentage and a count");
        assert_eq!(counted.percent, Some(40));
        assert_eq!(counted.total, Some(5));
    }

    #[test]
    fn a_sentence_about_a_percentage_is_not_progress() {
        assert!(as_progress("Compression improved by 50% on this take").is_none());
        assert!(as_progress("[INFO] rendering started").is_none());
        assert!(as_progress("# heading").is_none(), "a lone hash is not a bar");
        assert!(as_progress("").is_none());
    }

    #[test]
    fn four_hundred_redraws_become_one_line_in_the_digest() {
        let mut capture = Capture::default();
        capture.accept("[INFO] render started");
        for frame in 1..=375 {
            // Shaped exactly like the renderer's own bar: filled blocks padded
            // out with empty ones, so the width never changes.
            let filled = frame * 25 / 375;
            capture.accept(&format!(
                "  {}{}  {}%  Streaming frame {}/375",
                "█".repeat(filled),
                "░".repeat(25 - filled),
                frame * 100 / 375,
                frame
            ));
        }
        capture.accept("Render complete");

        let captured = capture.finish();
        assert_eq!(captured.updates, 375);
        let digest_lines: Vec<&str> = captured.digest.lines().collect();
        assert_eq!(
            digest_lines.len(),
            3,
            "the model should see the start, one progress statement, and the end: {:?}",
            digest_lines
        );
        assert!(digest_lines[1].starts_with("[progress]"), "{}", digest_lines[1]);
        assert!(digest_lines[1].contains("375/375"), "{}", digest_lines[1]);
        assert_eq!(captured.last.as_ref().unwrap().percent, Some(100));
        assert!(digest_lines[1].contains("375 updates"), "{}", digest_lines[1]);
        assert_eq!(digest_lines[2], "Render complete");
        // And nothing was lost: the raw log still has every redraw.
        assert_eq!(captured.raw.lines().count(), 377);
    }

    #[test]
    fn truncation_never_cuts_a_character_in_half() {
        // The exact shape of the crash: a '█' straddling the byte the old code
        // sliced at.
        let text = format!("{}{}", "x".repeat(19_998), "█".repeat(20_000));
        assert!(text.len() > 40_000, "it has to be long enough to be cut at all");
        assert!(!text.is_char_boundary(20_000), "the boundary must be inside a bar");

        let clamped = clamp(text.clone(), 40_000);
        assert!(clamped.contains("bytes omitted"));
        assert!(!clamped.contains('\u{fffd}'), "no character was cut in half");
        assert!(clamped.starts_with("xxx"));
        assert!(clamped.ends_with('█'));

        // And the primitives themselves, at every offset around the character.
        for max in 19_995..20_010 {
            let cut = head(&text, max);
            assert!(cut.len() <= max);
            assert!(text.starts_with(cut));
            let end = tail(&text, max);
            assert!(end.len() <= max);
            assert!(text.ends_with(end));
        }
    }

    #[test]
    fn short_output_is_left_exactly_as_it_was() {
        let text = "café 🎬 ünïcödé 日本語\n".to_string();
        assert_eq!(clamp(text.clone(), 40_000), text);
    }

    #[test]
    fn every_multibyte_width_survives_the_clamp() {
        for filler in ["é", "€", "🎬", "日", "█"] {
            let text = filler.repeat(30_000);
            let clamped = clamp(text, 40_000);
            assert!(
                !clamped.contains('\u{fffd}'),
                "{} was cut in half by the clamp",
                filler
            );
        }
    }
}
