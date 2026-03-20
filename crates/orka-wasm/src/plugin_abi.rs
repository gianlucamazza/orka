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

    #[test]
    fn pack_zero_values() {
        let packed = pack_ptr_len(0, 0);
        assert_eq!(packed, 0);
        assert_eq!(unpack_ptr_len(packed), (0, 0));
    }

    #[test]
    fn pack_max_values() {
        let packed = pack_ptr_len(u32::MAX, u32::MAX);
        let (ptr, len) = unpack_ptr_len(packed);
        assert_eq!(ptr, u32::MAX);
        assert_eq!(len, u32::MAX);
    }

    #[test]
    fn pack_ptr_only() {
        let packed = pack_ptr_len(100, 0);
        let (ptr, len) = unpack_ptr_len(packed);
        assert_eq!(ptr, 100);
        assert_eq!(len, 0);
    }

    #[test]
    fn pack_len_only() {
        let packed = pack_ptr_len(0, 256);
        let (ptr, len) = unpack_ptr_len(packed);
        assert_eq!(ptr, 0);
        assert_eq!(len, 256);
    }

    #[test]
    fn abi_version_is_2() {
        assert_eq!(ABI_VERSION, 2);
    }

    #[test]
    fn export_names_are_prefixed() {
        assert!(exports::ABI_VERSION.starts_with("orka_"));
        assert!(exports::PLUGIN_INFO.starts_with("orka_"));
        assert!(exports::PLUGIN_INIT.starts_with("orka_"));
        assert!(exports::PLUGIN_EXECUTE.starts_with("orka_"));
        assert!(exports::PLUGIN_CLEANUP.starts_with("orka_"));
        assert!(exports::ALLOC.starts_with("orka_"));
        assert!(exports::DEALLOC.starts_with("orka_"));
    }
}
