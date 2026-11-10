//! One mailbox message in words, for the frame details.

use ecat_wire::{
    CoeEmergency, CoeHeader, CoeService, EoeHeader, EoeType, FoeHeader, FoeOpcode, MailboxErrorReply, MailboxHeader,
    MailboxType, SdoCommand, SdoHeader, SdoInfoHeader, SoeHeader, VoeHeader, foe_error_text, sdo_abort_text,
    segment_data_len,
};

use super::*;

/// One line and labelled fields for a mailbox message (`body` = the bytes after the
/// mailbox header, `header.length` long).
pub fn describe(h: &MailboxHeader, body: &[u8]) -> (String, Vec<(String, String)>) {
    let mut f: Vec<(String, String)> = Vec::new();
    let mut kv = |k: &str, v: String| f.push((k.to_owned(), v));
    let summary = match h.mailbox_type {
        MailboxType::Coe => match CoeHeader::parse(body) {
            Err(_) => "CoE (truncated)".to_owned(),
            Ok(c) => {
                let rest = &body[CoeHeader::LEN..];
                kv("CoE service", format!("{:?}", c.service));
                match c.service {
                    CoeService::SdoRequest | CoeService::SdoResponse if !rest.is_empty() => {
                        let cmd = SdoCommand::decode(rest[0], c.service == CoeService::SdoRequest);
                        kv("SDO command", format!("0x{:02X} — {}", rest[0], cmd.name()));
                        let segment = matches!(
                            cmd,
                            SdoCommand::UploadSegmentRequest { .. }
                                | SdoCommand::UploadSegmentResponse { .. }
                                | SdoCommand::DownloadSegmentRequest { .. }
                                | SdoCommand::DownloadSegmentResponse { .. }
                        );
                        if segment {
                            let (t, data) = match cmd {
                                SdoCommand::UploadSegmentResponse { toggle, unused, last }
                                | SdoCommand::DownloadSegmentRequest { toggle, unused, last } => {
                                    let n = segment_data_len(h.length, unused);
                                    kv("Segment data", format!("{n} B{}", if last { ", last" } else { "" }));
                                    (toggle, format!(" {n} B{}", if last { " last" } else { "" }))
                                }
                                SdoCommand::UploadSegmentRequest { toggle }
                                | SdoCommand::DownloadSegmentResponse { toggle } => (toggle, String::new()),
                                _ => (false, String::new()),
                            };
                            kv("Toggle", (t as u8).to_string());
                            format!("CoE SDO {} t={}{data}", cmd.short_name(), t as u8)
                        } else if let Ok(s) = SdoHeader::parse(rest) {
                            kv("Index", format!("0x{:04X}:{:02X}", s.index, s.subindex));
                            let obj = format!("0x{:04X}:{:02X}", s.index, s.subindex);
                            match cmd {
                                SdoCommand::Abort => {
                                    let text = sdo_abort_text(s.data).unwrap_or("unknown abort code");
                                    kv("Abort code", format!("0x{:08X} — {text}", s.data));
                                    format!("CoE SDO abort {obj}: {text}")
                                }
                                SdoCommand::InitiateUploadResponse {
                                    expedited: true, size_indicated, unused, ..
                                }
                                | SdoCommand::InitiateDownloadRequest {
                                    expedited: true, size_indicated, unused, ..
                                } => {
                                    let n = if size_indicated { 4 - unused as usize } else { 4 };
                                    let v = &s.data.to_le_bytes()[..n];
                                    let hex: String = v.iter().rev().map(|b| format!("{b:02X}")).collect();
                                    kv("Expedited data", format!("{n} B = 0x{hex}"));
                                    format!("CoE SDO {} {obj} = 0x{hex}", cmd.short_name())
                                }
                                SdoCommand::InitiateUploadResponse { .. }
                                | SdoCommand::InitiateDownloadRequest { .. } => {
                                    kv("Complete size", format!("{} B", s.data));
                                    let here = body.len().saturating_sub(CoeHeader::LEN + SdoHeader::LEN);
                                    kv("Data in this message", format!("{here} B"));
                                    format!("CoE SDO {} {obj} size {} B", cmd.short_name(), s.data)
                                }
                                _ => format!("CoE SDO {} {obj}", cmd.short_name()),
                            }
                        } else {
                            format!("CoE SDO {} (truncated)", cmd.short_name())
                        }
                    }
                    CoeService::Emergency => match CoeEmergency::parse(rest) {
                        Ok(e) => {
                            kv("Error code", format!("0x{:04X}", e.error_code));
                            kv("Error register", format!("0x{:02X}", e.error_register));
                            format!("CoE emergency 0x{:04X} reg 0x{:02X}", e.error_code, e.error_register)
                        }
                        Err(_) => "CoE emergency (truncated)".into(),
                    },
                    CoeService::SdoInformation => match SdoInfoHeader::parse(rest) {
                        Ok(i) => {
                            kv("Op code", format!("{} — {}", i.opcode, i.opcode_name()));
                            kv("Fragments left", i.fragments_left.to_string());
                            format!("CoE SDO info {}", i.opcode_name())
                        }
                        Err(_) => "CoE SDO info (truncated)".into(),
                    },
                    other => format!("CoE {other:?}"),
                }
            }
        },
        MailboxType::Foe => match FoeHeader::parse(body) {
            Err(_) => "FoE (truncated)".into(),
            Ok(x) => {
                let payload = &body[FoeHeader::LEN..];
                kv("Op code", x.opcode.name().into());
                match x.opcode {
                    FoeOpcode::Read | FoeOpcode::Write => {
                        kv("Password", format!("0x{:08X}", x.value));
                        kv("File name", text_of(payload));
                        format!("FoE {} '{}'", x.opcode.name(), text_of(payload))
                    }
                    FoeOpcode::Data => {
                        kv("Packet", x.value.to_string());
                        kv("Data", format!("{} B", payload.len()));
                        format!("FoE DATA #{} {} B", x.value, payload.len())
                    }
                    FoeOpcode::Ack => {
                        kv("Packet", x.value.to_string());
                        format!("FoE ACK #{}", x.value)
                    }
                    FoeOpcode::Error => {
                        let t = foe_error_text(x.value).map(str::to_owned).unwrap_or_else(|| text_of(payload));
                        kv("Error", format!("0x{:08X} — {t}", x.value));
                        format!("FoE ERR 0x{:X} {t}", x.value)
                    }
                    FoeOpcode::Busy => {
                        let (d, e) = x.busy_progress();
                        kv("Progress", format!("{d}/{e}"));
                        format!("FoE BUSY {d}/{e}")
                    }
                    FoeOpcode::Reserved(c) => format!("FoE op {c}"),
                }
            }
        },
        MailboxType::Eoe => match EoeHeader::parse(body) {
            Err(_) => "EoE (truncated)".into(),
            Ok(e) => {
                kv("Type", e.frame_type.name().into());
                kv("Port", e.port.to_string());
                if e.frame_type == EoeType::Fragment {
                    let n = body.len() - EoeHeader::LEN;
                    kv("Frame number", e.frame_no.to_string());
                    kv("Fragment", e.fragment.to_string());
                    if e.fragment == 0 {
                        kv("Frame size", format!("≤ {} B", e.offset_blocks as usize * EoeHeader::BLOCK));
                    } else {
                        kv("Offset", format!("{} B", e.offset_blocks as usize * EoeHeader::BLOCK));
                    }
                    kv("Fragment data", format!("{n} B"));
                    kv("Last fragment", e.last_fragment.to_string());
                    format!(
                        "EoE frame {} fragment {} {n} B{}",
                        e.frame_no,
                        e.fragment,
                        if e.last_fragment { " last" } else { "" }
                    )
                } else {
                    format!("EoE {}", e.frame_type.name())
                }
            }
        },
        MailboxType::Soe => match SoeHeader::parse(body) {
            Err(_) => "SoE (truncated)".into(),
            Ok(s) => {
                kv("Op code", format!("{} — {}", s.opcode, s.opcode_name()));
                kv("Drive", s.drive.to_string());
                kv("IDN", format!("0x{:04X}", s.idn));
                kv("Error", s.error.to_string());
                format!("SoE {} IDN 0x{:04X}{}", s.opcode_name(), s.idn, if s.error { " error" } else { "" })
            }
        },
        MailboxType::Voe => match VoeHeader::parse(body) {
            Err(_) => "VoE (truncated)".into(),
            Ok(v) => {
                kv("Vendor id", format!("0x{:08X}", v.vendor_id));
                kv("Vendor type", format!("0x{:04X}", v.vendor_type));
                match super::voe::parse(body) {
                    Some((op, seq, tail)) => {
                        let name = super::voe::op_name(op);
                        kv("FlowLab VoE", format!("{name}, sequence {seq}"));
                        if op == 2 {
                            format!("FlowLab VoE DATA {seq} ({} B)", tail.len())
                        } else {
                            format!("FlowLab VoE {name} {seq}")
                        }
                    }
                    None => format!("VoE vendor 0x{:08X} type 0x{:04X}", v.vendor_id, v.vendor_type),
                }
            }
        },
        MailboxType::Error => match MailboxErrorReply::parse(body) {
            Ok(e) => {
                kv("Detail", format!("0x{:04X} — {}", e.detail, e.detail_text()));
                format!("Mailbox error: {}", e.detail_text())
            }
            Err(_) => "Mailbox error (truncated)".into(),
        },
        MailboxType::Aoe => "AoE".into(),
        MailboxType::Reserved(c) => format!("mailbox type {c}"),
    };
    (summary, f)
}
