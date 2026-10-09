use crate::config::*;

use crate::pdu::{PduByte0, PduByte1};

use embassy_nrf::pac::radio::regs::Rxaddresses;
use embassy_nrf::{
    Peri,
    pac::{
        self,
        radio::{
            Radio,
            regs::Datawhiteiv,
            vals::{self, S1incl, Txpower},
        },
    },
    peripherals,
};

#[derive(Clone, Copy, Default)]
pub(crate) struct RadioEvents {
    pub crc_ok: bool,
    pub disabled: bool,
}

pub(crate) struct YipRadio {
    pub channel: u8,

    #[allow(dead_code)]
    radio: Peri<'static, peripherals::RADIO>,
}

impl YipRadio {
    pub fn regs(&mut self) -> Radio {
        pac::RADIO
    }

    pub fn new(radio: Peri<'static, peripherals::RADIO>) -> Self {
        Self {
            radio: radio,
            channel: 0,
        }
    }

    pub(crate) fn init(&mut self) {
        self.set_radio_frequency();
        self.configure_crc();
        self.set_base_addr();
        self.configure_payload_format();
        self.set_short();
        self.set_radio_rampup_mode();

        defmt::debug!("radio setup complete");
    }

    fn set_radio_rampup_mode(&mut self) {
        pac::RADIO.modecnf0().write(|w| w.set_ru(RADIO_RAMPUP_MODE));
    }

    fn set_radio_frequency(&mut self) {
        // set radio to 2Mbit
        pac::RADIO.mode().write(|w| w.set_mode(RADIO_BITRATE));
    }

    fn configure_crc(&mut self) {
        let radio = pac::RADIO;

        // CRC 초기값
        radio.crcinit().write(|w| w.set_crcinit(0xFFFF));

        // CRC 길이와 주소 포함 여부
        radio.crccnf().write(|w| {
            w.set_len(RADIO_CRC_LEN_MODE);
            w.set_skipaddr(RADIO_CRC_SKIPADDR);
        });

        // x^16 + x^12 + x^5 + 1
        radio.crcpoly().write(|w| w.set_crcpoly(0x11021));
    }

    fn configure_payload_format(&mut self) {
        let radio = pac::RADIO;

        radio.pcnf0().write(|w| {
            w.set_lflen(PduByte0::LENGTH_BIT_COUNT as u8);
            w.set_s0len(false);
            w.set_s1len(PduByte1::S1_BITS);

            w.set_s1incl(S1incl::Automatic);
            w.set_cilen(0);
            w.set_plen(RADIO_PREAMBLE_LEN_MODE);
            w.set_crcinc(vals::Crcinc::Exclude);
            w.set_termlen(0);
        });

        radio.pcnf1().write(|w| {
            w.set_maxlen(YIP_SUB_PACKET_SIZE as u8);
            w.set_statlen(0);
            w.set_endian(vals::Endian::Big);
            w.set_balen(RADIO_ADDR_BALEN);
            w.set_whiteen(true);
        });
    }

    fn set_base_addr(&mut self) {
        use pac::radio::regs::Prefix0;
        use pac::radio::regs::Prefix1;

        let radio = pac::RADIO;

        radio
            .prefix0()
            .write_value(Prefix0(u32::from_le_bytes(RADIO_ADDR_PREFIX_0)));
        radio
            .prefix1()
            .write_value(Prefix1(u32::from_le_bytes(RADIO_ADDR_PREFIX_1)));
        radio
            .base0()
            .write_value(u32::from_le_bytes(RADIO_ADDR_BASE_0));
        radio
            .base1()
            .write_value(u32::from_le_bytes(RADIO_ADDR_BASE_1));
    }

    fn set_short(&mut self) {
        let radio = pac::RADIO;

        radio.shorts().write(|w| {
            w.set_ready_start(true);
            w.set_end_disable(true);
            w.set_address_rssistart(true);
            w.set_disabled_rssistop(true);
        });
    }

    pub unsafe fn set_packetptr(&mut self, packet: *mut u8) {
        pac::RADIO.packetptr().write_value(packet as usize as u32);
    }

    pub fn set_txaddress(&mut self, pipe: u8) {
        pac::RADIO.txaddress().write(|w| w.set_txaddress(pipe));
    }

    pub fn set_rxaddress(&mut self, pipe: u8) {
        pac::RADIO
            .rxaddresses()
            .write_value(Rxaddresses(1u32 << pipe));
    }

    pub fn set_channel(&mut self, channel: u8) {
        self.channel = channel;
        pac::RADIO.frequency().write(|w| w.set_frequency(channel));
    }

    pub(crate) fn take_interrupt_events(&mut self) -> RadioEvents {
        let radio = embassy_nrf::pac::RADIO;
        let mut events = RadioEvents::default();

        if radio.events_crcok().read() != 0 {
            radio.events_crcok().write_value(0);
            events.crc_ok = true;
        }

        if radio.events_disabled().read() != 0 {
            radio.events_disabled().write_value(0);
            events.disabled = true;
        }

        events
    }

    #[allow(dead_code)]
    pub fn irq_set_disabled(&mut self, set: bool) {
        pac::RADIO.intenset().write(|w| w.set_disabled(set));
    }

    #[allow(dead_code)]
    pub fn irq_set_crcok(&mut self, set: bool) {
        pac::RADIO.intenset().write(|w| w.set_crcok(set));
    }

    pub fn set_whiten_seed(&mut self, channel: u8) {
        pac::RADIO
            .datawhiteiv()
            .write_value(Datawhiteiv(channel as u32));
    }

    pub fn get_rssi_sample(&mut self) -> u8 {
        pac::RADIO.rssisample().read().rssisample()
    }

    pub fn get_rxcrc(&mut self) -> u32 {
        pac::RADIO.rxcrc().read().rxcrc()
    }

    #[allow(dead_code)]
    pub fn set_tx_power(&mut self, power: Txpower) {
        pac::RADIO.txpower().write(|w| w.set_txpower(power));
    }

    #[allow(dead_code)]
    pub fn disable(&mut self) {
        pac::RADIO.tasks_disable().write_value(1);
        while pac::RADIO.events_disabled().read() == 0 {}
        pac::RADIO.events_disabled().write_value(0);
    }
}
