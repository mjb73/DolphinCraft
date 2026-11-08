//! Added feature, asked for: *a video's first frame as its preview*.
//!
//! The person's words: "for video files there is no preview. It should show the
//! first frame."
//!
//! Pictures are decoded in-process by the `image` crate. Video is not something
//! that crate can read, so the frame is taken by the program the machine already
//! has for video — **ffmpeg** — and decoded here. The frame goes out of ffmpeg as
//! a PNG on its standard output, scaled down to the size the view asked for so a
//! 4K film does not put a 24 MB picture through the pipe for a 96 px tile.
//!
//! Nothing is required: without ffmpeg in `PATH` the item simply keeps its kind
//! icon, exactly as it did before this existed, and no error is raised. The
//! archive tools already work that way (they use the machine's `zip`/`tar` and
//! say so when one is missing); this follows the same rule.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The longest edge a frame is scaled down to before it leaves ffmpeg. The views
/// never ask for more than this (2048 is the thumbnail cache's own ceiling).
pub const MAX_PX: u32 = 2048;

/// How long one frame is allowed to take before ffmpeg is given up on. A local
/// file answers in well under a second; this only guards against a damaged file
/// that would otherwise sit on a worker thread for ever.
pub const BUDGET: Duration = Duration::from_secs(30);

/// Where ffmpeg is, if this machine has it.
pub fn ffmpeg() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The arguments that take one frame out of `path` at most `max_px` along its
/// longest edge, as a PNG on standard output.
///
/// `-nostdin` keeps ffmpeg from reading the window's own input, and the scale
/// expression is quoted inside the argument because ffmpeg's filter parser strips
/// the quotes itself — no shell is involved, so nothing else could.
pub fn first_frame_args(path: &Path, max_px: u32) -> Vec<String> {
    let px = max_px.clamp(16, MAX_PX);
    vec![
        "-v".to_string(),
        "error".to_string(),
        "-nostdin".to_string(),
        "-i".to_string(),
        path.to_string_lossy().to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        "-vf".to_string(),
        format!("scale='min({px},iw)':'min({px},ih)':force_original_aspect_ratio=decrease"),
        "-f".to_string(),
        "image2pipe".to_string(),
        "-vcodec".to_string(),
        "png".to_string(),
        "-".to_string(),
    ]
}

/// The video's first frame, decoded, or `None` when ffmpeg is missing, the file
/// cannot be read, or it holds no frame at all.
pub fn first_frame(path: &Path, max_px: u32) -> Option<image::DynamicImage> {
    let bytes = first_frame_png(path, max_px)?;
    image::load_from_memory(&bytes).ok()
}

/// The raw PNG ffmpeg writes, which is also what the checks look at.
pub fn first_frame_png(path: &Path, max_px: u32) -> Option<Vec<u8>> {
    let exe = ffmpeg()?;
    let mut child = Command::new(exe)
        .args(first_frame_args(path, max_px))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // The picture is read on its own thread: a pipe holds only 64 KB, so waiting
    // for ffmpeg to exit before reading would deadlock on anything larger.
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + BUDGET;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader.join().ok()?;
                if status.success() && !bytes.is_empty() {
                    return Some(bytes);
                }
                return None;
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
}
