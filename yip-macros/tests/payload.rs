// The included runtime file has the parent crate's optional defmt feature.
#![allow(unexpected_cfgs)]

// Exercise the actual protocol types and primitive codecs without MCU dependencies.
#[allow(dead_code)]
#[path = "../../src/payload.rs"]
mod payload;

use payload::{ControlPacket, ControlType, DownHeader, Payload, SyncPacket, UpHeader};

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
    assert_eq!(ControlPacket::SIZE, 1);
    let mut bytes = [0xff];
    ControlPacket {
        ctrl_type: ControlType::DropCurrentTx,
    }
    .encode(&mut bytes);
    assert_eq!(bytes, [0]);
    assert!(ControlPacket::decode(&bytes).unwrap().ctrl_type == ControlType::DropCurrentTx);
    assert!(ControlPacket::decode(&[]).is_err());
    assert!(ControlPacket::decode(&[0, 0]).is_err());
    assert!(ControlPacket::decode(&[1]).is_err());
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
