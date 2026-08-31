//! Touch calibration policy: validated records and board activation.

use kernel_core::touch::Calibration;

/// Persist and activate a calibration as one confirmed policy operation.
pub fn commit(calibration: Calibration) -> bool {
    let mut record = [0u8; kernel_core::touch::CALIBRATION_RECORD_LEN];
    if calibration.encode(&mut record).is_err() {
        return false;
    }
    if crate::durable::put(b"touch-cal", &record).is_err() {
        return false;
    }
    crate::bsp::board::touch::set_calibration(calibration)
}
