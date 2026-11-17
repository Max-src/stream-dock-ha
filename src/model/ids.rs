//! The ids that tell this PC, its pads and its profiles apart, as the
//! integration protocol's section 2 describes (docs/integration-protocol.md).

/// A new random id: 16 lowercase hex characters.
pub fn new_id() -> String {
    let mut bytes = [0u8; 8];
    if !system_random(&mut bytes) {
        // Windows always has a random source; this is only a fallback so an id
        // is never empty. The time and a counter keep it from repeating.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        let mixed = fnv1a(format!("{now}:{}:{}", COUNTER.fetch_add(1, Ordering::SeqCst), std::process::id()).as_bytes());
        bytes = mixed.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fills `bytes` from Windows' own random source.
fn system_random(bytes: &mut [u8]) -> bool {
    use windows_sys::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), bytes.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    status >= 0
}

/// Whether some text has the shape of an id.
pub fn is_id(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// FNV-1a, 64 bits.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, b| (hash ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The protocol's `machine` value for a MachineGuid: FNV-1a 64 of
/// `"stream-dock-ha:" + guid`, as 16 lowercase hex characters.
pub fn machine_hash_of(guid: &str) -> String {
    format!("{:016x}", fnv1a(format!("stream-dock-ha:{guid}").as_bytes()))
}

/// This PC's `machine` value. It tells a settings file copied from another
/// PC apart from this PC's own, without saying anything about the PC.
pub fn machine_hash() -> String {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| machine_hash_of(&machine_guid().unwrap_or_else(|| "unknown".into()))).clone()
}

/// Windows' own id for this installation, from the 64-bit registry, which is
/// where it lives whatever the app is built for.
fn machine_guid() -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, RRF_RT_REG_SZ,
    };
    let path = crate::win::wide("SOFTWARE\\Microsoft\\Cryptography");
    let name = crate::win::wide("MachineGuid");
    let mut key: HKEY = std::ptr::null_mut();
    unsafe {
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ | KEY_WOW64_64KEY, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = [0u16; 128];
        let mut bytes = (buffer.len() * 2) as u32;
        let status = RegGetValueW(key, std::ptr::null(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut bytes);
        RegCloseKey(key);
        let guid = crate::win::from_wide(&buffer);
        (status == ERROR_SUCCESS && !guid.trim().is_empty()).then(|| guid.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_sixteen_hex_characters_and_differ() {
        let (a, b) = (new_id(), new_id());
        assert!(is_id(&a) && is_id(&b), "{a} {b}");
        assert_ne!(a, b);
    }

    #[test]
    fn the_machine_value_is_fnv1a_of_the_guid() {
        // FNV-1a 64 of the empty input is its offset basis.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        let hash = machine_hash_of("made-up-guid");
        assert!(is_id(&hash), "{hash}");
        assert_eq!(hash, format!("{:016x}", fnv1a(b"stream-dock-ha:made-up-guid")));
        assert_ne!(hash, machine_hash_of("unknown"));
        assert!(is_id(&machine_hash()));
        assert_eq!(machine_hash(), machine_hash());
    }
}
