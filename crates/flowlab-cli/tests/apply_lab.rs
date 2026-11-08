//! `flowlab apply-lab` puts measured lab inputs into a simulator spec that `simulate` runs.

use std::path::{Path, PathBuf};
use std::process::Command;

fn flowlab(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_flowlab")).args(args).output().expect("run flowlab")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flowlab-apply-lab-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn lab_inputs_reach_the_spec_and_the_spec_runs() {
    let dir = scratch("ok");
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/sim/example.json");
    let lab = dir.join("lab.json");
    std::fs::write(
        &lab,
        r#"{ "pdi": { "read_bps": 16000000, "write_bps": 22000000, "access_overhead_ns": 11500 },
             "mailbox_policy": { "kind": "fixed_quota", "accesses_per_cycle": 3 },
             "sources": ["pdi: test values"] }"#,
    )
    .unwrap();
    let out = dir.join("with-lab.json");
    let r = flowlab(&[
        "apply-lab",
        spec.to_str().unwrap(),
        "--lab",
        lab.to_str().unwrap(),
        "--slave",
        "2",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v["slaves"][2]["pdi"]["read_bps"], 16_000_000);
    assert_eq!(v["slaves"][2]["pdi"]["access_overhead_ns"], 11_500);
    assert_eq!(v["mailbox_policy"]["accesses_per_cycle"], 3);
    assert!(v["description"].as_str().unwrap().contains("lab inputs on slave 2: pdi: test values"));

    let r = flowlab(&["simulate", out.to_str().unwrap()]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(String::from_utf8_lossy(&r.stdout).contains("fixed quota: 3 mailbox datagrams per cycle"));

    let r = flowlab(&["apply-lab", spec.to_str().unwrap(), "--lab", lab.to_str().unwrap(), "--slave", "9"]);
    assert!(!r.status.success());
    assert!(String::from_utf8_lossy(&r.stderr).contains("no slave 9"));
    std::fs::remove_dir_all(&dir).ok();
}
