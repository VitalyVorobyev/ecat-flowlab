//! The hardware backend: Linux spidev (`/dev/spidevB.C`). On other systems [`open_spidev`]
//! returns an error; the rest of the crate works everywhere.

use std::path::Path;

use crate::PdiError;
use crate::spi::SpiBus;

/// Open an spidev device at `clock_hz` in SPI `mode` (0–3; the LAN9252 takes 0 or 3), 8 bits
/// per word, MSB first. Chip select is held for all segments of a transaction.
pub fn open_spidev(path: &Path, clock_hz: u32, mode: u8) -> Result<Box<dyn SpiBus>, PdiError> {
    if mode > 3 {
        return Err(PdiError::Options(format!("SPI mode {mode}: must be 0 to 3")));
    }
    if clock_hz == 0 {
        return Err(PdiError::Options("the SPI clock must be positive".into()));
    }
    imp::open(path, clock_hz, mode)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::path::Path;

    use spidev::{SpiModeFlags, Spidev, SpidevOptions, SpidevTransfer};

    use crate::PdiError;
    use crate::spi::{Segment, SpiBus};

    struct SpidevBus {
        dev: Spidev,
        speed_hz: u32,
    }

    impl SpiBus for SpidevBus {
        fn transaction(&mut self, segments: &mut [Segment<'_>]) -> io::Result<()> {
            let mut transfers: Vec<SpidevTransfer<'_, '_>> = segments
                .iter_mut()
                .map(|s| {
                    let mut t = match s {
                        Segment::Write(b) => SpidevTransfer::write(b),
                        Segment::Read(b) => SpidevTransfer::read(b),
                    };
                    t.speed_hz = self.speed_hz;
                    t
                })
                .collect();
            self.dev.transfer_multiple(&mut transfers)
        }
    }

    pub(super) fn open(path: &Path, clock_hz: u32, mode: u8) -> Result<Box<dyn SpiBus>, PdiError> {
        let at = |e: io::Error| PdiError::Spi(io::Error::new(e.kind(), format!("{}: {e}", path.display())));
        let mut dev = Spidev::open(path).map_err(at)?;
        let flags = match mode {
            0 => SpiModeFlags::SPI_MODE_0,
            1 => SpiModeFlags::SPI_MODE_1,
            2 => SpiModeFlags::SPI_MODE_2,
            _ => SpiModeFlags::SPI_MODE_3,
        };
        let options = SpidevOptions::new().bits_per_word(8).max_speed_hz(clock_hz).lsb_first(false).mode(flags).build();
        dev.configure(&options).map_err(at)?;
        Ok(Box::new(SpidevBus { dev, speed_hz: clock_hz }))
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::path::Path;

    use crate::PdiError;
    use crate::spi::SpiBus;

    pub(super) fn open(path: &Path, _clock_hz: u32, _mode: u8) -> Result<Box<dyn SpiBus>, PdiError> {
        Err(PdiError::Unsupported(format!(
            "{}: the hardware backend needs Linux spidev (/dev/spidevB.C); use --sim to try the benchmark here",
            path.display()
        )))
    }
}
