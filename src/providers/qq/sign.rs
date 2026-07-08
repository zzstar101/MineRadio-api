pub fn hash33(input: &str) -> u32 {
    input.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_add(hash << 5).wrapping_add(byte as u32)
    }) & 0x7fff_ffff
}

pub fn gtk_from_pskey(input: &str) -> u32 {
    input.bytes().fold(5381_u32, |hash, byte| {
        hash.wrapping_add(hash << 5).wrapping_add(byte as u32)
    }) & 0x7fff_ffff
}

#[cfg(test)]
mod tests {
    use super::{gtk_from_pskey, hash33};

    #[test]
    fn hash33_is_stable() {
        assert_eq!(hash33(""), 0);
        assert_eq!(hash33("abc"), 108966);
        assert_eq!(gtk_from_pskey(""), 5381);
        assert_eq!(gtk_from_pskey("abc"), 193485963);
    }
}
