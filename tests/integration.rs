use rj45_sound_card::audio::format::{AudioFormat, SampleFormat};
use rj45_sound_card::audio::jitter::JitterBuffer;
use rj45_sound_card::net::audio_stream::{parse_audio_packet_decrypt, AudioPacketHeader};
use rj45_sound_card::net::control::compute_auth_response;
use rj45_sound_card::net::crypto::PacketCrypto;

#[test]
fn test_audio_format_defaults() {
    let fmt = AudioFormat::default();
    assert_eq!(fmt.channels, 2);
    assert_eq!(fmt.sample_rate, 48000);
    assert_eq!(fmt.buffer_frames, 256);
    assert_eq!(fmt.sample_format, SampleFormat::F32);
    assert_eq!(fmt.bytes_per_sample(), 4);
    assert_eq!(fmt.bytes_per_frame(), 8);
    assert_eq!(fmt.buffer_bytes(), 2048);
}

#[test]
fn test_audio_format_i16() {
    let fmt = AudioFormat::with_format(2, 48000, 256, SampleFormat::I16);
    assert_eq!(fmt.bytes_per_sample(), 2);
    assert_eq!(fmt.bytes_per_frame(), 4);
    assert_eq!(fmt.buffer_bytes(), 1024);
}

#[test]
fn test_audio_format_bitrate() {
    let fmt = AudioFormat::with_format(8, 96000, 128, SampleFormat::I32);
    let mbps = fmt.bitrate_mbps();
    assert!((mbps - 24.576).abs() < 0.1);
}

#[test]
fn test_jitter_buffer_basic() {
    let mut jb = JitterBuffer::new(100, 48000, 2);
    jb.push(0, vec![0.5; 512]);
    jb.push(1, vec![0.6; 512]);
    assert_eq!(jb.pop().unwrap()[0], 0.5);
    assert_eq!(jb.pop().unwrap()[0], 0.6);
    assert_eq!(jb.pop(), None);
}

#[test]
fn test_jitter_buffer_reorder() {
    let mut jb = JitterBuffer::new(100, 48000, 2);
    jb.push(2, vec![2.0; 256]);
    jb.push(1, vec![1.0; 256]);
    jb.push(0, vec![0.0; 256]);
    assert_eq!(jb.pop().unwrap()[0], 0.0);
    assert_eq!(jb.pop().unwrap()[0], 1.0);
    assert_eq!(jb.pop().unwrap()[0], 2.0);
}

#[test]
fn test_header_all_formats() {
    for (fmt, expected_val) in &[
        (SampleFormat::F32, 0),
        (SampleFormat::I16, 1),
        (SampleFormat::I24, 2),
        (SampleFormat::I32, 3),
    ] {
        let h = AudioPacketHeader::new(1, 0, 2, 48000, *fmt, 256);
        let bytes = h.to_bytes();
        let decoded = AudioPacketHeader::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.sample_format, *expected_val);
        let roundtrip = SampleFormat::from_u8(decoded.sample_format).unwrap();
        assert_eq!(roundtrip, *fmt);
    }
}

#[test]
fn test_encrypt_decrypt_packet() {
    let crypto =
        PacketCrypto::from_hex("deadbeef0102030405060708deadbeef0102030405060708").unwrap();
    let _header = AudioPacketHeader::new(1, 42, 2, 48000, SampleFormat::F32, 64);
    let samples = vec![0.5f32; 128];

    let parse_result = parse_audio_packet_decrypt(&[], Some(&crypto));
    assert!(parse_result.is_none());

    let raw = samples
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect::<Vec<u8>>();
    let mut encrypted = raw.clone();
    let tag = crypto.encrypt(&mut encrypted, 42);
    assert_ne!(encrypted, raw);

    let verified_tag = crypto.compute_tag(&encrypted, 42);
    assert_eq!(verified_tag, tag);

    crypto.decrypt(&mut encrypted, 42);
    assert_eq!(encrypted, raw);
}

#[test]
fn test_crypto_detects_tampering() {
    let crypto =
        PacketCrypto::from_hex("deadbeef0102030405060708deadbeef0102030405060708").unwrap();
    let _header = AudioPacketHeader::new(1, 0, 2, 48000, SampleFormat::F32, 64);
    let samples = vec![0.5f32; 128];
    let raw: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();

    let mut data = raw.clone();
    let tag = crypto.encrypt(&mut data, 0);
    data[10] ^= 0x01;
    let new_tag = crypto.compute_tag(&data, 0);
    assert_ne!(tag, new_tag);
}

#[test]
fn test_auth_response() {
    let challenge = "deadbeefcafebabe";
    let key = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
    let resp = compute_auth_response(challenge, key);
    assert_eq!(resp.len(), 16);

    let resp2 = compute_auth_response(challenge, key);
    assert_eq!(resp, resp2);

    let resp3 = compute_auth_response("different", key);
    assert_ne!(resp, resp3);
}

#[test]
fn test_sample_format_serde() {
    let json = serde_json::to_string(&SampleFormat::I16).unwrap();
    assert_eq!(json, "\"i16\"");
    let decoded: SampleFormat = serde_json::from_str("\"f32\"").unwrap();
    assert_eq!(decoded, SampleFormat::F32);
    let decoded: SampleFormat = serde_json::from_str("\"i24\"").unwrap();
    assert_eq!(decoded, SampleFormat::I24);
}

#[test]
fn test_sample_format_from_invalid() {
    assert!(SampleFormat::from_u8(99).is_none());
    assert!(SampleFormat::from_u8(255).is_none());
}
