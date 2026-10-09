use crate::role::YipRole;

pub(crate) trait YipPayload {
    fn encoded_len(&self) -> usize;
    fn encode_into(self, dst: &mut [u8]);
}

pub(crate) trait YipRoleHeader: YipRole {
    type TxHeader: YipHeader;
    type RxHeader: YipHeader;
}

pub(crate) trait YipHeader: YipPayload {
    const SIZE: usize;

    fn new() -> Self;
    fn copy(&mut self, src: &[u8]) -> usize;
}

impl YipPayload for &[u8] {
    fn encoded_len(&self) -> usize {
        self.len()
    }

    fn encode_into(self, dst: &mut [u8]) {
        dst.copy_from_slice(self);
    }
}

impl YipPayload for u8 {
    fn encoded_len(&self) -> usize {
        1
    }

    fn encode_into(self, dst: &mut [u8]) {
        dst[0] = self;
    }
}

pub(crate) struct DownHeader {
    pub(crate) rssi: u8,
}

impl YipPayload for DownHeader {
    fn encoded_len(&self) -> usize {
        1
    }

    fn encode_into(self, dst: &mut [u8]) {
        dst[0] = self.rssi;
    }
}

impl YipHeader for DownHeader {
    const SIZE: usize = 1;

    fn new() -> Self {
        Self { rssi: 0 }
    }

    fn copy(&mut self, src: &[u8]) -> usize {
        self.rssi = src[0];

        1
    }
}

pub(crate) struct UpHeader {
    pub(crate) dummy: [u8; 4],
}

impl YipPayload for UpHeader {
    fn encoded_len(&self) -> usize {
        4
    }

    fn encode_into(self, dst: &mut [u8]) {
        dst[0..4].copy_from_slice(&self.dummy);
    }
}

impl YipHeader for UpHeader {
    const SIZE: usize = 4;

    fn new() -> Self {
        Self { dummy: [0; 4] }
    }

    fn copy(&mut self, src: &[u8]) -> usize {
        self.dummy.copy_from_slice(&src[0..4]);

        4
    }
}
