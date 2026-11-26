//! USB transport for Garmin/Dynastream ANT sticks (USB2 and USB-m, and clones that reuse their
//! IDs) through `nusb`. On Windows the stick needs the WinUSB driver; on Linux the kernel's
//! usb-serial driver is detached automatically.

use std::time::Duration;

use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, Direction, In, Out, TransferError};
use nusb::{Endpoint, MaybeFuture};

use super::{StickOpener, Transport};
use crate::error::{KtError, KtResult};

/// Dynastream Innovations (Garmin).
pub const VENDOR_DYNASTREAM: u16 = 0x0FCF;
/// ANT USB2 Stick (nRF24AP2-USB).
pub const PRODUCT_USB2: u16 = 0x1008;
/// ANT USB-m Stick.
pub const PRODUCT_USB_M: u16 = 0x1009;

/// Whether a USB device is an ANT stick KittyTooth can drive.
pub fn is_ant_stick(vendor: u16, product: u16) -> bool {
    vendor == VENDOR_DYNASTREAM && matches!(product, PRODUCT_USB2 | PRODUCT_USB_M)
}

const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

pub struct UsbOpener;

impl StickOpener for UsbOpener {
    fn open(&self) -> KtResult<(Box<dyn Transport>, String)> {
        let sticks: Vec<_> = nusb::list_devices()
            .wait()
            .map_err(|e| KtError::AntStick(e.to_string()))?
            .filter(|d| is_ant_stick(d.vendor_id(), d.product_id()))
            .collect();
        let mut last = KtError::AntStickMissing;
        // With several sticks plugged in, use the first one that isn't taken.
        for info in sticks {
            match open_stick(&info) {
                Ok(t) => {
                    let name = info.product_string().unwrap_or("ANT USB stick").to_string();
                    return Ok((Box::new(t), name));
                }
                Err(e) => {
                    tracing::debug!(error = %e, pid = info.product_id(), "could not open ANT stick");
                    last = e;
                }
            }
        }
        Err(last)
    }
}

fn open_stick(info: &nusb::DeviceInfo) -> KtResult<UsbTransport> {
    #[cfg(target_os = "windows")]
    if let Some(driver) = info.driver() {
        if !driver.eq_ignore_ascii_case("winusb") {
            return Err(KtError::AntDriver(driver.to_string()));
        }
    }
    let device = info.open().wait().map_err(open_error)?;
    let intf = device.detach_and_claim_interface(0).wait().map_err(open_error)?;
    let (mut ep_in, mut ep_out) = (None, None);
    if let Some(desc) = intf.descriptor() {
        for ep in desc.endpoints() {
            if ep.transfer_type() != TransferType::Bulk {
                continue;
            }
            match ep.direction() {
                Direction::In => ep_in = ep_in.or(Some(ep.address())),
                Direction::Out => ep_out = ep_out.or(Some(ep.address())),
            }
        }
    }
    let ep_in = intf
        .endpoint::<Bulk, In>(ep_in.unwrap_or(0x81))
        .map_err(open_error)?;
    let ep_out = intf
        .endpoint::<Bulk, Out>(ep_out.unwrap_or(0x01))
        .map_err(open_error)?;
    Ok(UsbTransport { ep_in, ep_out })
}

/// Maps a failure to open or claim the stick to what the user can do about it.
fn open_error(e: nusb::Error) -> KtError {
    classify(e.kind(), e.os_error(), e.to_string())
}

fn classify(kind: nusb::ErrorKind, os_error: Option<u32>, detail: String) -> KtError {
    use nusb::ErrorKind;
    match kind {
        ErrorKind::Busy => KtError::AntStickBusy,
        ErrorKind::Disconnected | ErrorKind::NotFound => KtError::AntStickMissing,
        ErrorKind::Unsupported => KtError::AntDriver(String::new()),
        // Windows: ERROR_ACCESS_DENIED from CreateFile means another program holds the stick.
        _ if cfg!(target_os = "windows") && os_error == Some(5) => KtError::AntStickBusy,
        ErrorKind::PermissionDenied if cfg!(target_os = "windows") => KtError::AntStickBusy,
        ErrorKind::PermissionDenied => KtError::AntDriver("permission denied".into()),
        _ => KtError::AntStick(detail),
    }
}

fn transfer_error(e: TransferError) -> KtError {
    match e {
        TransferError::Disconnected => KtError::AntStickMissing,
        TransferError::Cancelled => KtError::AntStick("the stick stopped responding".into()),
        other => KtError::AntStick(other.to_string()),
    }
}

struct UsbTransport {
    ep_in: Endpoint<Bulk, In>,
    ep_out: Endpoint<Bulk, Out>,
}

impl Transport for UsbTransport {
    fn write(&mut self, frame: &[u8]) -> KtResult<()> {
        self.ep_out
            .transfer_blocking(Buffer::from(frame.to_vec()), WRITE_TIMEOUT)
            .status
            .map_err(transfer_error)
    }

    fn read(&mut self, timeout: Duration) -> KtResult<Vec<u8>> {
        if self.ep_in.pending() == 0 {
            let len = self.ep_in.max_packet_size();
            self.ep_in.submit(Buffer::new(len));
        }
        // A timed-out read stays queued and completes on a later call.
        let Some(c) = self.ep_in.wait_next_complete(timeout) else {
            return Ok(Vec::new());
        };
        match c.status {
            Ok(()) => {
                let mut v = c.buffer.into_vec();
                v.truncate(c.actual_len);
                Ok(v)
            }
            Err(TransferError::Stall) => {
                let _ = self.ep_in.clear_halt().wait();
                Ok(Vec::new())
            }
            Err(e) => Err(transfer_error(e)),
        }
    }
}

impl Drop for UsbTransport {
    fn drop(&mut self) {
        self.ep_in.cancel_all();
        while self.ep_in.pending() > 0 {
            if self
                .ep_in
                .wait_next_complete(Duration::from_millis(500))
                .is_none()
            {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_ant_sticks() {
        assert!(is_ant_stick(0x0FCF, 0x1008));
        assert!(is_ant_stick(0x0FCF, 0x1009));
        // USB1 sticks are serial (CP210x) devices, not driven over raw USB.
        assert!(!is_ant_stick(0x0FCF, 0x1004));
        assert!(!is_ant_stick(0x10C4, 0x1008));
    }

    #[test]
    fn open_errors_are_actionable() {
        use nusb::ErrorKind;
        assert_eq!(classify(ErrorKind::Busy, None, "x".into()), KtError::AntStickBusy);
        assert!(matches!(
            classify(ErrorKind::Unsupported, None, "x".into()),
            KtError::AntDriver(_)
        ));
        assert_eq!(
            classify(ErrorKind::Disconnected, None, "x".into()),
            KtError::AntStickMissing
        );
        if cfg!(target_os = "windows") {
            assert_eq!(
                classify(ErrorKind::Other, Some(5), "x".into()),
                KtError::AntStickBusy
            );
        }
        assert_eq!(
            transfer_error(TransferError::Disconnected),
            KtError::AntStickMissing
        );
    }
}
