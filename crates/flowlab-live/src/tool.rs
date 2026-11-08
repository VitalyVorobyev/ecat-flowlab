//! The capture tools FlowLab drives: Wireshark's `dumpcap` and `tcpdump`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::LiveError;

/// Capture filter for EtherCAT: EtherType 0x88A4, and EtherCAT over UDP (port 34980).
pub const ETHERCAT_FILTER: &str = "ether proto 0x88a4 or udp port 34980";

/// A program that captures packets from an interface and writes them to standard output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Tool {
    /// Wireshark's capture engine. Writes pcapng with nanosecond timestamps where the
    /// platform provides them.
    Dumpcap,
    /// tcpdump. Writes classic pcap.
    Tcpdump,
}

impl Tool {
    /// Program name.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Dumpcap => "dumpcap",
            Tool::Tcpdump => "tcpdump",
        }
    }

    /// Places to look besides `PATH`: the Wireshark installation on Windows (under
    /// `%ProgramFiles%`), the app bundle on macOS, the system directories on Linux.
    fn known_paths(self) -> Vec<PathBuf> {
        if cfg!(windows) {
            return match self {
                Tool::Dumpcap => ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
                    .iter()
                    .filter_map(std::env::var_os)
                    .map(|dir| PathBuf::from(dir).join("Wireshark").join("dumpcap.exe"))
                    .chain([PathBuf::from(r"C:\Program Files\Wireshark\dumpcap.exe")])
                    .collect(),
                // tcpdump does not run on Windows.
                Tool::Tcpdump => Vec::new(),
            };
        }
        let paths: &[&str] = match self {
            Tool::Dumpcap => &["/Applications/Wireshark.app/Contents/MacOS/dumpcap", "/usr/bin/dumpcap"],
            Tool::Tcpdump => &["/usr/sbin/tcpdump", "/usr/bin/tcpdump"],
        };
        paths.iter().map(PathBuf::from).collect()
    }

    /// Where the program is, if installed: on `PATH` (with `.exe` on Windows), else in the
    /// Wireshark installation (`%ProgramFiles%\Wireshark` on Windows, the app bundle on macOS)
    /// or a system directory.
    pub fn locate(self) -> Option<PathBuf> {
        let exe = if cfg!(windows) { format!("{}.exe", self.name()) } else { self.name().to_owned() };
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .map(|dir| dir.join(&exe))
            .chain(self.known_paths())
            .find(|p| p.is_file())
    }

    /// Arguments that capture on `interface` and write the packets to standard output.
    pub fn capture_args(self, interface: &str, filter: Option<&str>) -> Vec<OsString> {
        let mut args: Vec<OsString> = match self {
            // -n: pcapng; -q: no packet counts on stderr.
            Tool::Dumpcap => vec!["-i".into(), interface.into(), "-n".into(), "-q".into(), "-w".into(), "-".into()],
            // -U with --immediate-mode: every packet reaches the pipe at once.
            Tool::Tcpdump => vec![
                "-i".into(),
                interface.into(),
                "-U".into(),
                "--immediate-mode".into(),
                "-s".into(),
                "0".into(),
                "--time-stamp-precision".into(),
                "nano".into(),
                "-w".into(),
                "-".into(),
            ],
        };
        if let Some(f) = filter {
            if self == Tool::Dumpcap {
                args.push("-f".into());
            }
            args.push(f.into());
        }
        args
    }
}

/// The capture tool to use: the preferred one, else `dumpcap`, else `tcpdump` (not on
/// Windows, where only `dumpcap` from Wireshark is looked for).
pub fn find_tool(preferred: Option<Tool>) -> Result<(Tool, PathBuf), LiveError> {
    let order = match preferred {
        Some(t) => vec![t],
        None if cfg!(windows) => vec![Tool::Dumpcap],
        None => vec![Tool::Dumpcap, Tool::Tcpdump],
    };
    order
        .iter()
        .find_map(|&t| t.locate().map(|p| (t, p)))
        .ok_or_else(|| LiveError::NoTool(not_found_message(&order, cfg!(windows))))
}

/// What to say when none of `tools` is installed.
fn not_found_message(tools: &[Tool], windows: bool) -> String {
    let names = tools.iter().map(|t| t.name()).collect::<Vec<_>>().join(" or ");
    if windows {
        let tcpdump = if tools.contains(&Tool::Tcpdump) { "tcpdump does not run on Windows; " } else { "" };
        format!(
            "{names} not found; {tcpdump}install Wireshark with Npcap (https://www.wireshark.org), \
             which puts dumpcap.exe in C:\\Program Files\\Wireshark"
        )
    } else {
        format!("{names} not found; install Wireshark (dumpcap) or tcpdump")
    }
}

/// A network interface a capture tool can open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CaptureInterface {
    /// Name to capture on, e.g. `eth1` or `en5`.
    pub name: String,
    /// What the tool says about it, e.g. `Wi-Fi` or `Up, Running`.
    pub description: Option<String>,
}

/// The interfaces `tool` (at `path`) lists with `-D`.
pub fn list_interfaces(tool: Tool, path: &Path) -> Result<Vec<CaptureInterface>, LiveError> {
    let out = Command::new(path).arg("-D").output().map_err(|e| LiveError::Spawn(tool.name().into(), e))?;
    if !out.status.success() {
        return Err(LiveError::Tool(format!(
            "{} -D failed: {}",
            tool.name(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(parse_interface_list(&String::from_utf8_lossy(&out.stdout)))
}

/// Parse `dumpcap -D` (`1. en0 (Wi-Fi)`) or `tcpdump -D` (`1.en0 [Up, Running]`) output.
pub fn parse_interface_list(text: &str) -> Vec<CaptureInterface> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().trim_start_matches(|c: char| c.is_ascii_digit());
            let rest = rest.strip_prefix('.')?.trim();
            let (name, desc) = match rest.split_once(char::is_whitespace) {
                Some((n, d)) => (n, Some(d.trim())),
                None => (rest, None),
            };
            let description = desc
                .map(|d| d.trim_start_matches(['(', '[']).trim_end_matches([')', ']']).trim().to_owned())
                .filter(|d| !d.is_empty());
            (!name.is_empty()).then(|| CaptureInterface { name: name.to_owned(), description })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_lists_of_both_tools_parse() {
        let d = parse_interface_list("1. utun0\n2. en0 (Wi-Fi)\n3. \\Device\\NPF_{AB} (Ethernet 2)\n");
        assert_eq!(d[0], CaptureInterface { name: "utun0".into(), description: None });
        assert_eq!(d[1].description.as_deref(), Some("Wi-Fi"));
        assert_eq!(d[2].name, "\\Device\\NPF_{AB}");
        let t = parse_interface_list("1.en0 [Up, Running, Wireless]\n2.lo0 [Up, Running, Loopback]\n");
        assert_eq!(t[0].name, "en0");
        assert_eq!(t[1].description.as_deref(), Some("Up, Running, Loopback"));
    }

    #[test]
    fn missing_tools_point_to_wireshark_and_npcap_on_windows() {
        let w = not_found_message(&[Tool::Dumpcap], true);
        assert!(w.contains("Npcap") && w.contains(r"C:\Program Files\Wireshark"), "{w}");
        assert!(!w.contains("tcpdump"), "{w}");
        let t = not_found_message(&[Tool::Tcpdump], true);
        assert!(t.contains("tcpdump does not run on Windows"), "{t}");
        let u = not_found_message(&[Tool::Dumpcap, Tool::Tcpdump], false);
        assert_eq!(u, "dumpcap or tcpdump not found; install Wireshark (dumpcap) or tcpdump");
    }

    #[test]
    fn dumpcap_is_looked_for_in_the_wireshark_installation() {
        let known = Tool::Dumpcap.known_paths();
        if cfg!(windows) {
            assert!(known.iter().all(|p| p.ends_with(r"Wireshark\dumpcap.exe")), "{known:?}");
            assert!(Tool::Tcpdump.known_paths().is_empty());
        } else {
            assert!(known.iter().all(|p| p.ends_with("dumpcap")), "{known:?}");
        }
    }

    #[test]
    fn capture_args_write_to_stdout_with_the_filter() {
        let a = Tool::Dumpcap.capture_args("eth1", Some(ETHERCAT_FILTER));
        assert!(a.windows(2).any(|w| w[0] == "-w" && w[1] == "-"));
        assert!(a.windows(2).any(|w| w[0] == "-f" && w[1] == ETHERCAT_FILTER));
        let t = Tool::Tcpdump.capture_args("eth1", Some(ETHERCAT_FILTER));
        assert_eq!(t.last().unwrap(), ETHERCAT_FILTER);
        assert!(t.windows(2).any(|w| w[0] == "-i" && w[1] == "eth1"));
    }
}
