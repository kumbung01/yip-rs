use crate::hardware::api::{EguDriver, PpiDriver};

use embassy_nrf::{
    Peri,
    pac::{
        self,
        common::{RW, Reg},
        shared,
    },
    ppib::{BridgeChannel, Ppib},
};

use crate::hardware::{
    clock::Nrf54Clock,
    egu::{Nrf54Egu, Trigger},
};

#[path = "ppi/resources.rs"]
mod resources;
pub use resources::*;

#[macro_export]
macro_rules! ppi_channels {
    ($($channel:expr),+ $(,)?) => {
        [$(::core::convert::Into::into($channel)),+]
    };
}

// it uses EGU10, TIMER10 fixed for now
// needs grtc cc

pub(crate) struct Nrf54Ppi {
    radio_channels: Channels<'static, Radio, 3>,
    peri_channels: Channels<'static, PeriDomain, 1>,
    peri_to_radio: Ppib<'static>,
    group: Group<'static, Radio>,
}

impl Nrf54Ppi {
    pub(crate) fn new<S: BridgeChannel>(
        radio_channels: [Channel<'static, Radio>; 3],
        peri_channels: [Channel<'static, PeriDomain>; 1],
        peri_to_radio: (Peri<'static, S>, Peri<'static, S::Paired>),
        group: Group<'static, Radio>,
    ) -> Self {
        let mut this = Self {
            radio_channels: Channels::new(radio_channels),
            peri_channels: Channels::new(peri_channels),
            peri_to_radio: Ppib::new(peri_to_radio.0, peri_to_radio.1),
            group,
        };

        this.clear_all();
        this
    }

    fn connect<D: Domain>(
        channel: &Channel<'_, D>,
        pubs: &[Reg<shared::regs::Publish, RW>],
        subs: &[Reg<shared::regs::Subscribe, RW>],
    ) {
        let ch = channel.number();
        let dm = channel.domain();

        for publish in pubs {
            publish.write(|w| {
                w.set_chidx(ch as u8);
                w.set_en(true);
            });
        }

        for subscribe in subs {
            subscribe.write(|w| {
                w.set_chidx(ch as u8);
                w.set_en(true);
            });
        }

        dm.chenset().write(|w| w.set_ch(ch, true));
    }

    fn clear_all(&mut self) {
        self.clear();

        // DPPI endpoints are configured on each peripheral.
        pac::GRTC.publish_compare(0).write(|w| w.set_en(false));
        self.peri_to_radio.disable();
        pac::RADIO.subscribe_txen().write(|w| w.set_en(false));
        pac::RADIO.subscribe_rxen().write(|w| w.set_en(false));
        pac::RADIO.subscribe_disable().write(|w| w.set_en(false));
        pac::RADIO.publish_disabled().write(|w| w.set_en(false));
        pac::RADIO.publish_crcok().write(|w| w.set_en(false));
        pac::TIMER10.subscribe_start().write(|w| w.set_en(false));
        pac::TIMER10.subscribe_stop().write(|w| w.set_en(false));
        pac::TIMER10.subscribe_clear().write(|w| w.set_en(false));
        pac::TIMER10.publish_compare(0).write(|w| w.set_en(false));
        pac::EGU10
            .subscribe_trigger(Trigger::Start as usize)
            .write(|w| w.set_en(false));
        self.group.clear();
    }
}

// central impl

// peripheral impl

impl Drop for Nrf54Ppi {
    fn drop(&mut self) {
        self.clear_all();
    }
}

impl PpiDriver for Nrf54Ppi {
    type Clock = Nrf54Clock;
    type Egu = Nrf54Egu;

    fn clear(&mut self) {
        self.disable();
        self.group.clear();
    }

    fn enable(&mut self) {
        self.radio_channels.enable();
        self.peri_channels.enable();
    }

    fn disable(&mut self) {
        self.radio_channels.disable();
        self.peri_channels.disable();
    }

    fn configure_central(&mut self, clock: &mut Nrf54Clock, egu: &mut Nrf54Egu) {
        self.clear_all();

        let radio = pac::RADIO;
        let timer = pac::TIMER10;
        let grtc = pac::GRTC;
        let egu = egu.regs();

        let trigger = Trigger::TimeslotEnd as usize;

        // Configure group membership before enabling group subscriptions.
        self.group.include(self.radio_channels.number(1));

        Self::connect(
            self.peri_channels.channel(0),
            &[grtc.publish_compare(clock.grtc_num())],
            &[self.peri_to_radio.subscribe_send()],
        );

        Self::connect(
            self.radio_channels.channel(0),
            &[self.peri_to_radio.publish_receive()],
            &[
                radio.subscribe_txen(),
                timer.subscribe_start(),
                self.group.subscribe_en(),
            ],
        );

        Self::connect(
            self.radio_channels.channel(1),
            &[radio.publish_disabled()],
            &[radio.subscribe_rxen(), self.group.subscribe_dis()],
        );

        Self::connect(
            self.radio_channels.channel(2),
            &[timer.publish_compare(0), radio.publish_crcok()],
            &[
                egu.subscribe_trigger(trigger),
                radio.subscribe_disable(),
                timer.subscribe_stop(),
                timer.subscribe_clear(),
            ],
        );
    }

    fn configure_sync(&mut self, _clock: &mut Nrf54Clock, _egu: &mut Nrf54Egu) {
        todo!();
    }

    fn configure_peripheral_prepare_rx(&mut self, _clock: &mut Nrf54Clock, _egu: &mut Nrf54Egu) {
        todo!();
    }

    fn configure_peripheral_next_rx(&mut self, _clock: &mut Nrf54Clock, _egu: &mut Nrf54Egu) {
        todo!();
    }

    fn configure_peripheral_tx_ack(&mut self, _egu: &mut Nrf54Egu) {
        todo!();
    }
}
