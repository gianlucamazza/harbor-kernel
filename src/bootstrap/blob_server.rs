//! EL1 durable storage service (P2).
//!
//! The service owns the durable backend and exposes only fixed IPC messages.
//! Agents receive a request SEND capability and a separate reply RECV
//! capability; no storage syscall or shared user buffer exists.

use crate::durable;
use crate::ipc::{self, Message};
use crate::sched;
use kernel_core::blob::{self, Request};
use kernel_core::durable::{DecodeError, EncodeError, MAX_PAYLOAD};

/// Request and reply slots installed by `authority::start_blob_service`.
const REQUEST_SLOT: usize = 0;
const REPLY_SLOT: usize = 1;

pub fn run() {
    let (Some(request), Some(reply)) = (sched::my_cap(REQUEST_SLOT), sched::my_cap(REPLY_SLOT))
    else {
        crate::kprintln!("blob: service missing endpoint caps");
        return;
    };
    loop {
        let message = match ipc::recv(request) {
            Ok(message) => message,
            Err(error) => {
                crate::kprintln!("blob: request recv FAILED {error:?}");
                return;
            }
        };
        let response = handle(message);
        if ipc::send(reply, response).is_err() {
            crate::kprintln!("blob: reply send FAILED");
            return;
        }
    }
}

fn handle(message: Message) -> Message {
    let request = match blob::decode(message) {
        Ok(request) => request,
        Err(error) => {
            crate::kprintln!("blob: malformed {error:?}");
            return blob::bad_request();
        }
    };
    match request {
        Request::Put { key, payload } => match durable::put(key.as_slice(), payload.as_slice()) {
            Ok(()) => {
                crate::kprintln!("blob: put ok");
                blob::ok(0)
            }
            Err(error) => {
                crate::kprintln!("blob: put refused {error:?}");
                blob::store_error(put_error_code(error))
            }
        },
        Request::Get { key } => {
            let mut payload = [0u8; MAX_PAYLOAD];
            match durable::get(key.as_slice(), &mut payload) {
                Ok(len) if len <= blob::MAX_WIRE_BYTES => {
                    let packed = blob::Field::pack(&payload[..len]);
                    crate::kprintln!("blob: got");
                    blob::ok(packed)
                }
                Ok(_) => {
                    crate::kprintln!("blob: reply too large");
                    blob::store_error(3)
                }
                Err(DecodeError::BadKey) => {
                    crate::kprintln!("blob: missing");
                    blob::missing()
                }
                Err(error) => {
                    crate::kprintln!("blob: read failed {error:?}");
                    blob::store_error(4)
                }
            }
        }
    }
}

const fn put_error_code(error: EncodeError) -> u64 {
    match error {
        EncodeError::TooLarge => 2,
        EncodeError::BadKey => 1,
    }
}
