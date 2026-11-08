//! The bus a transfer runs over: configured-address reads and writes, a clock, a cycle hook.

use std::future::Future;

use ecat_wire::{SM_REGISTER_BASE, SM_REGISTER_LEN, SyncManagerConfig};
use serde::{Deserialize, Serialize};

/// What came back from a read: the datagram's data and its working counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// Data as returned (zeros, or whatever the ESC left, when `wkc` is 0).
    pub data: Vec<u8>,
    /// Working counter: 1 when the addressed slave served the access, 0 otherwise.
    pub wkc: u16,
}

/// Why an access has no answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BusError {
    /// The frame did not come back in time. The slave may or may not have processed it.
    #[error("frame lost (no response in time)")]
    Lost,
    /// The interface or the master failed; the transfer cannot continue.
    #[error("{0}")]
    Fatal(String),
}

/// Access to one EtherCAT segment as the transfer engine needs it.
///
/// Every mailbox access is a configured-address read (FPRD) or write (FPWR) of one datagram,
/// as on the wire, so the working counter keeps its ESC meaning: a write into a full
/// "mailbox out" buffer and a read of an empty "mailbox in" buffer return WKC 0.
///
/// Implementations: [`VirtualSlave`](crate::VirtualSlave) (an ESC model, virtual time) and,
/// with the `hardware` feature, EtherCrab on a network interface.
pub trait MailboxBus {
    /// Monotonic time, ns since an origin of the implementation's choosing.
    fn now_ns(&self) -> u64;

    /// Wait until [`now_ns`](Self::now_ns) reaches `t_ns` (returns at once if it has).
    fn sleep_until_ns(&self, t_ns: u64) -> impl Future<Output = ()>;

    /// The cyclic exchange at the start of a master cycle: process data, if the
    /// implementation has any. Returns its working counter, or `None` without process data.
    fn cyclic(&self) -> impl Future<Output = Result<Option<u16>, BusError>>;

    /// Read `len` bytes at `register` of the slave with configured address `station` (FPRD).
    fn fprd(&self, station: u16, register: u16, len: u16) -> impl Future<Output = Result<Reply, BusError>>;

    /// Write `data` at `register` of the slave with configured address `station` (FPWR).
    /// Returns the working counter.
    fn fpwr(&self, station: u16, register: u16, data: &[u8]) -> impl Future<Output = Result<u16, BusError>>;
}

/// Where a slave's mailbox lives: SyncManager 0 (master → slave, "mailbox out") and
/// SyncManager 1 (slave → master, "mailbox in").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxLayout {
    /// Configured station address.
    pub station: u16,
    /// SM0 start address.
    pub out_start: u16,
    /// SM0 length, bytes.
    pub out_len: u16,
    /// SM1 start address.
    pub in_start: u16,
    /// SM1 length, bytes.
    pub in_len: u16,
}

/// Index of the "mailbox out" SyncManager.
pub const SM_MAILBOX_OUT: u8 = 0;
/// Index of the "mailbox in" SyncManager.
pub const SM_MAILBOX_IN: u8 = 1;
/// Activate register bit 1: repeat request (master toggles it).
pub const SM_REPEAT_REQUEST: u8 = 0x02;
/// PDI control register bit 1: repeat acknowledge (the slave toggles it).
pub const SM_REPEAT_ACK: u8 = 0x02;

/// Register address of SyncManager `sm`'s 8-byte block.
pub const fn sm_register(sm: u8) -> u16 {
    SM_REGISTER_BASE + SM_REGISTER_LEN * sm as u16
}

/// Write a whole "mailbox out" buffer (a write that ends before the buffer's last byte does
/// not hand the buffer to the slave). Returns the working counter.
pub async fn write_mailbox<B: MailboxBus>(bus: &B, m: &MailboxLayout, buffer: &[u8]) -> Result<u16, BusError> {
    bus.fpwr(m.station, m.out_start, buffer).await
}

/// Read the whole "mailbox in" buffer (reading its last byte releases it on the ESC).
pub async fn read_mailbox<B: MailboxBus>(bus: &B, m: &MailboxLayout) -> Result<Reply, BusError> {
    bus.fprd(m.station, m.in_start, m.in_len).await
}

/// Read SyncManager `sm`'s register block (start, length, control, status, activate, PDI
/// control). `None` when the slave did not answer (WKC 0) or the data is short.
pub async fn read_sm<B: MailboxBus>(bus: &B, station: u16, sm: u8) -> Result<Option<SyncManagerConfig>, BusError> {
    let r = bus.fprd(station, sm_register(sm), SM_REGISTER_LEN).await?;
    if r.wkc == 0 {
        return Ok(None);
    }
    Ok(SyncManagerConfig::parse(&r.data).ok())
}

/// Read the status register of SyncManager `sm` (one byte at `0x0805 + 8·sm`).
pub async fn read_sm_status<B: MailboxBus>(bus: &B, station: u16, sm: u8) -> Result<Reply, BusError> {
    bus.fprd(station, sm_register(sm) + 5, 1).await
}
