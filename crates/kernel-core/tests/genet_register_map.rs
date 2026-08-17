//! The GENET register map, pinned to the numbers it was derived from.
//!
//! # Why this file exists
//!
//! `crates/kernel-core/src/genet.rs` carries 129 `pub const` register offsets
//! and bit definitions, written as expressions — `UMAC + 0x08`, `1 << 13`,
//! `(1 << 5) | (1 << 6)`. Nothing asserted most of them, and the first full
//! mutation run over the module said so immediately: of the first 28 survivors,
//! **27 were register constants** whose `<<` became `>>` or whose `+` became
//! `-` with every test still passing. A register map is precisely where a
//! wrong number is invisible in review and catastrophic on silicon, and this
//! project spent twenty-five hardware slices on the semantics of these
//! registers.
//!
//! # What the literals are, and why they are not circular
//!
//! Each value is written as a literal rather than re-derived from the
//! definition, so changing a definition fails here and forces whoever changed
//! it back to the datasheet. That is the same shape as the assertions already
//! living beside the code (`assert_eq!(registers::UMAC_TX_POK, 0xcec)`), just
//! complete instead of partial.
//!
//! # The audit these came from
//!
//! Every constant was dumped from the crate and compared by name against the
//! `#define`s of Linux's `bcmgenet.h` — the register contract ADR-0106 cites.
//! **32 have a Linux counterpart and none of them disagree.** Nine appear to at
//! first glance and do not: Linux defines those offsets relative to their
//! block, and Harbor defines them absolute from the GENET base.
//!
//! | Constant | Linux | Harbor | Block base |
//! | --- | --- | --- | --- |
//! | `RBUF_CTRL` | `0x0` | `0x300` | RBUF `0x300` |
//! | `RBUF_CHK_CTRL` | `0x14` | `0x314` | RBUF `0x300` |
//! | `RBUF_TBUF_SIZE_CTRL` | `0xb4` | `0x3b4` | RBUF `0x300` |
//! | `TBUF_CTRL` | `0x0` | `0x600` | TBUF `0x600` |
//! | `EXT_RGMII_OOB_CTRL` | `0xc` | `0x8c` | EXT `0x80` |
//! | `UMAC_MIB_START` | `0x400` | `0xc00` | UMAC `0x800` |
//! | `UMAC_MIB_CTRL` | `0x580` | `0xd80` | UMAC `0x800` |
//! | `UMAC_MDIO_CMD` | `0x614` | `0xe14` | UMAC `0x800` |
//! | `HFB_CTRL` | `0x0` | `0xfc00` | HFB regs `0xfc00` |
//!
//! The other 97 have no Linux counterpart by name: Harbor's own derived values,
//! and the PHY/MDIO/interrupt vocabularies.

#[test]
fn registers_hold_their_documented_values() {
    use kernel_core::genet::registers::*;
    assert_eq!(SYS_REV_CTRL, 0x0);
    assert_eq!(SYS_PORT_CTRL, 0x4);
    assert_eq!(PORT_MODE_EXT_GPHY, 0x3);
    assert_eq!(EXT, 0x80);
    assert_eq!(EXT_RGMII_OOB_CTRL, 0x8c);
    assert_eq!(RGMII_LINK, 0x10);
    assert_eq!(OOB_DISABLE, 0x20);
    assert_eq!(RGMII_MODE_EN, 0x40);
    assert_eq!(ID_MODE_DIS, 0x10000);
    assert_eq!(INTRL2_0, 0x200);
    assert_eq!(INTRL2_1, 0x240);
    assert_eq!(RBUF, 0x300);
    assert_eq!(UMAC, 0x800);
    assert_eq!(MDIO, 0xe14);
    assert_eq!(RDMA, 0x2000);
    assert_eq!(TDMA, 0x4000);
    assert_eq!(INTRL2_CPU_CLEAR, 0x8);
    assert_eq!(INTRL2_CPU_MASK_SET, 0x10);
    assert_eq!(SYS_RBUF_FLUSH_CTRL, 0x8);
    assert_eq!(SYS_UMAC_SW_RESET, 0x2);
    assert_eq!(SYS_RBUF_FLUSH, 0x1);
    assert_eq!(RBUF_CTRL, 0x300);
    assert_eq!(RBUF_64B_EN, 0x1);
    assert_eq!(RBUF_ALIGN_2B, 0x2);
    assert_eq!(RBUF_CHK_CTRL, 0x314);
    assert_eq!(RBUF_RXCHK_EN, 0x1);
    assert_eq!(RBUF_SKIP_FCS, 0x10);
    assert_eq!(RBUF_L3_PARSE_DIS, 0x20);
    assert_eq!(RBUF_TBUF_SIZE_CTRL, 0x3b4);
    assert_eq!(RBUF_TBUF_SIZE, 0x1);
    assert_eq!(UMAC_CMD, 0x808);
    assert_eq!(UMAC_CMD_SW_RESET, 0x2000);
    assert_eq!(UMAC_CMD_TX_EN, 0x1);
    assert_eq!(UMAC_CMD_RX_EN, 0x2);
    assert_eq!(UMAC_CMD_SPEED_SHIFT, 0x2);
    assert_eq!(UMAC_CMD_SPEED_MASK, 0xc);
    assert_eq!(UMAC_CMD_PAD_EN, 0x20);
    assert_eq!(UMAC_CMD_CRC_FWD, 0x40);
    assert_eq!(UMAC_CMD_NO_LEN_CHK, 0x1000000);
    assert_eq!(UMAC_MAC0, 0x80c);
    assert_eq!(UMAC_MAC1, 0x810);
    assert_eq!(UMAC_MAX_FRAME_LEN, 0x814);
    assert_eq!(UMAC_MIB_START, 0xc00);
    assert_eq!(UMAC_TX_PKTS_PACKED, 0xc9c);
    assert_eq!(UMAC_TX_PKTS, 0xca8);
    assert_eq!(UMAC_TX_POK, 0xcec);
    assert_eq!(UMAC_MIB_CTRL, 0xd80);
    assert_eq!(MIB_RESET_RX, 0x1);
    assert_eq!(MIB_RESET_RUNT, 0x2);
    assert_eq!(MIB_RESET_TX, 0x4);
    assert_eq!(UMAC_TX_FLUSH, 0xb34);
    assert_eq!(UMAC_MDIO_CMD, 0xe14);
    assert_eq!(HFB_RAM, 0x8000);
    assert_eq!(HFB_REG, 0xfc00);
    assert_eq!(HFB_CTRL, 0xfc00);
    assert_eq!(HFB_FLT_ENABLE, 0xfc04);
    assert_eq!(HFB_FLT_LEN, 0xfc1c);
    assert_eq!(HFB_EN, 0x1);
    assert_eq!(TBUF, 0x600);
    assert_eq!(TBUF_CTRL, 0x600);
    assert_eq!(TBUF_64B_EN, 0x1);
    assert_eq!(SYS_TBUF_FLUSH_CTRL, 0xc);
}

#[test]
fn dma_registers_hold_their_documented_values() {
    use kernel_core::genet::dma_registers::*;
    assert_eq!(RING_BYTES, 0x40);
    assert_eq!(RING_COUNT, 0x11);
    assert_eq!(WORDS_PER_DESCRIPTOR, 0x3);
    assert_eq!(DESCRIPTOR_RAM_BYTES, 0xc00);
    assert_eq!(RING_BASE, 0xc00);
    assert_eq!(COMMON_BASE, 0x1040);
    assert_eq!(CTRL, 0x1044);
    assert_eq!(STATUS, 0x1048);
    assert_eq!(SCB_BURST_SIZE, 0x104c);
    assert_eq!(ARB_CTRL, 0x106c);
    assert_eq!(DMA_ARBITER_RR, 0x0);
    assert_eq!(DMA_ARBITER_WRR, 0x1);
    assert_eq!(DMA_ARBITER_SP, 0x2);
    assert_eq!(DMA_PRIORITY_0, 0x1070);
    assert_eq!(DMA_PRIORITY_1, 0x1074);
    assert_eq!(DMA_PRIORITY_2, 0x1078);
    assert_eq!(RING_CFG, 0x1040);
    assert_eq!(INDEX2RING_0, 0x10b0);
    assert_eq!(INDEX2RING_COUNT, 0x8);
    assert_eq!(RING0, 0xc00);
    assert_eq!(DMA_ENABLE, 0x1);
    assert_eq!(RING_BUF_EN_SHIFT, 0x1);
    assert_eq!(READ_PTR, 0x0);
    assert_eq!(READ_PTR_HI, 0x4);
    assert_eq!(CONS_INDEX, 0x8);
    assert_eq!(PROD_INDEX, 0xc);
    assert_eq!(RING_BUF_SIZE, 0x10);
    assert_eq!(START_ADDR, 0x14);
    assert_eq!(START_ADDR_HI, 0x18);
    assert_eq!(END_ADDR, 0x1c);
    assert_eq!(END_ADDR_HI, 0x20);
    assert_eq!(MBUF_DONE_THRESH, 0x24);
    assert_eq!(FLOW_PERIOD, 0x28);
    assert_eq!(WRITE_PTR, 0x2c);
    assert_eq!(WRITE_PTR_HI, 0x30);
    assert_eq!(RING_SIZE_SHIFT, 0x10);
}

#[test]
fn mdio_hold_their_documented_values() {
    use kernel_core::genet::mdio::*;
    assert_eq!(START_BUSY, 0x20000000);
    assert_eq!(READ_FAIL, 0x10000000);
    assert_eq!(READ, 0x8000000);
    assert_eq!(WRITE, 0x4000000);
    assert_eq!(PHY_SHIFT, 0x15);
    assert_eq!(PHY_MASK, 0x1f);
    assert_eq!(REG_SHIFT, 0x10);
    assert_eq!(REG_MASK, 0x1f);
    assert_eq!(DATA_MASK, 0xffff);
    assert_eq!(PHYIDR1, 0x2);
    assert_eq!(PHYIDR2, 0x3);
}

#[test]
fn phy_hold_their_documented_values() {
    use kernel_core::genet::phy::*;
    assert_eq!(BMCR, 0x0);
    assert_eq!(BMSR, 0x1);
    assert_eq!(BMCR_RESET, 0x8000);
    assert_eq!(BMCR_ANENABLE, 0x1000);
    assert_eq!(BMCR_ANRESTART, 0x200);
    assert_eq!(BMSR_LINK, 0x4);
    assert_eq!(BMSR_ANEG_DONE, 0x20);
    assert_eq!(LPA, 0x5);
    assert_eq!(CTRL1000, 0x9);
    assert_eq!(STAT1000, 0xa);
    assert_eq!(LPA_10, 0x60);
    assert_eq!(LPA_100, 0x180);
    assert_eq!(CTRL1000_1000, 0x300);
    assert_eq!(STAT1000_1000, 0xc00);
}

#[test]
fn interrupt_hold_their_documented_values() {
    use kernel_core::genet::interrupt::*;
    assert_eq!(LINK_EVENT, 0x30);
    assert_eq!(MDIO_EVENT, 0x1800000);
    assert_eq!(RX_DONE, 0x2000);
    assert_eq!(TX_DONE, 0x10000);
    assert_eq!(QUEUE_RX_SHIFT, 0x10);
    assert_eq!(QUEUE_TX_MASK, 0xffff);
}
