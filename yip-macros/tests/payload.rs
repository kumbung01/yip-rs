// The included runtime file has the parent crate's optional defmt feature.
#![allow(unexpected_cfgs)]

// Exercise the actual protocol types and primitive codecs without MCU dependencies.
#[allow(dead_code)]
#[path = "../../src/payload.rs"]
mod payload;

use payload::{ControlHeader, ControlCommand, DownHeader, Payload, SyncPacket, UpHeader};

#[test]
fn sync_has_exact_little_endian_wire_layout() {
    assert_eq!(SyncPacket::SIZE, 9);
    let packet = SyncPacket {
        refslot: 0x12345678,
        slot_ticks: 250_000,
        pipes: 2,
    };
    let mut bytes = [0; SyncPacket::SIZE];
    packet.encode(&mut bytes);
    assert_eq!(bytes, [0x78, 0x56, 0x34, 0x12, 0x90, 0xd0, 0x03, 0, 2]);
    let decoded = SyncPacket::decode(&bytes).unwrap();
    assert_eq!(decoded.refslot, 0x12345678);
    assert_eq!(decoded.slot_ticks, 250_000);
    assert_eq!(decoded.pipes, 2);
    for len in 0..SyncPacket::SIZE {
        assert!(SyncPacket::decode(&bytes[..len]).is_err());
    }
    assert!(SyncPacket::decode(&[0; 10]).is_err());
}

#[test]
fn header_boundaries_do_not_consume_application_data() {
    assert_eq!(UpHeader::SIZE, 0);
    assert!(UpHeader::decode(&[]).is_ok());
    assert!(UpHeader::decode(&[2, 0, 0, 0]).is_err());
    let mut bytes = [0, 2, 0, 0, 0];
    DownHeader { rssi: 60 }.encode(&mut bytes);
    assert_eq!(bytes, [60, 2, 0, 0, 0]);
    assert_eq!(
        DownHeader::decode(&bytes[..DownHeader::SIZE]).unwrap().rssi,
        60
    );
}

#[test]
fn control_is_one_byte_and_rejects_invalid_discriminants() {
    assert_eq!(ControlHeader::SIZE, 1);
    let mut bytes = [0xff];
    ControlHeader {
        command: ControlCommand::Sync,
    }
    .encode(&mut bytes);
    assert_eq!(bytes, [0]);
    assert!(ControlHeader::decode(&bytes).unwrap().command == ControlCommand::Sync);
    assert!(ControlHeader::decode(&[]).is_err());
    assert!(ControlHeader::decode(&[0, 0]).is_err());
    assert!(ControlHeader::decode(&[1]).is_err());
}

#[test]
fn encoding_supports_unaligned_destination_and_preserves_surrounding_bytes() {
    let mut bytes = [0xa5; 11];
    SyncPacket { refslot: 0x12345678, slot_ticks: 250_000, pipes: 2 }
        .encode(&mut bytes[1..10]);
    assert_eq!(bytes, [0xa5, 0x78, 0x56, 0x34, 0x12, 0x90, 0xd0, 3, 0, 2, 0xa5]);
}

#[test]
fn short_destination_is_rejected_before_writing() {
    let mut bytes = [0xa5; 8];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        SyncPacket { refslot: 1, slot_ticks: 500, pipes: 2 }.encode(&mut bytes);
    }));
    assert!(result.is_err());
    assert_eq!(bytes, [0xa5; 8]);
}

#[derive(Payload)]
struct Nested<T> {
    tag: u8,
    value: T,
    bytes: [u8; 2],
}

#[derive(Payload)]
struct Pair(u16, i32);

#[derive(Payload)]
struct Empty;

#[repr(u8)]
#[derive(Payload)]
enum Explicit {
    First = 3,
    Second = 9,
}

#[test]
fn nested_generic_tuple_unit_and_explicit_enum_layouts() {
    let mut bytes = [0; Nested::<Pair>::SIZE];
    Nested {
        tag: 7,
        value: Pair(0x1234, -2),
        bytes: [8, 9],
    }
    .encode(&mut bytes);
    assert_eq!(bytes, [7, 0x34, 0x12, 0xfe, 0xff, 0xff, 0xff, 8, 9]);
    let decoded = Nested::<Pair>::decode(&bytes).unwrap();
    assert_eq!(decoded.value.0, 0x1234);
    assert_eq!(decoded.value.1, -2);
    assert_eq!(decoded.bytes, [8, 9]);
    assert_eq!(Empty::SIZE, 0);
    Empty.encode(&mut []);
    assert!(Empty::decode(&[]).is_ok());
    assert!(matches!(Explicit::decode(&[9]), Ok(Explicit::Second)));
    assert!(Explicit::decode(&[0]).is_err());
}

#[test]
fn decoding_supports_unaligned_input_and_checks_lengths() {
    #[repr(align(4))]
    struct Aligned([u8; 11]);
    let bytes = Aligned([0xa5, 0x78, 0x56, 0x34, 0x12, 0x90, 0xd0, 3, 0, 2, 0xa5]);
    let packet = SyncPacket::decode(&bytes.0[1..10]).unwrap();
    assert_eq!(packet.refslot, 0x12345678);
    assert_eq!(packet.slot_ticks, 250_000);
    assert_eq!(packet.pipes, 2);
    assert_eq!(u32::decode(&bytes.0[1..5]), Ok(0x12345678));
    assert!(u32::decode(&bytes.0[1..4]).is_err());
    assert!(u32::decode(&bytes.0[1..6]).is_err());
    assert_eq!(<[u8; 0]>::decode(&[]), Ok([]));
    assert!(<[u8; 2]>::decode(&[1]).is_err());
}

#[test]
fn nested_enum_values_are_still_validated() {
    #[derive(Payload)]
    struct Tagged { prefix: u16, kind: Explicit, suffix: [u8; 2] }
    assert_eq!(Tagged::SIZE, 5);
    for value in 0..=u8::MAX {
        let decoded = Tagged::decode(&[0x34, 0x12, value, 8, 9]);
        assert_eq!(decoded.is_ok(), value == 3 || value == 9);
        if let Ok(decoded) = decoded {
            assert_eq!(decoded.prefix, 0x1234);
            assert_eq!(decoded.suffix, [8, 9]);
        }
    }
    assert!(Tagged::decode(&[0; 4]).is_err());
    assert!(Tagged::decode(&[0; 6]).is_err());
}
