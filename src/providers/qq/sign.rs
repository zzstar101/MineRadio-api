pub fn hash33(input: &str) -> u32 {
    input.bytes().fold(5381_u32, |hash, byte| {
        hash.wrapping_add(hash << 5).wrapping_add(byte as u32)
    }) & 0x7fff_ffff
}

#[cfg(test)]
mod tests {
    use super::hash33;

    #[test]
    fn hash33_is_stable() {
        assert_eq!(hash33(""), 5381);
        assert_eq!(hash33("abc"), 193485963);
    }
}
