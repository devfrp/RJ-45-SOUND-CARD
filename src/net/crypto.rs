use std::hash::Hasher;

#[derive(Debug, Clone)]
pub struct PacketCrypto {
    key: Vec<u8>,
}

impl PacketCrypto {
    pub fn new(key: Vec<u8>) -> Self {
        assert!(!key.is_empty(), "Encryption key must not be empty");
        Self { key }
    }

    pub fn from_hex(key_hex: &str) -> Result<Self, String> {
        let key = hex::decode(key_hex).map_err(|e| format!("Invalid hex key: {}", e))?;
        if key.is_empty() {
            return Err("Key must not be empty".to_string());
        }
        Ok(Self { key })
    }

    pub fn encrypt(&self, data: &mut [u8], sequence: u64) -> u32 {
        self.xor_with_key(data, sequence);
        self.compute_tag(data, sequence)
    }

    pub fn decrypt(&self, data: &mut [u8], sequence: u64) -> u32 {
        self.xor_with_key(data, sequence);
        self.compute_tag(data, sequence)
    }

    pub fn compute_tag(&self, data: &[u8], sequence: u64) -> u32 {
        let mut hasher = siphasher::sip::SipHasher::new_with_keys(0, 0);
        hasher.write(&self.key);
        hasher.write(&sequence.to_le_bytes());
        hasher.write(data);
        let hash = hasher.finish();
        crc32fast::hash(&hash.to_le_bytes())
    }

    fn xor_with_key(&self, data: &mut [u8], sequence: u64) {
        let mut hasher = siphasher::sip::SipHasher::new_with_keys(0, 0);
        hasher.write(&self.key);
        hasher.write(&sequence.to_le_bytes());
        let offset = (hasher.finish() % self.key.len() as u64) as usize;

        for (i, byte) in data.iter_mut().enumerate() {
            let key_idx = (offset + i) % self.key.len();
            *byte ^= self.key[key_idx];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let crypto =
            PacketCrypto::from_hex("deadbeef0102030405060708deadbeef0102030405060708").unwrap();
        let original = b"Hello, audio world!".to_vec();
        let mut data = original.clone();
        let _tag1 = crypto.encrypt(&mut data, 42);
        assert_ne!(data, original);
        let _tag2 = crypto.decrypt(&mut data, 42);
        assert_eq!(data, original);
    }

    #[test]
    fn test_tag_detects_tampering() {
        let crypto =
            PacketCrypto::from_hex("deadbeef0102030405060708deadbeef0102030405060708").unwrap();
        let mut data = vec![1, 2, 3, 4, 5];
        let tag = crypto.encrypt(&mut data, 0);
        data[0] ^= 0xFF;
        let new_tag = crypto.compute_tag(&data, 0);
        assert_ne!(tag, new_tag);
    }

    #[test]
    fn test_different_sequence_different_output() {
        let crypto =
            PacketCrypto::from_hex("deadbeef0102030405060708deadbeef0102030405060708").unwrap();
        let mut d1 = vec![0u8; 32];
        let mut d2 = vec![0u8; 32];
        crypto.encrypt(&mut d1, 0);
        crypto.encrypt(&mut d2, 1);
        assert_ne!(d1, d2);
    }
}
