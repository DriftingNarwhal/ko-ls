//! Shared by the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Scales a wall-clock timeout to how much machine there is.
///
/// # Why these are not constants
///
/// Every deadline in these tests was tuned on a 24-core development box, which
/// makes them an assumption about the machine rather than about the software.
/// The suite runs its tests in parallel and each one spawns two or three
/// daemons that sign, verify and encrypt, so a smaller machine does not run the
/// same work a little slower — it runs several tests' worth of it against a
/// fraction of the cores.
///
/// A four-core CI runner is where that first showed up, as three timeouts in
/// `two_nodes` on Windows and nowhere else, which reads exactly like a
/// platform bug. It is not one: pinning the same suite to two cores on Linux
/// reproduces it. The daemons were making steady progress and simply had less
/// of the machine than the numbers assumed.
///
/// So patience is a function of available parallelism. `KOLS_TEST_PATIENCE`
/// overrides it with a plain multiplier for anybody who wants to be explicit —
/// a loaded laptop looks nothing like an idle one, and `available_parallelism`
/// reports cores rather than idleness.
pub fn patience(base: Duration) -> Duration {
    base * factor()
}

fn factor() -> u32 {
    if let Some(explicit) = std::env::var("KOLS_TEST_PATIENCE")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
    {
        return explicit;
    }

    // Twelve is the width this suite was written on and comfortable at, so it
    // is the numerator rather than a tuning knob: at or above it nothing is
    // scaled, and below it the shortfall is the multiplier. Bounded at eight
    // because past that a hang should be reported as a hang rather than waited
    // out for a quarter of an hour.
    let cores = std::thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get);
    u32::try_from(12_usize.div_ceil(cores))
        .unwrap_or(8)
        .clamp(1, 8)
}

pub struct Home(PathBuf);

impl Home {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-2n-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A `kols serve` that is killed when the test ends, however it ends.
pub struct Daemon {
    child: Child,
    log: PathBuf,
    /// How much of the log previous waits have already consumed.
    ///
    /// See [`Daemon::wait_for`]: without this, waiting for something a daemon
    /// has *already* said returns immediately and the test walks on before the
    /// thing it meant to wait for has happened.
    read: usize,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.log);
    }
}

impl Daemon {
    pub fn output(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Whether this daemon has exited, and with what.
    ///
    /// `wait_for` asks before every sleep, because a daemon that has *died* and
    /// one that is merely slow look identical from the outside — a log that
    /// stopped growing — and waiting the full deadline out to say "it never
    /// printed this" describes the symptom rather than the cause.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Waits for the **next** `needle`, or gives up and shows what did appear.
    ///
    /// # Why this consumes the log rather than searching all of it
    ///
    /// It used to search the whole file, and that made every wait on a needle a
    /// daemon says more than once a coin flip. `a_channel_created_after_a_member_joins_reaches_them`
    /// waits for `"picked up"` after creating one channel and again after
    /// creating a second; the second wait matched the **first** channel's line,
    /// still sitting in the log, and returned in *thirteen microseconds*. The
    /// test then asserted that Bob had a channel Alice had not finished adopting
    /// — sometimes true, sometimes not, depending on which won the race.
    ///
    /// That is the whole of the flake three separate sittings looked for a
    /// distributed-systems cause for. It presented as "the governance entry did
    /// not travel", which is a real failure mode and was not this one.
    ///
    /// So a match consumes everything up to and including itself, and the next
    /// wait starts after it. Sequential steps in a scenario are what every call
    /// site means, and now that is what they get.
    pub fn wait_for(&mut self, needle: &str, within: Duration) -> String {
        // Scaled here rather than at each call site: a deadline is a claim about
        // the machine, and one place to make it is one place to be wrong.
        let within = patience(within);
        let deadline = Instant::now() + within;
        loop {
            let seen = self.output();
            // Only what this daemon has said since the last wait finished.
            let fresh = seen.get(self.read..).unwrap_or("");
            if let Some(at) = fresh.find(needle) {
                self.read += at + needle.len();
                return seen;
            }
            // Read the log *before* checking, so a daemon that printed the
            // needle and then exited is a success rather than a race.
            if let Some(status) = self.exited() {
                panic!(
                    "the daemon exited {status} while waiting for {needle:?}. It said:\n{seen}\n\n\
                     and every other daemon in this run said:\n{}",
                    every_daemon_log()
                );
            }
            assert!(
                Instant::now() < deadline,
                "waited {within:?} for {needle:?} — not counting anything said before the \
                 previous wait — saw:\n{seen}\n\n\
                 and every other daemon in this run said:\n{}",
                every_daemon_log()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Every daemon log this process wrote, for a failure that names one of them.
///
/// A two-node test that fails prints the log of the daemon it was waiting on,
/// which is the half that did not do the thing — and the reason is almost always
/// in the other half. The founder refusing to answer a key request says so on
/// *its* terminal, and a joiner waiting for the answer cannot see it, which is
/// exactly how a stall reads as "nothing happened" from one side.
pub fn every_daemon_log() -> String {
    let mine = format!("-{}.log", std::process::id());
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return "(the temp directory would not open)".to_owned();
    };
    let mut logs: Vec<(String, String)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("kols-2n-") && name.ends_with(&mine))
        })
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_owned();
            Some((name, std::fs::read_to_string(&path).ok()?))
        })
        .collect();
    logs.sort();
    logs.into_iter()
        .map(|(name, body)| format!("---- {name} ----\n{body}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn run(home: &Home, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kols"))
        .arg("--home")
        .arg(home.path())
        .args(args)
        .output()
        .expect("the binary runs")
}

pub fn ok(home: &Home, args: &[&str]) -> String {
    let out = run(home, args);
    assert!(
        out.status.success(),
        "`kols {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn serve(home: &Home, port: u16, peer: Option<&str>) -> Daemon {
    serve_tuned(home, port, peer, None, true, None)
}

pub fn serve_sealing(
    home: &Home,
    port: u16,
    peer: Option<&str>,
    seal_bytes: Option<usize>,
    live: bool,
) -> Daemon {
    serve_tuned(home, port, peer, seal_bytes, live, None)
}

/// `serve`, with an optional segment-seal threshold.
///
/// Sealing at `design/01` §3.1's real 4 MiB target would need a test to write
/// four megabytes of chat to produce a single boundary. The threshold is local
/// publishing tuning rather than a validity rule — a reader accepts whatever
/// boundaries an author chose — so a small one here produces history that is
/// ordinary in every respect except how quickly it reaches the second segment.
pub fn serve_tuned(
    home: &Home,
    port: u16,
    peer: Option<&str>,
    seal_bytes: Option<usize>,
    live: bool,
    live_window_millis: Option<i64>,
) -> Daemon {
    let log = std::env::temp_dir().join(format!("kols-2n-{port}-{}.log", std::process::id()));
    let file = std::fs::File::create(&log).expect("a log file");
    let errors = file.try_clone().expect("a second handle on the log");
    let mut command = Command::new(env!("CARGO_BIN_EXE_kols"));
    command
        .arg("--home")
        .arg(home.path())
        .args(["serve", "--listen"])
        .arg(format!("/ip4/127.0.0.1/tcp/{port}"));
    if let Some(peer) = peer {
        command.args(["--peer", peer]);
    }
    if let Some(bytes) = seal_bytes {
        command.args(["--seal-bytes", &bytes.to_string()]);
    }
    if !live {
        command.arg("--no-live");
    }
    if let Some(window) = live_window_millis {
        command.args(["--live-window-millis", &window.to_string()]);
    }
    let child = command
        .stdout(Stdio::from(file))
        // **stderr goes to the same log as stdout, and used to go nowhere.**
        // A daemon that exits early — a lock it could not take, a store it could
        // not read — says why on stderr and says nothing more on stdout. With
        // stderr discarded, every such exit presented as `wait_for` running out
        // of patience against a log that simply stopped, which reads as a hang.
        // The reason was being thrown away at the point it was produced.
        .stderr(Stdio::from(errors))
        .spawn()
        .expect("serve starts");
    Daemon { child, log, read: 0 }
}

pub fn field(output: &str, prefix: &str) -> String {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no {prefix:?} in:\n{output}"))
        .trim()
        .to_owned()
}
