//! The scenarios the Simulate view starts from: the bulk-transfer study's specs
//! (`configs/studies/bulk-transfer/`) and the built-in example.

use serde::Serialize;
use ts_rs::TS;

use crate::SimSpec;

/// A scenario to start from in the UI.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SimPreset {
    /// Stable identifier.
    pub id: String,
    /// What the scenario is, in one line.
    pub title: String,
    /// The scenario.
    pub spec: SimSpec,
}

/// The shipped scenarios: the bulk-transfer study's specs and the built-in example.
pub fn presets() -> Vec<SimPreset> {
    const SHIPPED: [(&str, &str, &str); 3] = [
        (
            "download-10mb",
            "10 MB download over SPI: FoE, then EoE",
            include_str!("../../../configs/studies/bulk-transfer/large-payload.json"),
        ),
        (
            "upload-beside-io",
            "10 MB upload by EoE beside cyclic I/O",
            include_str!("../../../configs/studies/bulk-transfer/mixed-load.json"),
        ),
        (
            "four-protocols",
            "64 KiB by each protocol, 1 ms cycle",
            include_str!("../../../configs/studies/bulk-transfer/smoke.json"),
        ),
    ];
    let mut out: Vec<SimPreset> = SHIPPED
        .iter()
        .map(|(id, title, json)| {
            // Fixed at build time, and checked by the test below.
            let mut spec: SimSpec = serde_json::from_str(json).expect("a shipped preset is a valid SimSpec");
            spec.description = (*title).to_owned();
            SimPreset { id: (*id).to_owned(), title: (*title).to_owned(), spec }
        })
        .collect();
    let mut example = SimSpec::example();
    example.description = "256 kB by each protocol, three slaves".into();
    out.push(SimPreset { id: "example".into(), title: example.description.clone(), spec: example });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_preset_is_a_valid_spec() {
        let p = presets();
        assert_eq!(p.len(), 4);
        for x in &p {
            x.spec.validate().unwrap_or_else(|e| panic!("{}: {e}", x.id));
            assert_eq!(x.spec.description, x.title);
        }
        let parsed: crate::SimSlave = serde_json::from_str(r#"{ "name": "s" }"#).unwrap();
        assert_eq!(parsed, crate::SimSlave::new("s"), "new() and the serde defaults agree");
        let mut ids: Vec<_> = p.iter().map(|x| x.id.as_str()).collect();
        ids.dedup();
        assert_eq!(ids.len(), p.len());
    }
}
