//! Developer commands for the workspace, run through the cargo aliases in `.cargo/config.toml`:
//!
//! ```text
//! cargo ui [--no-open] [--demo] [CAPTURES…] [-- SERVE OPTIONS]   build the UI if needed, serve it
//! cargo ui-dev [--no-open] [--demo] [CAPTURES…]      API server + Vite dev server with hot reload
//! cargo ci                                  the checks CI runs (Rust, UI, docs)
//! cargo site                                build the Pages site into _site/
//! ```
//!
//! The UI opens on the Simulate view, or on the analysis of the captures given. `--demo`
//! generates and loads the synthetic demo capture.
//! Needs Bun for the UI; `ci` also uses mdBook when it is installed.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};
use std::time::SystemTime;

type Result<T = ()> = std::result::Result<T, String>;

const HELP: &str = "\
cargo ui [--no-open] [--demo] [CAPTURES…] [-- SERVE OPTIONS]   build the UI if needed, serve it, open it
cargo ui-dev [--no-open] [--demo] [CAPTURES…]      API server + Vite dev server with hot reload
  --demo   also load a synthetic demo capture (generated once into results/demo.pcapng)
cargo ci                                  the checks CI runs (Rust, UI, docs)
cargo site                                build the Pages site into _site/";

const DEMO_SPEC: &str = "configs/synth/demo.json";
const DEMO_CAPTURE: &str = "results/demo.pcapng";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = args.split_first().map_or(("help", &[][..]), |(c, r)| (c.as_str(), r));
    let result = match cmd {
        "ui" => ui(rest),
        "ui-dev" => ui_dev(rest),
        "ci" => ci(),
        "site" => run(Command::new("bash").arg("scripts/build-site.sh")),
        _ => {
            eprintln!("{HELP}");
            return if cmd == "help" { ExitCode::SUCCESS } else { ExitCode::FAILURE };
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace root (the parent of this crate).
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace").to_path_buf()
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn bun() -> Command {
    let mut c = Command::new("bun");
    c.current_dir(root().join("ui"));
    c
}

fn run(c: &mut Command) -> Result {
    let shown = format!("{c:?}");
    let status = c.current_dir_if_unset().status().map_err(|e| format!("{shown}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{shown} failed ({status})")) }
}

trait CurrentDir {
    fn current_dir_if_unset(&mut self) -> &mut Self;
}

impl CurrentDir for Command {
    fn current_dir_if_unset(&mut self) -> &mut Self {
        if self.get_current_dir().is_none() {
            self.current_dir(root());
        }
        self
    }
}

/// Newest modification time of the files under `path` (one file or a directory tree).
fn newest(path: &Path) -> Option<SystemTime> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_file() {
        return meta.modified().ok();
    }
    std::fs::read_dir(path).ok()?.flatten().filter_map(|e| newest(&e.path())).max()
}

fn ensure_bun_deps() -> Result {
    let ui = root().join("ui");
    let lock = newest(&ui.join("bun.lock")).or_else(|| newest(&ui.join("package.json")));
    let installed = newest(&ui.join("node_modules/.bun-tag"))
        .or_else(|| std::fs::metadata(ui.join("node_modules")).ok().and_then(|m| m.modified().ok()));
    if installed.is_none() || lock > installed {
        eprintln!("xtask: installing UI dependencies (bun install)");
        run(bun().args(["install", "--frozen-lockfile"]))
            .map_err(|e| format!("{e}\nIs Bun installed? https://bun.sh"))?;
    }
    Ok(())
}

/// Build `ui/dist` when it is missing or older than the UI sources.
fn ensure_ui_built() -> Result {
    ensure_bun_deps()?;
    let ui = root().join("ui");
    let built = newest(&ui.join("dist/index.html"));
    let sources = ["src", "index.html", "package.json", "bun.lock", "vite.config.ts", "tsconfig.json"]
        .iter()
        .filter_map(|p| newest(&ui.join(p)))
        .max();
    if built.is_none() || sources > built {
        eprintln!("xtask: building the UI (bun run build)");
        run(bun().args(["run", "build"]))?;
    }
    Ok(())
}

/// Flags `--no-open` and `--demo` taken out of `args`: (open, captures).
fn flags(args: &[String]) -> Result<(bool, Vec<String>)> {
    let open = !args.iter().any(|a| a == "--no-open");
    let demo = args.iter().any(|a| a == "--demo");
    let mut caps: Vec<String> = args.iter().filter(|a| *a != "--no-open" && *a != "--demo").cloned().collect();
    if demo {
        caps.push(demo_capture()?);
    }
    Ok((open, caps))
}

/// The demo capture, generated once.
fn demo_capture() -> Result<String> {
    if !root().join(DEMO_CAPTURE).is_file() {
        eprintln!("xtask: generating the demo capture {DEMO_CAPTURE}");
        run(cargo().args([
            "run",
            "--quiet",
            "--release",
            "-p",
            "flowlab-cli",
            "--",
            "synth",
            DEMO_SPEC,
            "-o",
            DEMO_CAPTURE,
        ]))?;
    }
    Ok(DEMO_CAPTURE.into())
}

fn ui(args: &[String]) -> Result {
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let (caps, extra) = (&args[..split], args.get(split + 1..).unwrap_or_default());
    ensure_ui_built()?;
    let (open, caps) = flags(caps)?;
    let mut c = cargo();
    c.args(["run", "--quiet", "--release", "-p", "flowlab-cli", "--", "serve"]).args(&caps).args(extra);
    if open {
        c.arg("--open");
    }
    run(&mut c)
}

fn ui_dev(args: &[String]) -> Result {
    ensure_bun_deps()?;
    let (open, caps) = flags(args)?;
    // Build the server first so Vite does not proxy to a port that is still compiling.
    run(cargo().args(["build", "--quiet", "-p", "flowlab-cli"]))?;
    let mut server = cargo()
        .args(["run", "--quiet", "-p", "flowlab-cli", "--", "serve", "--ui-dir", "ui/no-dist"])
        .args(&caps)
        .spawn()
        .map_err(|e| format!("server: {e}"))?;
    eprintln!("xtask: API on http://127.0.0.1:7878, UI with hot reload on http://localhost:5173");
    let mut vite_cmd = bun();
    vite_cmd.args(["run", "dev"]);
    if open {
        vite_cmd.arg("--open");
    }
    let mut vite = match vite_cmd.spawn() {
        Ok(v) => v,
        Err(e) => {
            stop(&mut server);
            return Err(format!("vite: {e}"));
        }
    };
    // Whichever exits first (or Ctrl-C, which reaches both) ends the session.
    loop {
        if let Ok(Some(status)) = server.try_wait() {
            stop(&mut vite);
            return if status.success() { Ok(()) } else { Err(format!("server exited ({status})")) };
        }
        if let Ok(Some(status)) = vite.try_wait() {
            stop(&mut server);
            return if status.success() { Ok(()) } else { Err(format!("vite exited ({status})")) };
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

fn stop(c: &mut Child) {
    let _ = c.kill();
    let _ = c.wait();
}

fn ci() -> Result {
    let step = |name: &str| eprintln!("\nxtask ci: {name}");
    step("rustfmt");
    run(cargo().args(["fmt", "--all", "--check"]))?;
    step("clippy");
    run(cargo().args(["clippy", "--all-targets", "--quiet", "--", "-D", "warnings"]))?;
    step("tests");
    run(cargo().args(["test", "--quiet"]))?;
    step("generated UI types are committed");
    run(Command::new("git").args(["diff", "--exit-code", "--stat", "--", "ui/src/api/gen"]))?;
    step("rustdoc");
    run(cargo().args(["doc", "--no-deps", "--workspace", "--quiet"]).env("RUSTDOCFLAGS", "-D warnings"))?;
    step("UI: format, lint, types, tests, build");
    ensure_bun_deps()?;
    for script in ["fmt:check", "lint", "typecheck", "test:run", "build"] {
        run(bun().args(["run", script]))?;
    }
    if Command::new("mdbook").arg("--version").output().is_ok() {
        step("book");
        run(Command::new("mdbook").args(["build", "book"]))?;
    } else {
        eprintln!("xtask ci: mdbook not installed, book skipped");
    }
    eprintln!(
        "\nxtask ci: all checks passed (CI also runs the Wireshark cross-check, cargo-deny and the report freshness check)"
    );
    Ok(())
}
