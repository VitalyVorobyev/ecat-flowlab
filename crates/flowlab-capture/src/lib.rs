//! Capture files and the EtherCAT trace index built from them.
//!
//! * [`CaptureFile::parse`] reads classic pcap (µs and ns) and pcapng (any `if_tsresol`,
//!   `if_tsoffset`, multiple sections and interfaces) from memory. No libpcap.
//! * [`PcapNgWriter`] writes pcapng with ns timestamps and a section comment (where the
//!   generators put the provenance marker).
//! * [`StreamSplitter`] cuts a capture arriving as a byte stream (live capture) into whole
//!   blocks, so excerpts of it can be parsed like files.
//! * [`Trace::build`] decodes every frame with `ecat-wire` once and keeps a compact
//!   per-frame / per-datagram index that analysis and the server query.

mod file;
mod pcapng_write;
mod stream;
mod trace;

pub use file::{CaptureError, CaptureFile, CaptureFormat, InterfaceInfo, LINKTYPE_ETHERNET, PacketRecord};
pub use pcapng_write::PcapNgWriter;
pub use stream::{Block, BlockKind, StreamSplitter};
pub use trace::{DatagramRecord, Direction, DirectionMode, FrameKind, FrameRecord, Trace};
