//! Pure VideoCore property-mailbox framebuffer contract (ADR-0113).
//!
//! The MMIO transport stays in the Raspberry Pi BSP. This module owns only the
//! bounded message shape and response validation, so malformed firmware data
//! cannot become a framebuffer mapping by accident.

/// Property-channel request/response marker.
pub const RESPONSE_SUCCESS: u32 = 0x8000_0000;
/// End of a property-tag list.
pub const END_TAG: u32 = 0;
/// Set physical display width/height.
pub const TAG_SET_PHYSICAL_WIDTH: u32 = 0x0004_8003;
pub const TAG_SET_PHYSICAL_HEIGHT: u32 = 0x0004_8004;
/// Set virtual framebuffer width/height.
pub const TAG_SET_VIRTUAL_WIDTH: u32 = 0x0004_8009;
pub const TAG_SET_VIRTUAL_HEIGHT: u32 = 0x0004_800a;
/// Set pixel depth and allocate framebuffer.
pub const TAG_SET_DEPTH: u32 = 0x0004_8005;
pub const TAG_ALLOCATE_BUFFER: u32 = 0x0004_0001;
pub const TAG_GET_PITCH: u32 = 0x0004_0008;

/// Fixed mode requested by the first product composition.
pub const WIDTH: u32 = 1024;
pub const HEIGHT: u32 = 768;
pub const DEPTH: u32 = 16;
pub const BYTES_PER_PIXEL: u32 = 2;
pub const MAX_WORDS: usize = 64;

/// Validated framebuffer metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramebufferInfo {
    pub address: u64,
    pub size: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub depth: u32,
}

/// Why a mailbox response cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BufferTooSmall,
    RequestMalformed,
    FirmwareError,
    MissingTag(u32),
    BadResponse(u32),
    UnsupportedMode,
    UnalignedAddress,
    InvalidSize,
}

/// Build one bounded property request in VideoCore wire format.
pub fn build_request(out: &mut [u32]) -> Result<usize, Error> {
    if out.len() < 32 {
        return Err(Error::BufferTooSmall);
    }
    out.fill(0);
    let mut at = 2;
    for (tag, value) in [
        (TAG_SET_PHYSICAL_WIDTH, WIDTH),
        (TAG_SET_PHYSICAL_HEIGHT, HEIGHT),
        (TAG_SET_VIRTUAL_WIDTH, WIDTH),
        (TAG_SET_VIRTUAL_HEIGHT, HEIGHT),
        (TAG_SET_DEPTH, DEPTH),
    ] {
        out[at..at + 4].copy_from_slice(&[tag, 4, 0, value]);
        at += 4;
    }
    out[at..at + 5].copy_from_slice(&[TAG_ALLOCATE_BUFFER, 8, 0, 4096, 0]);
    at += 5;
    out[at..at + 4].copy_from_slice(&[TAG_GET_PITCH, 4, 0, 0]);
    at += 4;
    out[at] = END_TAG;
    out[0] = ((at + 1) * 4) as u32;
    out[1] = 0;
    Ok(at + 1)
}

fn find_tag(words: &[u32], wanted: u32, require_response: bool) -> Result<(usize, u32), Error> {
    let mut at = 2;
    while at < words.len() {
        let tag = words[at];
        if tag == END_TAG {
            return Err(Error::MissingTag(wanted));
        }
        if at + 2 >= words.len() {
            return Err(Error::RequestMalformed);
        }
        let value_words = (words[at + 1] / 4) as usize;
        if value_words == 0 || at + 3 + value_words > words.len() {
            return Err(Error::RequestMalformed);
        }
        if tag == wanted {
            if require_response && words[at + 2] & RESPONSE_SUCCESS == 0 {
                return Err(Error::BadResponse(tag));
            }
            return Ok((at + 3, words[at + 2]));
        }
        at += 3 + value_words;
    }
    Err(Error::MissingTag(wanted))
}

/// Validate the firmware response and extract the framebuffer descriptor.
pub fn parse_response(words: &[u32]) -> Result<FramebufferInfo, Error> {
    if words.len() < 3 || words[0] as usize > words.len() * 4 || words[1] != RESPONSE_SUCCESS {
        return Err(if words.get(1).copied() == Some(0x8000_0001) {
            Error::FirmwareError
        } else {
            Error::RequestMalformed
        });
    }
    let (width_at, _) = find_tag(words, TAG_SET_PHYSICAL_WIDTH, true)?;
    let (height_at, _) = find_tag(words, TAG_SET_PHYSICAL_HEIGHT, true)?;
    let (depth_at, _) = find_tag(words, TAG_SET_DEPTH, true)?;
    let (buffer_at, _) = find_tag(words, TAG_ALLOCATE_BUFFER, true)?;
    let (pitch_at, _) = find_tag(words, TAG_GET_PITCH, true)?;
    let width = words[width_at];
    let height = words[height_at];
    let depth = words[depth_at];
    let address = words[buffer_at] as u64;
    let size = words[buffer_at + 1];
    let pitch = words[pitch_at];
    if width != WIDTH || height != HEIGHT || depth != DEPTH {
        return Err(Error::UnsupportedMode);
    }
    if address == 0 || !address.is_multiple_of(4096) {
        return Err(Error::UnalignedAddress);
    }
    let minimum = pitch.checked_mul(height).ok_or(Error::InvalidSize)?;
    if pitch < WIDTH * BYTES_PER_PIXEL || size < minimum {
        return Err(Error::InvalidSize);
    }
    Ok(FramebufferInfo {
        address,
        size,
        width,
        height,
        pitch,
        depth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> [u32; MAX_WORDS] {
        let mut words = [0; MAX_WORDS];
        let len = build_request(&mut words).unwrap();
        words[1] = RESPONSE_SUCCESS;
        let mut at = 2;
        while words[at] != END_TAG {
            let tag = words[at];
            let value = at + 3;
            words[at + 2] |= RESPONSE_SUCCESS;
            match tag {
                TAG_ALLOCATE_BUFFER => {
                    words[value] = 0x0100_0000;
                    words[value + 1] = HEIGHT * WIDTH * 2;
                }
                TAG_GET_PITCH => words[value] = WIDTH * 2,
                _ => {}
            }
            at += 3 + (words[at + 1] / 4) as usize;
        }
        words[0] = (len * 4) as u32;
        words
    }

    #[test]
    fn request_is_bounded_and_contains_required_tags() {
        let mut words = [0; MAX_WORDS];
        let len = build_request(&mut words).unwrap();
        assert!(len < MAX_WORDS);
        assert_eq!(words[1], 0);
        assert_eq!(words[len - 1], END_TAG);
        assert!(find_tag(&words[..len], TAG_ALLOCATE_BUFFER, false).is_ok());
    }

    #[test]
    fn valid_response_yields_framebuffer_descriptor() {
        let info = parse_response(&response()).unwrap();
        assert_eq!(
            (info.width, info.height, info.depth),
            (WIDTH, HEIGHT, DEPTH)
        );
        assert_eq!(info.pitch, WIDTH * BYTES_PER_PIXEL);
    }

    #[test]
    fn malformed_response_is_refused() {
        let mut words = response();
        words[1] = 0;
        assert_eq!(parse_response(&words), Err(Error::RequestMalformed));
    }

    #[test]
    fn unaligned_framebuffer_is_refused() {
        let mut words = response();
        let mut at = 2;
        while words[at] != TAG_ALLOCATE_BUFFER {
            at += 3 + (words[at + 1] / 4) as usize;
        }
        words[at + 3] = 0x0100_0001;
        assert_eq!(parse_response(&words), Err(Error::UnalignedAddress));
    }

    #[test]
    fn tag_without_success_bit_is_refused() {
        let mut words = response();
        let mut at = 2;
        while words[at] != TAG_GET_PITCH {
            at += 3 + (words[at + 1] / 4) as usize;
        }
        words[at + 2] = 4;
        assert_eq!(
            parse_response(&words),
            Err(Error::BadResponse(TAG_GET_PITCH))
        );
    }
}
