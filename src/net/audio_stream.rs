use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;

use crate::audio::format::SampleFormat;
use crate::net::crypto::PacketCrypto;

pub const DEFAULT_AUDIO_PORT: u16 = 42001;
pub const MAX_PACKET_SIZE: usize = 65507;
pub const ENCRYPTED_AUTH_TAG_SIZE: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioPacketHeader {
    pub magic: u32,
    pub stream_id: u32,
    pub sequence: u64,
    pub timestamp_us: u64,
    pub channels: u16,
    pub sample_rate: u32,
    pub sample_format: u8,
    pub _reserved: u8,
    pub frame_count: u32,
    pub data_len: u32,
}

impl AudioPacketHeader {
    pub const MAGIC: u32 = 0x524A3435;
    pub const SIZE: usize = 40;

    pub fn new(
        stream_id: u32,
        sequence: u64,
        channels: u16,
        sample_rate: u32,
        fmt: SampleFormat,
        frame_count: u32,
    ) -> Self {
        let bps = fmt.bytes_per_sample() as u32;
        Self {
            magic: Self::MAGIC,
            stream_id,
            sequence,
            timestamp_us: 0,
            channels,
            sample_rate,
            sample_format: fmt.to_u8(),
            _reserved: 0,
            frame_count,
            data_len: frame_count * channels as u32 * bps,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::SIZE);
        buf.extend_from_slice(&self.magic.to_le_bytes());
        buf.extend_from_slice(&self.stream_id.to_le_bytes());
        buf.extend_from_slice(&self.sequence.to_le_bytes());
        buf.extend_from_slice(&self.timestamp_us.to_le_bytes());
        buf.extend_from_slice(&self.channels.to_le_bytes());
        buf.extend_from_slice(&self.sample_rate.to_le_bytes());
        buf.push(self.sample_format);
        buf.push(self._reserved);
        buf.extend_from_slice(&self.frame_count.to_le_bytes());
        buf.extend_from_slice(&self.data_len.to_le_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::SIZE {
            return None;
        }
        let magic = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
        if magic != Self::MAGIC {
            return None;
        }
        Some(Self {
            magic,
            stream_id: u32::from_le_bytes(bytes[4..8].try_into().ok()?),
            sequence: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
            timestamp_us: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
            channels: u16::from_le_bytes(bytes[24..26].try_into().ok()?),
            sample_rate: u32::from_le_bytes(bytes[26..30].try_into().ok()?),
            sample_format: bytes[30],
            _reserved: bytes[31],
            frame_count: u32::from_le_bytes(bytes[32..36].try_into().ok()?),
            data_len: u32::from_le_bytes(bytes[36..40].try_into().ok()?),
        })
    }
}

pub async fn send_audio_packet(
    socket: &UdpSocket,
    addr: &std::net::SocketAddr,
    header: &AudioPacketHeader,
    audio_data: &[f32],
    crypto: Option<&PacketCrypto>,
) -> Result<()> {
    let sample_format = SampleFormat::from_u8(header.sample_format).unwrap_or(SampleFormat::F32);
    let sample_bytes = f32_slice_to_bytes(audio_data, sample_format);
    send_raw_audio(socket, addr, header, &sample_bytes, crypto).await
}

pub async fn send_raw_audio(
    socket: &UdpSocket,
    addr: &std::net::SocketAddr,
    header: &AudioPacketHeader,
    raw_audio: &[u8],
    crypto: Option<&PacketCrypto>,
) -> Result<()> {
    let hdr_bytes = header.to_bytes();

    let payload = if let Some(c) = crypto {
        let mut combined = raw_audio.to_vec();
        let tag = c.encrypt(&mut combined, header.sequence);
        let mut pkt =
            Vec::with_capacity(hdr_bytes.len() + combined.len() + ENCRYPTED_AUTH_TAG_SIZE);
        pkt.extend_from_slice(&hdr_bytes);
        pkt.extend_from_slice(&combined);
        pkt.extend_from_slice(&tag.to_le_bytes());
        pkt
    } else {
        let mut pkt = Vec::with_capacity(hdr_bytes.len() + raw_audio.len());
        pkt.extend_from_slice(&hdr_bytes);
        pkt.extend_from_slice(raw_audio);
        pkt
    };

    socket.send_to(&payload, addr).await?;
    Ok(())
}

pub fn parse_audio_packet(data: &[u8]) -> Option<(AudioPacketHeader, Vec<f32>)> {
    let header = AudioPacketHeader::from_bytes(data)?;
    let sample_format = SampleFormat::from_u8(header.sample_format).unwrap_or(SampleFormat::F32);
    let audio_bytes = parse_raw_audio(data, &header)?;
    Some((header, bytes_to_f32_slice(&audio_bytes, sample_format)))
}

pub fn parse_audio_packet_decrypt(
    data: &[u8],
    crypto: Option<&PacketCrypto>,
) -> Option<(AudioPacketHeader, Vec<f32>)> {
    let header = AudioPacketHeader::from_bytes(data)?;
    let sample_format = SampleFormat::from_u8(header.sample_format).unwrap_or(SampleFormat::F32);
    let audio_bytes = if let Some(c) = crypto {
        parse_decrypt_audio(data, &header, c)?
    } else {
        parse_raw_audio(data, &header)?
    };
    Some((header, bytes_to_f32_slice(&audio_bytes, sample_format)))
}

fn parse_raw_audio(data: &[u8], header: &AudioPacketHeader) -> Option<Vec<u8>> {
    let audio_start = AudioPacketHeader::SIZE;
    if data.len() < audio_start + header.data_len as usize {
        return None;
    }
    Some(data[audio_start..audio_start + header.data_len as usize].to_vec())
}

fn parse_decrypt_audio(
    data: &[u8],
    header: &AudioPacketHeader,
    crypto: &PacketCrypto,
) -> Option<Vec<u8>> {
    let audio_start = AudioPacketHeader::SIZE;
    let expected_len = header.data_len as usize + ENCRYPTED_AUTH_TAG_SIZE;
    if data.len() < audio_start + expected_len {
        return None;
    }

    let mut audio = data[audio_start..audio_start + header.data_len as usize].to_vec();
    let received_tag = u32::from_le_bytes(
        data[audio_start + header.data_len as usize..audio_start + expected_len]
            .try_into()
            .ok()?,
    );

    let computed_tag = crypto.decrypt(&mut audio, header.sequence);
    if computed_tag != received_tag {
        log::warn!(
            "Packet auth tag mismatch for seq {}: expected {:08x}, got {:08x}",
            header.sequence,
            computed_tag,
            received_tag
        );
        return None;
    }

    Some(audio)
}

fn f32_slice_to_bytes(samples: &[f32], fmt: SampleFormat) -> Vec<u8> {
    match fmt {
        SampleFormat::F32 => samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
        SampleFormat::I16 => samples
            .iter()
            .flat_map(|s| {
                let v = (*s * 32767.0).clamp(-32768.0, 32767.0) as i16;
                v.to_le_bytes()
            })
            .collect(),
        SampleFormat::I24 => samples
            .iter()
            .flat_map(|s| {
                let v = (*s * 8388607.0).clamp(-8388608.0, 8388607.0) as i32;
                let b = v.to_le_bytes();
                [b[0], b[1], b[2]]
            })
            .collect(),
        SampleFormat::I32 => samples
            .iter()
            .flat_map(|s| {
                let v = (*s * 2147483647.0).clamp(-2147483648.0, 2147483647.0) as i32;
                v.to_le_bytes()
            })
            .collect(),
    }
}

fn bytes_to_f32_slice(bytes: &[u8], fmt: SampleFormat) -> Vec<f32> {
    match fmt {
        SampleFormat::F32 => bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect(),
        SampleFormat::I16 => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                let v = i16::from_le_bytes(*c) as f32;
                (v / 32767.0).clamp(-1.0, 1.0)
            })
            .collect(),
        SampleFormat::I24 => {
            let mut samples = Vec::with_capacity(bytes.len() / 3);
            let mut i = 0;
            while i + 3 <= bytes.len() {
                let sign_byte = if (bytes[i + 2] & 0x80) != 0 {
                    0xFFu8
                } else {
                    0x00u8
                };
                let b = [bytes[i], bytes[i + 1], bytes[i + 2], sign_byte];
                let v = i32::from_le_bytes(b) as f32;
                samples.push((v / 8388607.0).clamp(-1.0, 1.0));
                i += 3;
            }
            samples
        }
        SampleFormat::I32 => bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                let v = i32::from_le_bytes(*c) as f32;
                (v / 2147483647.0).clamp(-1.0, 1.0)
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f32_roundtrip() {
        let original = vec![0.5, -0.3, 1.0, -1.0, 0.0];
        let bytes = f32_slice_to_bytes(&original, SampleFormat::F32);
        let decoded = bytes_to_f32_slice(&bytes, SampleFormat::F32);
        for (a, b) in original.iter().zip(decoded.iter()) {
            assert!((a - b).abs() < 0.0001);
        }
    }

    #[test]
    fn test_i16_roundtrip() {
        let original = vec![0.5, -0.3, 1.0, -1.0, 0.0];
        let bytes = f32_slice_to_bytes(&original, SampleFormat::I16);
        let decoded = bytes_to_f32_slice(&bytes, SampleFormat::I16);
        for (a, b) in original.iter().zip(decoded.iter()) {
            assert!((a - b).abs() < 0.001);
        }
    }

    #[test]
    fn test_i32_roundtrip() {
        let original = vec![0.5, -0.3, 1.0, -1.0, 0.0];
        let bytes = f32_slice_to_bytes(&original, SampleFormat::I32);
        let decoded = bytes_to_f32_slice(&bytes, SampleFormat::I32);
        for (a, b) in original.iter().zip(decoded.iter()) {
            assert!((a - b).abs() < 0.0001);
        }
    }

    #[test]
    fn test_i24_roundtrip() {
        let original = vec![0.5, -0.3, 0.0];
        let bytes = f32_slice_to_bytes(&original, SampleFormat::I24);
        let decoded = bytes_to_f32_slice(&bytes, SampleFormat::I24);
        for (a, b) in original.iter().zip(decoded.iter()) {
            assert!((a - b).abs() < 0.001);
        }
    }

    #[test]
    fn test_encrypted_packet_roundtrip() {
        let crypto =
            PacketCrypto::from_hex("deadbeef0102030405060708deadbeef0102030405060708").unwrap();
        let _header = AudioPacketHeader::new(1, 0, 2, 48000, SampleFormat::F32, 64);
        let samples = vec![0.5f32; 128];

        let raw = f32_slice_to_bytes(&samples, SampleFormat::F32);
        let mut combined = raw.clone();
        let _tag = crypto.encrypt(&mut combined, 0);
        assert_ne!(combined, raw);

        crypto.decrypt(&mut combined, 0);
        assert_eq!(combined, raw);
    }

    #[test]
    fn test_header_roundtrip() {
        let h = AudioPacketHeader::new(42, 1000, 8, 96000, SampleFormat::I16, 256);
        let bytes = h.to_bytes();
        let h2 = AudioPacketHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h2.stream_id, 42);
        assert_eq!(h2.sequence, 1000);
        assert_eq!(h2.channels, 8);
        assert_eq!(h2.sample_rate, 96000);
        assert_eq!(h2.sample_format, 1);
        assert_eq!(h2.frame_count, 256);
        assert_eq!(h2.data_len, 256 * 8 * 2);
    }
}
