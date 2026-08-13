//! EL1 network service for the ADR-0104 directional packet protocol.
//!
//! The service owns transport state and the packet-pool state machine. Agents
//! receive only endpoint capabilities plus the explicitly mapped Normal-WB
//! packet pool; no descriptor or MMIO authority is transferred.

use crate::bootstrap::network_runtime;
use crate::ipc;
use crate::sched;
use kernel_core::net::{self, Request};

const TX_REQUEST_SLOT: usize = 0;
const TX_COMPLETE_SLOT: usize = 1;
const RX_RETURN_SLOT: usize = 2;
const RX_AVAILABLE_SLOT: usize = 3;

pub fn run() {
    let (Some(tx_request), Some(tx_complete), Some(rx_return), Some(rx_available)) = (
        sched::my_cap(TX_REQUEST_SLOT),
        sched::my_cap(TX_COMPLETE_SLOT),
        sched::my_cap(RX_RETURN_SLOT),
        sched::my_cap(RX_AVAILABLE_SLOT),
    ) else {
        crate::kprintln!("net: service missing endpoint caps");
        return;
    };
    network_runtime::enable_service();
    crate::kprintln!("net: service up");
    loop {
        let mut worked = false;
        if let Ok(message) = ipc::try_recv(tx_request) {
            worked = true;
            let response = match net::decode(message) {
                Ok(Request::TxSubmit(token)) => match network_runtime::submit_service_tx(token) {
                    Ok(()) => {
                        crate::kprintln!("net: tx accepted slot={} len={}", token.slot, token.len);
                        None
                    }
                    Err(error) => {
                        crate::kprintln!("net: tx refused {error:?}");
                        Some(net::refused(1))
                    }
                },
                Err(error) => {
                    crate::kprintln!("net: malformed {error:?}");
                    Some(net::refused(2))
                }
                Ok(Request::RxReturn(token)) => {
                    if let Err(error) = network_runtime::return_service_rx(token) {
                        crate::kprintln!("net: rx return refused {error:?}");
                    }
                    None
                }
            };
            if let Some(response) = response {
                let _ = ipc::send(tx_complete, response);
            }
        }
        if let Ok(message) = ipc::try_recv(rx_return) {
            worked = true;
            if let Ok(Request::RxReturn(token)) = net::decode(message)
                && let Err(error) = network_runtime::return_service_rx(token)
            {
                crate::kprintln!("net: rx return refused {error:?}");
            }
        }

        network_runtime::poll();
        if let Some(token) = network_runtime::take_tx_complete() {
            worked = true;
            crate::kprintln!("net: tx complete slot={} len={}", token.slot, token.len);
            let _ = ipc::send(tx_complete, net::tx_complete(token));
        }
        if let Some(token) = network_runtime::take_rx_available() {
            worked = true;
            if ipc::send(rx_available, net::rx_available(token)).is_err() {
                let _ = network_runtime::return_service_rx(token);
            }
        }
        if !worked {
            sched::yield_now();
        }
    }
}
