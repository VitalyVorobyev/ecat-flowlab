//! The hardware layer (EtherCrab, scan, run, transfers) end to end against the emulated ESC of
//! `flowlab-slave`, in this process: no frame reaches a network.
//!
//! One test, because EtherCrab's frame storage allows one session per process.

#![cfg(feature = "hardware")]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flowlab_core::{EMULATED_ESC_ASSUMPTION, EMULATED_ESC_NAME};
use flowlab_mailbox::msg::udp_payload;
use flowlab_master::hardware::{Session, SessionOptions};
use flowlab_master::{EoeAddresses, MailboxLayout, Payload, TransferJob, TransferOptions, TransferProtocol};
use flowlab_slave::{EmulatedSlave, SETTING_MAILBOX_BYTES, SlaveConfig};

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[1] ^ seed).collect()
}

fn sha256(d: &[u8]) -> String {
    flowlab_core::sha256_hex(d)
}

fn job(protocol: TransferProtocol, payload: Payload) -> TransferJob {
    // The mailbox is replaced by what the SyncManager registers say.
    let mailbox = MailboxLayout { station: 0x1000, out_start: 0, out_len: 0, in_start: 0, in_len: 0 };
    TransferJob { slave: String::new(), mailbox, protocol, payload, payload_name: "pattern".into() }
}

fn receive(expected: &[u8]) -> Payload {
    Payload::Receive { expected_bytes: Some(expected.len() as u64), expected_sha256: Some(sha256(expected)) }
}

#[test]
fn scan_run_and_transfers_against_the_emulated_slave() {
    let slave = Arc::new(Mutex::new(EmulatedSlave::new(SlaveConfig::default()).unwrap()));
    let origin = Instant::now();
    let s = slave.clone();
    let session = Session::in_process(
        move |frame| s.lock().unwrap().handle(frame, origin.elapsed().as_nanos() as u64),
        &SessionOptions::default(),
    )
    .unwrap();
    let replace = |cfg: SlaveConfig| *slave.lock().unwrap() = EmulatedSlave::new(cfg).unwrap();

    // Scan: identity from the EEPROM, mailbox SyncManagers configured, PRE-OP.
    let r = session.scan().unwrap();
    assert_eq!(r.interface, "in-process");
    assert_eq!(r.slaves.len(), 1);
    let s0 = &r.slaves[0];
    assert_eq!((s0.name.as_str(), s0.station, s0.al_state.as_str()), (EMULATED_ESC_NAME, 0x1000, "PREOP"));
    assert_eq!(s0.mailbox_protocols, ["EoE", "CoE", "FoE", "VoE"]);
    assert_eq!(s0.dc_support, "none");
    let mailboxes: Vec<_> =
        s0.sync_managers.iter().filter(|m| m.mode == "mailbox").map(|m| (m.start, m.length)).collect();
    assert_eq!(mailboxes, [(0x1000, 128), (0x1080, 128)]);
    assert!(r.meta.assumptions.iter().any(|a| a == EMULATED_ESC_ASSUMPTION));

    // Cyclic run: OP, one LRW over 4 output and 4 input bytes, WKC 3 every cycle.
    let r = session.run(Duration::from_millis(1), Duration::from_millis(200), 1000).unwrap();
    assert_eq!((r.slaves, r.process_image_bytes, r.expected_wkc), (1, 8, 3));
    assert_eq!((r.wkc_mismatches, r.lost_frames), (0, 0));
    assert_eq!(r.final_states, ["PREOP"]);
    assert!(r.timing.cycles >= 100, "{} cycles", r.timing.cycles);
    assert!(r.meta.assumptions.iter().any(|a| a == EMULATED_ESC_ASSUMPTION));

    let o = TransferOptions::default();

    // FoE write: byte-identical at the slave.
    let file = pattern(5000, 1);
    let r = session.transfer(0x1000, job(TransferProtocol::foe(), Payload::Send(file.clone())), &o, false).unwrap();
    assert!(r.completed, "FoE write: {:?}", r.error);
    assert_eq!(r.delivered_bytes, 5000);
    assert!(r.meta.assumptions.iter().any(|a| a == EMULATED_ESC_ASSUMPTION));
    assert_eq!(slave.lock().unwrap().log().files, vec![("flowlab.bin".to_string(), file)]);

    // CoE download, in OP (process data every cycle). On the way to OP EtherCrab read the
    // mapping over CoE: 8 requests, the last with counter 1. A first message with counter 1 is
    // dropped as a repeat and sent again after the reply timeout; the start value 0 is not.
    let object = pattern(3000, 2);
    let op = TransferOptions { first_counter: 0, ..o.clone() };
    let r = session.transfer(0x1000, job(TransferProtocol::coe(), Payload::Send(object.clone())), &op, true).unwrap();
    assert!(r.completed, "SDO download: {:?}", r.error);
    assert_eq!(r.resent_messages, 0);
    assert_eq!(slave.lock().unwrap().log().objects, vec![object.clone()]);
    let collide = TransferOptions { first_counter: 1, reply_timeout_ns: 50_000_000, ..o.clone() };
    let r = session.transfer(0x1000, job(TransferProtocol::coe(), Payload::Send(object)), &collide, true).unwrap();
    assert!(r.completed, "SDO download: {:?}", r.error);
    assert_eq!(r.resent_messages, 1);

    // FoE read and SDO upload of the slave's data.
    let data = pattern(4000, 3);
    let mut cfg = SlaveConfig::default();
    cfg.responder.upload = data.clone();
    replace(cfg.clone());
    let r = session.transfer(0x1000, job(TransferProtocol::foe(), receive(&data)), &o, false).unwrap();
    assert!(r.completed, "FoE read: {:?}", r.error);
    assert_eq!(r.verified, Some(true));
    replace(cfg);
    let r = session.transfer(0x1000, job(TransferProtocol::coe(), receive(&data)), &o, false).unwrap();
    assert!(r.completed, "SDO upload: {:?}", r.error);
    assert_eq!(r.verified, Some(true));

    // EoE to the slave: UDP frames reassembled there.
    replace(SlaveConfig::default());
    let payload = pattern(6000, 4);
    let r = session.transfer(0x1000, job(TransferProtocol::eoe(), Payload::Send(payload.clone())), &o, false).unwrap();
    assert!(r.completed, "EoE send: {:?}", r.error);
    let port = EoeAddresses::default().destination_port;
    let got: Vec<u8> =
        slave.lock().unwrap().log().eoe_frames.iter().filter_map(|f| udp_payload(f, port)).flatten().copied().collect();
    assert_eq!(got, payload);

    // EoE from the slave: it starts sending after PRE-OP, once EtherCrab's configuration is
    // done.
    let mut cfg = SlaveConfig { eoe_start_after_ns: 50_000_000, ..SlaveConfig::default() };
    cfg.responder.upload = payload.clone();
    cfg.responder.eoe_send = true;
    replace(cfg);
    let r = session.transfer(0x1000, job(TransferProtocol::eoe(), receive(&payload)), &o, false).unwrap();
    assert!(r.completed, "EoE receive: {:?}", r.error);
    assert_eq!(r.verified, Some(true));

    // FlowLab VoE both ways: the slave's DONE verifies a write, READY announces an upload.
    let mut cfg = SlaveConfig::default();
    cfg.responder.upload = payload.clone();
    replace(cfg);
    let r = session.transfer(0x1000, job(TransferProtocol::voe(), Payload::Send(payload.clone())), &o, false).unwrap();
    assert!(r.completed, "VoE send: {:?}", r.error);
    assert_eq!(r.verified, Some(true));
    assert_eq!(slave.lock().unwrap().log().voe_received, vec![payload.clone()]);
    let unknown_size = Payload::Receive { expected_bytes: None, expected_sha256: Some(sha256(&payload)) };
    let r = session.transfer(0x1000, job(TransferProtocol::voe(), unknown_size), &o, false).unwrap();
    assert!(r.completed, "VoE receive: {:?}", r.error);
    assert_eq!((r.payload_bytes, r.verified), (payload.len() as u64, Some(true)));

    // A mailbox size written over CoE applies at the next INIT: the next start reads it.
    let setting = TransferProtocol::Coe { index: SETTING_MAILBOX_BYTES.0, subindex: SETTING_MAILBOX_BYTES.1 };
    let r = session.transfer(0x1000, job(setting, Payload::Send(512u16.to_le_bytes().to_vec())), &o, false).unwrap();
    assert!(r.completed, "setting: {:?}", r.error);
    let r = session.scan().unwrap();
    let mailboxes: Vec<_> =
        r.slaves[0].sync_managers.iter().filter(|m| m.mode == "mailbox").map(|m| (m.start, m.length)).collect();
    assert_eq!(mailboxes, [(0x1000, 512), (0x1200, 512)]);
    let r = session.transfer(0x1000, job(TransferProtocol::voe(), Payload::Send(payload.clone())), &o, false).unwrap();
    assert!(r.completed, "VoE send in 512-byte mailboxes: {:?}", r.error);
    assert_eq!(r.verified, Some(true));
    assert_eq!(r.messages_to_slave, 1 + 6000u64.div_ceil(flowlab_mailbox::voe::capacity(512) as u64));
}
