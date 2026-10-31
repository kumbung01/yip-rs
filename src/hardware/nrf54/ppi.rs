use crate::hardware::api::egu::{PrepareRx, Rx, TxAck};
use crate::hardware::api::{EguDriver, PpiDriver};
use crate::hardware::{
    clock::Nrf54Clock,
    egu::{Nrf54Egu, Trigger},
};
use embassy_nrf::{
    Peri,
    pac::{
        self,
        common::{RW, Reg},
        shared,
    },
    ppib::{BridgeChannel, Ppib},
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

pub(crate) const RADIO_CHANNEL_COUNT: usize = 11;

pub(crate) struct Nrf54Ppi {
    radio_channels: Channels<'static, Radio, RADIO_CHANNEL_COUNT>,
    peri_channels: Channels<'static, PeriDomain, 1>,
    peri_to_radio: Ppib<'static>,
    group: Group<'static, Radio>,
    error_group: Group<'static, Radio>,
    completion_group: Group<'static, Radio>,
    ack_group: Group<'static, Radio>,
    grtc_channel: Option<usize>,
    active_mask: u32,
}

impl Nrf54Ppi {
    pub(crate) fn new<S: BridgeChannel>(
        radio_channels: [Channel<'static, Radio>; RADIO_CHANNEL_COUNT],
        peri_channels: [Channel<'static, PeriDomain>; 1],
        peri_to_radio: (Peri<'static, S>, Peri<'static, S::Paired>),
        groups: [Group<'static, Radio>; 4],
    ) -> Self {
        let [group, error_group, completion_group, ack_group] = groups;
        let mut this = Self {
            radio_channels: Channels::new(radio_channels),
            peri_channels: Channels::new(peri_channels),
            peri_to_radio: Ppib::new(peri_to_radio.0, peri_to_radio.1),
            group,
            error_group,
            completion_group,
            ack_group,
            grtc_channel: None,
            active_mask: 0,
        };
        this.clear_all();
        this
    }

    // Wiring never arms a channel. Publish/subscribe bindings remain fixed
    // throughout connected RX/ACK; only CHEN masks change between states.
    fn connect<D: Domain>(
        channel: &Channel<'_, D>,
        pubs: &[Reg<shared::regs::Publish, RW>],
        subs: &[Reg<shared::regs::Subscribe, RW>],
    ) {
        let ch = channel.number() as u8;
        for publish in pubs {
            publish.write(|w| {
                w.set_chidx(ch);
                w.set_en(true);
            });
        }
        for subscribe in subs {
            subscribe.write(|w| {
                w.set_chidx(ch);
                w.set_en(true);
            });
        }
    }

    fn mask(&self, indices: &[usize]) -> u32 {
        indices
            .iter()
            .fold(0, |mask, &i| mask | (1 << self.radio_channels.number(i)))
    }

    fn activate(&mut self, indices: &[usize]) {
        self.active_mask = self.mask(indices);
        self.enable();
    }

    fn clear_all(&mut self) {
        self.clear();
        cortex_m::asm::dsb();
        if let Some(channel) = self.grtc_channel {
            pac::GRTC
                .publish_compare(channel)
                .write(|w| w.set_en(false));
        }
        self.peri_to_radio.disable();
        let radio = pac::RADIO;
        let timer = pac::TIMER10;
        for endpoint in [
            radio.subscribe_txen(),
            radio.subscribe_start(),
            radio.subscribe_rxen(),
            radio.subscribe_disable(),
            timer.subscribe_start(),
            timer.subscribe_stop(),
            timer.subscribe_clear(),
            timer.subscribe_capture(2),
        ] {
            endpoint.write(|w| w.set_en(false));
        }
        for endpoint in [
            radio.publish_disabled(),
            radio.publish_txready(),
            radio.publish_crcok(),
            radio.publish_crcerror(),
            radio.publish_rxaddress(),
            timer.publish_compare(0),
            timer.publish_compare(1),
        ] {
            endpoint.write(|w| w.set_en(false));
        }
        for &trigger in Trigger::ALL {
            let index = trigger.number();
            pac::EGU10
                .subscribe_trigger(index)
                .write(|w| w.set_en(false));
            pac::EGU10
                .publish_triggered(index)
                .write(|w| w.set_en(false));
            if index >= 2 {
                pac::EGU10.events_triggered(index).write_value(0);
            }
        }
    }

    fn clear_peripheral_next_rx(&mut self) {
        self.disable();
        cortex_m::asm::dsb();
    }

    fn clear_peripheral_tx_ack(&mut self) {
        self.disable();
        cortex_m::asm::dsb();
    }

    fn connect_grtc(&mut self, clock: &Nrf54Clock, tasks: &[Reg<shared::regs::Subscribe, RW>]) {
        let channel = clock.grtc_num();
        self.grtc_channel = Some(channel);
        Self::connect(
            self.peri_channels.channel(0),
            &[pac::GRTC.publish_compare(channel)],
            &[self.peri_to_radio.subscribe_send()],
        );
        Self::connect(
            self.radio_channels.channel(0),
            &[self.peri_to_radio.publish_receive()],
            tasks,
        );
    }
}

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
        self.error_group.clear();
        self.completion_group.clear();
        self.ack_group.clear();
        self.active_mask = 0;
    }

    fn enable(&mut self) {
        pac::DPPIC10
            .chenset()
            .write_value(pac::dppic::regs::Chen(self.active_mask));
        self.peri_channels.enable();
    }

    fn disable(&mut self) {
        self.radio_channels.disable();
        self.peri_channels.disable();
    }

    fn configure_central(&mut self, clock: &mut Nrf54Clock, egu: &mut Nrf54Egu) {
        self.clear_all();
        let radio = pac::RADIO;
        radio.shorts().modify(|w| {
            w.set_ready_start(true);
            w.set_rxready_start(false);
        });
        let timer = pac::TIMER10;
        let egu = egu.regs();
        let trigger = Trigger::TimeslotEnd.number();
        self.group.include(self.radio_channels.number(1));
        // Central reuses the second group for one-shot completion. Both
        // CRCOK and timeout publish on channel 2, so one disable gates both.
        self.error_group.include(self.radio_channels.number(2));
        self.connect_grtc(
            clock,
            &[
                radio.subscribe_txen(),
                timer.subscribe_start(),
                self.group.subscribe_en(),
                self.error_group.subscribe_en(),
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
                self.error_group.subscribe_dis(),
            ],
        );
        self.error_group.disable();
        self.activate(&[0, 1]);
    }

    fn configure_sync(&mut self, clock: &mut Nrf54Clock, egu: &mut Nrf54Egu) {
        self.clear_all();
        let radio = pac::RADIO;
        radio.shorts().modify(|w| {
            w.set_ready_start(true);
            w.set_rxready_start(false);
        });
        let timer = pac::TIMER10;
        let egu = egu.regs();
        self.connect_grtc(
            clock,
            &[egu.subscribe_trigger(Trigger::Rx(Rx::Rxen).number())],
        );
        Self::connect(
            self.radio_channels.channel(4),
            &[egu.publish_triggered(Trigger::Rx(Rx::Rxen).number())],
            &[timer.subscribe_start(), radio.subscribe_rxen()],
        );
        Self::connect(
            self.radio_channels.channel(1),
            &[radio.publish_rxaddress()],
            &[timer.subscribe_capture(2)],
        );
        Self::connect(
            self.radio_channels.channel(2),
            &[timer.publish_compare(1)],
            &[radio.subscribe_disable()],
        );
        Self::connect(
            self.radio_channels.channel(3),
            &[radio.publish_disabled()],
            &[egu.subscribe_trigger(Trigger::TimeslotEnd.number())],
        );
        self.activate(&[0, 1, 2, 3, 4]);
    }

    fn configure_rx(&mut self, clock: &mut Nrf54Clock, _egu: &mut Nrf54Egu) {
        self.clear_all();
        let radio = pac::RADIO;
        let timer = pac::TIMER10;
        let egu = pac::EGU10;
        radio.shorts().modify(|w| {
            w.set_ready_start(false);
            w.set_rxready_start(true);
        });
        self.group.set_channels(&[
            self.radio_channels.number(1),
            self.radio_channels.number(2),
            self.radio_channels.number(5),
        ]);
        self.error_group.set_channels(&[
            self.radio_channels.number(1),
            self.radio_channels.number(2),
            self.radio_channels.number(5),
        ]);
        self.completion_group
            .set_channels(&[self.radio_channels.number(2), self.radio_channels.number(3)]);
        self.ack_group
            .set_channels(&[self.radio_channels.number(8)]);

        // 0: one GRTC publisher fans out to separate, state-gated EGU outputs.
        self.connect_grtc(
            clock,
            &[
                egu.subscribe_trigger(Trigger::PrepareRx(PrepareRx::Wake).number()),
                egu.subscribe_trigger(Trigger::Rx(Rx::Rxen).number()),
            ],
        );
        Self::connect(
            self.radio_channels.channel(1),
            &[radio.publish_rxaddress()],
            &[timer.subscribe_capture(2)],
        );
        // Errors must disarm automatic TXEN before forwarding completion.
        Self::connect(
            self.radio_channels.channel(2),
            &[timer.publish_compare(1), radio.publish_crcerror()],
            &[
                self.error_group.subscribe_dis(),
                egu.subscribe_trigger(Trigger::Rx(Rx::Failure).number()),
            ],
        );
        Self::connect(
            self.radio_channels.channel(3),
            &[
                radio.publish_crcok(),
                egu.publish_triggered(Trigger::Rx(Rx::Failure).number()),
            ],
            &[
                radio.subscribe_disable(),
                egu.subscribe_trigger(Trigger::Rx(Rx::Done).number()),
                self.completion_group.subscribe_dis(),
            ],
        );
        // DISABLED has one publisher; EGU splits it into RX and ACK routes.
        Self::connect(
            self.radio_channels.channel(4),
            &[radio.publish_disabled()],
            &[
                egu.subscribe_trigger(Trigger::Rx(Rx::Disabled).number()),
                egu.subscribe_trigger(Trigger::TxAck(TxAck::Disabled).number()),
            ],
        );
        Self::connect(
            self.radio_channels.channel(5),
            &[egu.publish_triggered(Trigger::Rx(Rx::Disabled).number())],
            &[radio.subscribe_txen(), self.group.subscribe_dis()],
        );
        Self::connect(
            self.radio_channels.channel(6),
            &[egu.publish_triggered(Trigger::TxAck(TxAck::Disabled).number())],
            &[egu.subscribe_trigger(Trigger::TxAck(TxAck::Done).number())],
        );
        // Both completions reach the SAME TimeslotEnd subscriber.
        Self::connect(
            self.radio_channels.channel(7),
            &[
                egu.publish_triggered(Trigger::Rx(Rx::Done).number()),
                egu.publish_triggered(Trigger::TxAck(TxAck::Done).number()),
            ],
            &[egu.subscribe_trigger(Trigger::TimeslotEnd.number())],
        );
        Self::connect(
            self.radio_channels.channel(8),
            &[
                radio.publish_txready(),
                egu.publish_triggered(Trigger::TxAck(TxAck::Start).number()),
            ],
            &[radio.subscribe_start(), self.ack_group.subscribe_dis()],
        );
        Self::connect(
            self.radio_channels.channel(9),
            &[egu.publish_triggered(Trigger::PrepareRx(PrepareRx::Wake).number())],
            &[egu.subscribe_trigger(Trigger::Start.number())],
        );
        Self::connect(
            self.radio_channels.channel(10),
            &[egu.publish_triggered(Trigger::Rx(Rx::Rxen).number())],
            &[
                radio.subscribe_rxen(),
                timer.subscribe_start(),
                self.group.subscribe_en(),
                self.error_group.subscribe_en(),
                self.completion_group.subscribe_en(),
            ],
        );
    }

    fn configure_peripheral_prepare_rx(&mut self, _clock: &mut Nrf54Clock, _egu: &mut Nrf54Egu) {
        self.clear_peripheral_tx_ack();
        self.activate(&[0, 9]);
    }

    fn configure_peripheral_next_rx(&mut self, _clock: &mut Nrf54Clock, _egu: &mut Nrf54Egu) {
        self.clear_peripheral_tx_ack();
        // Completion stays disarmed until the actual RX-start pulse enables
        // groups. Old CRCOK/timeout events cannot complete a future window.
        self.activate(&[0, 4, 7, 10]);
    }

    fn configure_peripheral_tx_ack(&mut self, _egu: &mut Nrf54Egu) {
        self.clear_peripheral_next_rx();
        // Completion must be armed before any START can transmit the ACK.
        self.activate(&[4, 6, 7]);
        // Make ACK data/PACKETPTR writes visible before enabling DMA START.
        cortex_m::asm::dsb();
        pac::DPPIC10
            .chenset()
            .write_value(pac::dppic::regs::Chen(self.mask(&[8])));
        self.active_mask |= self.mask(&[8]);
        // READY events are not replayed. A fallback uses the same one-shot
        // START channel, so READY racing this check cannot start TX twice.
        if pac::RADIO.state().read().state() == pac::radio::vals::State::TxIdle {
            Nrf54Egu::trigger_regs(&pac::EGU10, Trigger::TxAck(TxAck::Start));
        }
    }
}
