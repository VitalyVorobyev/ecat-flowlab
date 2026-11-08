//! The EoE-reassembled frames as their own pcapng.

use super::*;

/// Write the Ethernet frames reassembled from EoE as a pcapng, carrying the source
/// capture's provenance. Returns the number of frames written.
pub fn write_eoe_pcapng<W: std::io::Write>(
    a: &MailboxAnalysis,
    provenance: flowlab_core::Provenance,
    source_name: &str,
    out: W,
) -> std::io::Result<usize> {
    let comment = format!(
        "{}{}; derived=EoE reassembly of {source_name}; generator=flowlab {}",
        flowlab_core::Provenance::COMMENT_PREFIX,
        provenance.as_str(),
        env!("CARGO_PKG_VERSION")
    );
    let mut w =
        flowlab_capture::PcapNgWriter::new(out, concat!("flowlab ", env!("CARGO_PKG_VERSION")), &comment, "eoe0")?;
    for (t, bytes) in &a.eoe_packets {
        w.packet(*t, bytes)?;
    }
    w.finish()?;
    Ok(a.eoe_packets.len())
}
