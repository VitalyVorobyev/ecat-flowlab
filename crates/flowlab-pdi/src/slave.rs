//! ESC access for a slave application that takes the place of the ESC's own one.
//!
//! [`Lan9252`] on its own serves the benchmark: it never touches AL Control or the
//! SyncManagers and writes only process RAM. A slave application has to do more: read AL
//! Control (which acknowledges the master's request) and the SyncManager registers, and write
//! AL Status, AL Status Code and the SyncManagers' PDI Control bytes (bit 0 deactivates a
//! SyncManager from the PDI side; a stack sets it outside PRE-OP, and while it is set the ESC
//! refuses the master's mailbox writes). [`SlaveAccess`] allows exactly that and nothing else: it
//! reads any ESC address below 0x2000, writes AL Status (0x0130), AL Status Code (0x0134), PDI
//! Control (0x0807 + 8·n) and process RAM (the SyncManagers' buffers). It never writes AL
//! Control, the master's part of the SyncManager registers or any other register.
//!
//! Taking an ESC over is only safe when no other application uses it: the vendor's slave
//! application must be stopped first, or both answer the master. [`SlaveAccess::take_over`]
//! checks the SPI link; that nothing else drives the ESC is the caller's to ensure (the CLI
//! demands an explicit flag).

use crate::PdiError;
use crate::lan9252::{Lan9252, LinkInfo, csr_chunk, reg};
use crate::spi::SpiBus;

/// ESC AL Status Code register.
pub const ESC_AL_STATUS_CODE: u16 = 0x0134;

/// A LAN9252 driven by a replacement slave application: it reads any ESC address below 0x2000
/// and writes only AL Status, AL Status Code, the SyncManagers' PDI Control bytes and process RAM
/// (never AL Control or the master's part of the SyncManager registers).
#[derive(Debug)]
pub struct SlaveAccess<S> {
    esc: Lan9252<S>,
    link: LinkInfo,
}

impl<S: SpiBus> SlaveAccess<S> {
    /// Take the ESC over after checking the SPI link (`BYTE_TEST`, `HW_CFG`, `ID_REV` with
    /// `chip_id`). No AL Event Request read follows each access: the application reads it
    /// itself.
    pub fn take_over(esc: Lan9252<S>, chip_id: u16) -> Result<Self, PdiError> {
        let mut esc = esc.with_al_event_read(false);
        let link = esc.check_link(chip_id)?;
        Ok(Self { esc, link })
    }

    /// What the link check read.
    pub fn link(&self) -> LinkInfo {
        self.link
    }

    /// Read `buf.len()` bytes at `address`: registers below 0x1000 through the CSR interface
    /// (in aligned pieces of up to 4 bytes), process RAM through the read FIFO. Reading AL
    /// Control acknowledges the master's request; reading the last byte of a mailbox
    /// SyncManager's buffer frees it.
    pub fn read(&mut self, address: u16, buf: &mut [u8]) -> Result<(), PdiError> {
        check_span(address, buf.len())?;
        if address >= reg::PROCESS_RAM_START {
            return self.esc.pram_read(address, buf);
        }
        let mut a = address;
        let mut done = 0;
        while done < buf.len() {
            let n = csr_chunk(a, buf.len() - done);
            self.esc.csr_read(a, &mut buf[done..done + n])?;
            done += n;
            a += n as u16;
        }
        Ok(())
    }

    /// Write process RAM (0x1000 and up) through the write FIFO. Writing the last byte of a
    /// mailbox SyncManager's buffer hands the message to the master.
    pub fn write_ram(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
        check_span(address, data.len())?;
        if address < reg::PROCESS_RAM_START {
            return Err(PdiError::Refused(format!(
                "write to 0x{address:04X}: a slave application writes registers only through write_al_status"
            )));
        }
        self.esc.pram_write(address, data)
    }

    /// Write AL Status Code (0x0134), then AL Status (0x0130): the state in bits 3:0, bit 4 the
    /// error flag. The code goes first, so a master that sees the flag reads its code.
    pub fn write_al_status(&mut self, status: u16, code: u16) -> Result<(), PdiError> {
        self.esc.csr_write(ESC_AL_STATUS_CODE, &code.to_le_bytes())?;
        self.esc.csr_write(reg::ESC_AL_STATUS, &status.to_le_bytes())
    }

    /// Write the PDI Control byte of SyncManager `sm` (0x0807 + 8·sm): bit 0 deactivates it
    /// from the PDI side.
    pub fn write_pdi_control(&mut self, sm: u8, value: u8) -> Result<(), PdiError> {
        if sm >= 16 {
            return Err(PdiError::Refused(format!("SyncManager {sm}: the ESC has 16 at most")));
        }
        self.esc.csr_write(0x0807 + 8 * u16::from(sm), &[value])
    }

    /// The LAN9252 back.
    pub fn into_inner(self) -> Lan9252<S> {
        self.esc
    }
}

/// Refuse empty accesses, accesses beyond process RAM and accesses that cross from registers
/// into process RAM.
fn check_span(address: u16, len: usize) -> Result<(), PdiError> {
    let start = u32::from(address);
    let end = start + len as u32;
    if len == 0 || end > reg::PROCESS_RAM_END {
        return Err(PdiError::Refused(format!(
            "{len} bytes at 0x{address:04X}: accesses must be non-empty and end by 0x{:04X}",
            reg::PROCESS_RAM_END
        )));
    }
    if start < u32::from(reg::PROCESS_RAM_START) && end > u32::from(reg::PROCESS_RAM_START) {
        return Err(PdiError::Refused(format!(
            "{len} bytes at 0x{address:04X} cross from ESC registers into process RAM (0x1000)"
        )));
    }
    Ok(())
}
