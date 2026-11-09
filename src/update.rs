//! *Check for Updates…*: ask GitHub for the newest release, offer it, install it
//! and restart into it.
//!
//! **Nothing here dials out on its own.** The check runs when the person picks
//! the menu row, on a worker thread, so the window never waits on a socket; the
//! reply comes back through a channel that `App::update` polls each frame.
//!
//! The install is the released tarball the project already publishes
//! (`packaging/linux-tarball.sh`: `dolphincraft-<version>-<arch>-linux.tar.gz`
//! with a `.sha256` beside it), checked against that checksum before anything on
//! disk is touched, and only then unpacked and renamed over the running binary.
//! A rename replaces a running executable; writing into it would fail with
//! `ETXTBSY`, which is why the new build is staged beside the old one and moved
//! into place in one step.
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// The repository the release comes from. The person's own, as the About window
/// and the README say.
pub const REPO: &str = "mjb73/DolphinCraft";

/// GitHub asks for a user agent and refuses requests without one.
const UA: &str = concat!("DolphinCraft/", env!("CARGO_PKG_VERSION"));

/// No step may hang the worker forever: a check that never returns would leave
/// the menu row saying *Checking…* for the rest of the session.
const TIMEOUT: Duration = Duration::from_secs(30);

/// One downloadable file of a release.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// What the release says about itself, and which of its files belongs to this
/// machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Without the tag's `v`: `0.1.7`.
    pub version: String,
    /// The release body, as written in `.github/release-notes.md`.
    pub notes: String,
    /// `dolphincraft-0.1.7-x86_64-linux.tar.gz`.
    pub tarball: String,
    pub tarball_url: String,
    /// `…tar.gz.sha256`, when the release has it. A download without it is
    /// refused rather than trusted.
    pub sha_url: Option<String>,
    /// The release page, for the person who would rather read it in a browser.
    pub page: String,
}

/// Where the check asks. `DOLPHINCRAFT_UPDATE_ENDPOINT` points it somewhere else —
/// at a test server, or at a file on disk — without a rebuild; the reply must
/// have GitHub's release shape whatever the address.
pub fn endpoint() -> String {
    match std::env::var("DOLPHINCRAFT_UPDATE_ENDPOINT") {
        Ok(url) if !url.trim().is_empty() => url.trim().to_string(),
        _ => format!("https://api.github.com/repos/{REPO}/releases/latest"),
    }
}

/// The architecture the release assets are named for: `x86_64` or `aarch64`.
pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

/// `v0.1.7` and `0.1.7` both read as `[0, 1, 7]`; a part that is not a number
/// reads as 0, so a tag with a suffix (`0.2.0-beta`) still compares on its
/// numbers rather than crashing.
pub(crate) fn numbers(v: &str) -> Vec<u32> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .unwrap_or(0)
        })
        .collect()
}

/// Is `latest` a later version than `current`? Numeric, so `0.1.10` beats
/// `0.1.9` — string order would say the opposite.
pub fn newer(current: &str, latest: &str) -> bool {
    let mut a = numbers(current);
    let mut b = numbers(latest);
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    b > a
}

/// The first paragraph of the release notes, for the one-line toast.
pub fn summary(notes: &str) -> String {
    notes
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("")
        .to_string()
}

/// Read a GitHub release reply into the release this machine can install.
///
/// Pure, so the shape is checked against a fixture without a socket. A release
/// with no build for this architecture is an answer, not a crash: the error says
/// which architecture was missing.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("the reply was not a release: {e}"))?;
    let tag = v["tag_name"]
        .as_str()
        .ok_or("the reply has no release tag")?
        .to_string();
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if version.is_empty() {
        return Err("the release tag is empty".into());
    }
    let notes = v["body"].as_str().unwrap_or("").to_string();
    let page = v["html_url"].as_str().unwrap_or("").to_string();
    let want = format!("dolphincraft-{version}-{}-linux.tar.gz", arch());
    let assets: Vec<Asset> = v["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Asset {
                        name: a["name"].as_str()?.to_string(),
                        url: a["browser_download_url"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let tarball = assets
        .iter()
        .find(|a| a.name == want)
        .ok_or_else(|| format!("release {tag} has no build for {} ({want})", arch()))?;
    let sha = format!("{}.sha256", tarball.name);
    let sha_url = assets.iter().find(|a| a.name == sha).map(|a| a.url.clone());
    Ok(Release {
        version,
        notes,
        tarball: tarball.name.clone(),
        tarball_url: tarball.url.clone(),
        sha_url,
        page,
    })
}

/// One request, to bytes. A failure carries the reason in words the toast can
/// show: a status code, a TLS refusal, a timeout.
fn get(url: &str) -> Result<Vec<u8>, String> {
    let res = ureq::get(url)
        .set("User-Agent", UA)
        .set("Accept", "application/vnd.github+json")
        .timeout(TIMEOUT)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("GitHub answered {code}"),
            other => format!("could not reach GitHub ({other})"),
        })?;
    let mut out = Vec::new();
    res.into_reader()
        .read_to_end(&mut out)
        .map_err(|e| format!("the download stopped ({e})"))?;
    Ok(out)
}

/// What a check found.
pub enum Found {
    /// A later release, ready to offer.
    Newer(Box<Release>),
    /// The newest release is the one running.
    Same(String),
}

/// Ask GitHub for the newest release and compare it with the running build.
pub fn check() -> Result<Found, String> {
    let body = get(&endpoint())?;
    let text = String::from_utf8(body).map_err(|_| "the reply was not text".to_string())?;
    let rel = parse_release(&text)?;
    if newer(crate::APP_VERSION, &rel.version) {
        Ok(Found::Newer(Box::new(rel)))
    } else {
        Ok(Found::Same(rel.version))
    }
}

/// The hex digest of some bytes, the form a `.sha256` file carries.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// The digest named by a `.sha256` file: `sha256sum` writes the hex first, then
/// two spaces and the file name.
pub(crate) fn checksum_from_file(text: &str) -> Option<String> {
    let hex = text.split_whitespace().next()?;
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

/// Unpack the released tarball and put its binary where this one runs from.
///
/// `exe` is `current_exe()`: the desktop entry and install.sh both point at a
/// real file, so replacing that file is what "installed" means. The archive is
/// unpacked in a scratch folder, its member copied beside `exe` as
/// `<name>.new`, made executable, and then renamed over `exe` — one atomic step
/// the running process cannot lose, and one a running executable allows.
pub fn install_archive(tar: &[u8], exe: &Path) -> Result<(), String> {
    let work = std::env::temp_dir().join(format!("dolphincraft-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("no scratch folder ({e})"))?;
    let cleanup = |work: &Path| {
        let _ = std::fs::remove_dir_all(work);
    };
    let archive = work.join("release.tar.gz");
    std::fs::write(&archive, tar).map_err(|e| {
        cleanup(&work);
        format!("the download could not be saved ({e})")
    })?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&work)
        .status()
        .map_err(|e| {
            cleanup(&work);
            format!("tar could not be run ({e})")
        })?;
    if !status.success() {
        cleanup(&work);
        return Err("the downloaded archive would not unpack".into());
    }
    // The tarball holds one folder, `dolphincraft-<version>-<arch>-linux/`, and
    // the binary in it is called `dolphincraft`.
    let mut found: Option<PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir(&work) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("dolphincraft");
            if candidate.is_file() {
                found = Some(candidate);
                break;
            }
        }
    }
    let Some(binary) = found else {
        cleanup(&work);
        return Err("the archive holds no dolphincraft binary".into());
    };
    let name = exe
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "dolphincraft".to_string());
    let staged = exe.with_file_name(format!("{name}.new"));
    if let Err(e) = std::fs::copy(&binary, &staged) {
        cleanup(&work);
        return Err(format!("the new build could not be put in place ({e})"));
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755));
    }
    if let Err(e) = std::fs::rename(&staged, exe) {
        let _ = std::fs::remove_file(&staged);
        cleanup(&work);
        return Err(format!("the new build could not replace this one ({e})"));
    }
    cleanup(&work);
    Ok(())
}

/// Download the release, verify it, install it. The caller has already decided
/// the person asked for it.
pub fn download_and_install(rel: &Release) -> Result<PathBuf, String> {
    let sha_url = rel
        .sha_url
        .clone()
        .ok_or("the release has no checksum file, so the download cannot be checked")?;
    let tar = get(&rel.tarball_url)?;
    let sum_text = String::from_utf8(get(&sha_url)?)
        .map_err(|_| "the checksum file was not text".to_string())?;
    let want = checksum_from_file(&sum_text).ok_or("the checksum file is not a sha256 sum")?;
    let got = sha256_hex(&tar);
    if got != want {
        return Err(format!(
            "the download does not match its checksum ({} for {}, expected {})",
            &got[..12],
            rel.tarball,
            &want[..12]
        ));
    }
    let exe = std::env::current_exe().map_err(|e| format!("this build has no path ({e})"))?;
    install_archive(&tar, &exe)?;
    Ok(exe)
}

/// What one page-turn of `App::update` should do about the update.
pub enum Poll {
    /// Nothing finished this frame.
    Idle,
    /// Say this in the toast.
    Say(String),
    /// The new build is in place at this path: save the settings and restart
    /// into it. The path is carried rather than looked up at restart time:
    /// `current_exe()` after the rename answers `/…/dolphincraft (deleted)`,
    /// because the running process's own inode is the one that was replaced.
    Restart(PathBuf),
}

/// Messages from the worker thread.
enum Msg {
    Checked(Result<Found, String>),
    Installed(Result<PathBuf, String>),
}

/// The update state the window holds: whether a job is running, what to offer,
/// and what went wrong.
#[derive(Default)]
pub struct Update {
    pub checking: bool,
    pub installing: bool,
    /// The newer release, once one has been found.
    pub release: Option<Release>,
    /// The newest version seen, so "you are up to date" can name it.
    pub latest: String,
    /// The last failure, shown in its own small window until dismissed.
    pub error: Option<String>,
    rx: Option<Receiver<Msg>>,
}

impl Update {
    /// Start asking GitHub, on a thread. The row says *Checking…* until `poll`
    /// hears back.
    pub fn start_check(&mut self) {
        if self.checking || self.installing {
            return;
        }
        self.checking = true;
        self.error = None;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Checked(check()));
        });
    }

    /// Start downloading and installing the offered release, on a thread.
    pub fn start_install(&mut self) {
        let Some(rel) = self.release.clone() else {
            return;
        };
        if self.checking || self.installing {
            return;
        }
        self.installing = true;
        self.error = None;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Installed(download_and_install(&rel)));
        });
    }

    /// Put the offer away without installing it.
    pub fn dismiss(&mut self) {
        self.release = None;
        self.error = None;
    }

    /// Collect a finished job. Called once a frame; returns at most one action.
    pub fn poll(&mut self) -> Poll {
        let Some(rx) = self.rx.as_ref() else {
            return Poll::Idle;
        };
        let Ok(msg) = rx.try_recv() else {
            return Poll::Idle;
        };
        self.rx = None;
        self.checking = false;
        self.installing = false;
        match msg {
            Msg::Checked(Ok(Found::Newer(rel))) => {
                self.latest = rel.version.clone();
                self.release = Some(*rel);
                Poll::Idle
            }
            Msg::Checked(Ok(Found::Same(latest))) => {
                self.latest = latest.clone();
                Poll::Say(format!(
                    "DolphinCraft {latest} is the latest version — this is {}.",
                    crate::APP_VERSION
                ))
            }
            Msg::Checked(Err(e)) => {
                self.error = Some(e.clone());
                Poll::Say(format!("Could not check for updates: {e}"))
            }
            Msg::Installed(Ok(exe)) => Poll::Restart(exe),
            Msg::Installed(Err(e)) => {
                // The offer is put away as well: the window then shows the reason
                // instead of the same *Install and Restart* button that just failed.
                self.release = None;
                self.error = Some(e.clone());
                Poll::Say(format!("The update did not install: {e}"))
            }
        }
    }

    /// Is a job running (so the menu row can say so and do nothing)?
    pub fn busy(&self) -> bool {
        self.checking || self.installing
    }
}

/// Start the build that is now on disk, with the arguments this one was given,
/// so the window comes back in the same folder with the same flags.
pub fn restart(exe: &Path) -> Result<(), String> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    Command::new(exe)
        .args(&args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("the new build would not start ({e})"))
}
