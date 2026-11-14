//! The driver's own words: the numbers its requests and notices go by, and
//! the two structures it reads and fills, laid out as the kernel's headers
//! lay them out.

/// Build a driver request number the way the kernel's headers do.
const fn ioc(dir: u32, kind: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((kind as u32) << 8) | nr as u32
}

const WRITE: u32 = 1;
const READ: u32 = 2;

pub(crate) const BINDER_WRITE_READ: u32 = ioc(WRITE | READ, b'b', 1, size_of::<WriteRead>());
pub(crate) const BINDER_SET_MAX_THREADS: u32 = ioc(WRITE, b'b', 5, 4);
pub(crate) const BINDER_VERSION: u32 = ioc(WRITE | READ, b'b', 9, 4);

pub(crate) const BC_TRANSACTION: u32 = ioc(WRITE, b'c', 0, size_of::<TransactionData>());
pub(crate) const BC_REPLY: u32 = ioc(WRITE, b'c', 1, size_of::<TransactionData>());
pub(crate) const BC_TRANSACTION_SG: u32 = ioc(WRITE, b'c', 17, size_of::<TransactionData>() + 8);
pub(crate) const BC_FREE_BUFFER: u32 = ioc(WRITE, b'c', 3, 8);
pub(crate) const BC_ACQUIRE: u32 = ioc(WRITE, b'c', 5, 4);
pub(crate) const BC_RELEASE: u32 = ioc(WRITE, b'c', 6, 4);
pub(crate) const BC_INCREFS_DONE: u32 = ioc(WRITE, b'c', 8, 16);
pub(crate) const BC_ACQUIRE_DONE: u32 = ioc(WRITE, b'c', 9, 16);
pub(crate) const BC_ENTER_LOOPER: u32 = ioc(0, b'c', 12, 0);

pub(crate) const BR_ERROR: u32 = ioc(READ, b'r', 0, 4);
pub(crate) const BR_OK: u32 = ioc(0, b'r', 1, 0);
pub(crate) const BR_TRANSACTION: u32 = ioc(READ, b'r', 2, size_of::<TransactionData>());
pub(crate) const BR_REPLY: u32 = ioc(READ, b'r', 3, size_of::<TransactionData>());
pub(crate) const BR_DEAD_REPLY: u32 = ioc(0, b'r', 5, 0);
pub(crate) const BR_TRANSACTION_COMPLETE: u32 = ioc(0, b'r', 6, 0);
pub(crate) const BR_INCREFS: u32 = ioc(READ, b'r', 7, 16);
pub(crate) const BR_ACQUIRE: u32 = ioc(READ, b'r', 8, 16);
pub(crate) const BR_RELEASE: u32 = ioc(READ, b'r', 9, 16);
pub(crate) const BR_DECREFS: u32 = ioc(READ, b'r', 10, 16);
pub(crate) const BR_NOOP: u32 = ioc(0, b'r', 12, 0);
pub(crate) const BR_SPAWN_LOOPER: u32 = ioc(0, b'r', 13, 0);
pub(crate) const BR_FAILED_REPLY: u32 = ioc(0, b'r', 17, 0);
pub(crate) const BR_FROZEN_REPLY: u32 = ioc(0, b'r', 18, 0);

#[repr(C)]
#[derive(Default)]
pub(crate) struct WriteRead {
    pub(crate) write_size: u64,
    pub(crate) write_consumed: u64,
    pub(crate) write_buffer: u64,
    pub(crate) read_size: u64,
    pub(crate) read_consumed: u64,
    pub(crate) read_buffer: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub(crate) struct TransactionData {
    pub(crate) target: u64,
    pub(crate) cookie: u64,
    pub(crate) code: u32,
    pub(crate) flags: u32,
    pub(crate) sender_pid: i32,
    pub(crate) sender_euid: u32,
    pub(crate) data_size: u64,
    pub(crate) offsets_size: u64,
    pub(crate) buffer: u64,
    pub(crate) offsets: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    #[test]
    fn request_numbers_match_the_kernels_headers() {
        assert_eq!(BINDER_WRITE_READ, 0xc030_6201);
        assert_eq!(BINDER_SET_MAX_THREADS, 0x4004_6205);
        assert_eq!(BINDER_VERSION, 0xc004_6209);

        assert_eq!(BC_TRANSACTION, 0x4040_6300);
        assert_eq!(BC_REPLY, 0x4040_6301);
        assert_eq!(BC_TRANSACTION_SG, 0x4048_6311);
        assert_eq!(BC_FREE_BUFFER, 0x4008_6303);
        assert_eq!(BC_ACQUIRE, 0x4004_6305);
        assert_eq!(BC_RELEASE, 0x4004_6306);
        assert_eq!(BC_INCREFS_DONE, 0x4010_6308);
        assert_eq!(BC_ACQUIRE_DONE, 0x4010_6309);
        assert_eq!(BC_ENTER_LOOPER, 0x0000_630c);

        assert_eq!(BR_ERROR, 0x8004_7200);
        assert_eq!(BR_OK, 0x0000_7201);
        assert_eq!(BR_TRANSACTION, 0x8040_7202);
        assert_eq!(BR_REPLY, 0x8040_7203);
        assert_eq!(BR_DEAD_REPLY, 0x0000_7205);
        assert_eq!(BR_TRANSACTION_COMPLETE, 0x0000_7206);
        assert_eq!(BR_INCREFS, 0x8010_7207);
        assert_eq!(BR_ACQUIRE, 0x8010_7208);
        assert_eq!(BR_RELEASE, 0x8010_7209);
        assert_eq!(BR_DECREFS, 0x8010_720a);
        assert_eq!(BR_NOOP, 0x0000_720c);
        assert_eq!(BR_SPAWN_LOOPER, 0x0000_720d);
        assert_eq!(BR_FAILED_REPLY, 0x0000_7211);
        assert_eq!(BR_FROZEN_REPLY, 0x0000_7212);
    }

    #[test]
    fn the_structures_are_laid_out_as_the_driver_reads_them() {
        assert_eq!(size_of::<WriteRead>(), 48);
        assert_eq!(
            [
                offset_of!(WriteRead, write_size),
                offset_of!(WriteRead, write_consumed),
                offset_of!(WriteRead, write_buffer),
                offset_of!(WriteRead, read_size),
                offset_of!(WriteRead, read_consumed),
                offset_of!(WriteRead, read_buffer),
            ],
            [0, 8, 16, 24, 32, 40]
        );
        assert_eq!(size_of::<TransactionData>(), 64);
        assert_eq!(
            [
                offset_of!(TransactionData, target),
                offset_of!(TransactionData, cookie),
                offset_of!(TransactionData, code),
                offset_of!(TransactionData, flags),
                offset_of!(TransactionData, sender_pid),
                offset_of!(TransactionData, sender_euid),
                offset_of!(TransactionData, data_size),
                offset_of!(TransactionData, offsets_size),
                offset_of!(TransactionData, buffer),
                offset_of!(TransactionData, offsets),
            ],
            [0, 8, 16, 20, 24, 28, 32, 40, 48, 56]
        );
    }
}
