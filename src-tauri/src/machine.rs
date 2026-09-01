//! What this computer can actually do, as opposed to what it claims.
//!
//! ffmpeg lists every encoder it was compiled with, whether or not the hardware
//! behind it exists: on the machine this was written on, `ffmpeg -encoders`
//! offers NVIDIA and Quick Sync alongside VAAPI, and only VAAPI can encode a
//! single frame. So the only honest test is to encode one.
//!
//! Deliberately small. It answers three questions — how many cores, how much
//! memory, and is there a hardware encoder that works — and nothing else. A
//! render planner is a post-MVP concern.

use std::sync::OnceLock;
use std::time::Duration;

/// Candidates in the order they are worth having, with the extra arguments each
/// one needs to accept a frame.
const CANDIDATES: &[(&str, &[&str], &[&str])] = &[
    // name, input-side args, filter for the upload
    ("h264_nvenc", &[], &[]),
    ("h264_qsv", &[], &[]),
    (
        "h264_vaapi",
        &["-vaapi_device", "/dev/dri/renderD128"],
        &["-vf", "format=nv12,hwupload"],
    ),
    ("h264_videotoolbox", &[], &[]),
];

/// A probe encode has to be quick or it is not worth doing at startup.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

static HARDWARE_ENCODER: OnceLock<Option<Encoder>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct Encoder {
    pub name: String,
    /// Everything that has to appear before `-i`, if anything.
    pub input_args: Vec<String>,
    /// The filter that gets a frame into the encoder, if it needs one.
    pub filter_args: Vec<String>,
}

impl Encoder {
    /// The flags, in order, ready to be dropped into an ffmpeg command line.
    pub fn command_hint(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.input_args.iter().cloned());
        parts.push("-i <input>".into());
        parts.extend(self.filter_args.iter().cloned());
        parts.push(format!("-c:v {}", self.name));
        parts.join(" ")
    }
}

/// The first hardware encoder that actually encodes a frame on this machine, or
/// none. Probed once; every later caller gets the answer.
pub fn hardware_encoder() -> Option<Encoder> {
    HARDWARE_ENCODER.get_or_init(probe).clone()
}

/// Start the probe off the critical path, so the first thing the user asks for
/// does not wait on four ffmpeg launches.
pub fn warm_up() {
    std::thread::spawn(|| {
        let _ = hardware_encoder();
    });
}

fn probe() -> Option<Encoder> {
    let listed = listed_encoders()?;
    for (name, input_args, filter_args) in CANDIDATES {
        if !listed.contains(&format!(" {} ", name)) {
            continue;
        }
        let encoder = Encoder {
            name: (*name).to_string(),
            input_args: input_args.iter().map(|s| s.to_string()).collect(),
            filter_args: filter_args.iter().map(|s| s.to_string()).collect(),
        };
        if encodes_a_frame(&encoder) {
            return Some(encoder);
        }
    }
    None
}

fn listed_encoders() -> Option<String> {
    let out = run(&["-hide_banner", "-encoders"], PROBE_TIMEOUT)?;
    Some(format!(" {} ", String::from_utf8_lossy(&out).replace('\n', " ")))
}

/// One frame, to nowhere. If this fails the encoder is not usable here,
/// whatever ffmpeg advertises.
fn encodes_a_frame(encoder: &Encoder) -> bool {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    args.extend(encoder.input_args.iter().cloned());
    args.extend(
        [
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=25:duration=1",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args.extend(encoder.filter_args.iter().cloned());
    args.extend(
        ["-c:v", &encoder.name, "-f", "null", "-"]
            .iter()
            .map(|s| s.to_string()),
    );
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&borrowed, PROBE_TIMEOUT).is_some()
}

/// Run ffmpeg and return its output only if it succeeded. A probe that hangs is
/// a probe that failed.
fn run(args: &[&str], timeout: Duration) -> Option<Vec<u8>> {
    let mut child = std::process::Command::new("ffmpeg")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait().ok()? {
            Some(status) => {
                let out = child.wait_with_output().ok()?;
                return status.success().then_some(out.stdout);
            }
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Cores available to this process, which is what a render can actually use.
pub fn cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Total physical memory in gigabytes, where the platform will say.
pub fn memory_gb() -> Option<f32> {
    #[cfg(target_os = "linux")]
    {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: f64 = info
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()?;
        return Some((kb / 1024.0 / 1024.0) as f32);
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// One line for the system prompt: what the machine is, in terms that change
/// what the agent should do.
pub fn summary() -> String {
    let cores = cores();
    let memory = memory_gb()
        .map(|gb| format!("{:.1} GB RAM", gb))
        .unwrap_or_else(|| "unknown memory".into());
    let tight = memory_gb().map(|gb| gb < 8.0).unwrap_or(false) || cores <= 2;
    format!(
        "{} CPU core{}, {}{}",
        cores,
        if cores == 1 { "" } else { "s" },
        memory,
        if tight {
            " — a modest machine: renders are slow, so prefer short pieces, draft quality while \
             iterating, and do not run two renders at once"
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_describes_itself_in_terms_that_matter() {
        let summary = summary();
        assert!(summary.contains("CPU core"), "{}", summary);
        assert!(cores() >= 1);
    }

    #[test]
    fn a_listed_encoder_is_not_taken_at_its_word() {
        // Whatever this machine has, the answer must come from an encode that
        // actually ran — never from the advertised list alone.
        if let Some(encoder) = hardware_encoder() {
            assert!(encodes_a_frame(&encoder), "the probe must be reproducible");
            assert!(encoder.command_hint().contains(&encoder.name));
        }
    }

    #[test]
    fn a_probe_that_cannot_run_is_simply_no_encoder() {
        let nonsense = Encoder {
            name: "definitely_not_an_encoder".into(),
            input_args: Vec::new(),
            filter_args: Vec::new(),
        };
        assert!(!encodes_a_frame(&nonsense));
    }
}
