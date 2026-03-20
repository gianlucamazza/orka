/// ABI version this host supports.
pub const ABI_VERSION: i32 = 2;

/// Pack a (ptr, len) pair into a single `i64` for the plugin ABI.
pub fn pack_ptr_len(ptr: u32, len: u32) -> i64 {
    ((ptr as i64) << 32) | (len as i64)
}

/// Unpack a packed `i64` back into (ptr, len).
pub fn unpack_ptr_len(packed: i64) -> (u32, u32) {
    let ptr = ((packed >> 32) & 0xFFFF_FFFF) as u32;
    let len = (packed & 0xFFFF_FFFF) as u32;
    (ptr, len)
}

/// Well-known export names for the v2 plugin ABI.
pub mod exports {
    /// Export name for the ABI version query.
    pub const ABI_VERSION: &str = "orka_abi_version";
    /// Export name for retrieving plugin metadata.
    pub const PLUGIN_INFO: &str = "orka_plugin_info";
    /// Export name for the plugin initialisation entry point.
    pub const PLUGIN_INIT: &str = "orka_plugin_init";
    /// Export name for the main plugin execution entry point.
    pub const PLUGIN_EXECUTE: &str = "orka_plugin_execute";
    /// Export name for the plugin cleanup/teardown entry point.
    pub const PLUGIN_CLEANUP: &str = "orka_plugin_cleanup";
    /// Export name for the guest-side memory allocator.
    pub const ALLOC: &str = "orka_alloc";
    /// Export name for the guest-side memory deallocator.
    pub const DEALLOC: &str = "orka_dealloc";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let ptr: u32 = 0x0001_0000;
        let len: u32 = 42;
        let packed = pack_ptr_len(ptr, len);
        let (p2, l2) = unpack_ptr_len(packed);
        assert_eq!((ptr, len), (p2, l2));
    }
}
