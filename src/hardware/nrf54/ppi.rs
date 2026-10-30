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

// EGU10 and TIMER10 live in the RADIO DPPI domain. GRTC is bridged from PERI.
// Helper EGU event 2 forwards errors to the completion channel after ACK gating
// has been disabled. Its interrupt is never enabled (only Start/TimeslotEnd are).
const RX_FAILURE: usize = 2;
// Software READY fallback shares the same one-shot START route as TXREADY.
// Neither helper event enables an EGU interrupt.
const ACK_START: usize = 3;
pub(crate) const RADIO_CHANNEL_COUNT: usize = 5;

pub(crate) struct Nrf54Ppi {
    radio_channels: Channels<'static, Radio, RADIO_CHANNEL_COUNT>,
    peri_channels: Channels<'static, PeriDomain, 1>,
    peri_to_radio: Ppib<'static>,
    group: Group<'static, Radio>,
    error_group: Group<'static, Radio>,
    grtc_channel: Option<usize>,
    central_completion_mask: u32,
}

impl Nrf54Ppi {
    pub(crate) fn new<S: BridgeChannel>(
        radio_channels: [Channel<'static, Radio>; RADIO_CHANNEL_COUNT],
        peri_channels: [Channel<'static, PeriDomain>; 1],
        peri_to_radio: (Peri<'static, S>, Peri<'static, S::Paired>),
        groups: [Group<'static, Radio>; 2],
    ) -> Self {
        let [group, error_group] = groups;
        let mut this = Self {
            radio_channels: Channels::new(radio_channels),
            peri_channels: Channels::new(peri_channels),
            peri_to_radio: Ppib::new(peri_to_radio.0, peri_to_radio.1),
            group,
            error_group,
            grtc_channel: None,
            central_completion_mask: 0,
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
        let grtc = pac::GRTC;
        let radio = pac::RADIO;
        let timer = pac::TIMER10;
        let egu = pac::EGU10;
        if let Some(channel) = self.grtc_channel {
            // Never touch Embassy's reserved GRTC CC11, or an unowned CC0.
            grtc.publish_compare(channel).write(|w| w.set_en(false));
        }
        self.peri_to_radio.disable();
        for endpoint in [
            radio.subscribe_txen(),
            radio.subscribe_start(),
            radio.subscribe_rxen(),
            radio.subscribe_disable(),
            timer.subscribe_start(),
            timer.subscribe_stop(),
            timer.subscribe_clear(),
            timer.subscribe_capture(2),
            timer.subscribe_capture(3),
            timer.subscribe_capture(4),
            egu.subscribe_trigger(Trigger::Start as usize),
            egu.subscribe_trigger(Trigger::TimeslotEnd as usize),
            egu.subscribe_trigger(RX_FAILURE),
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
            egu.publish_triggered(RX_FAILURE),
            egu.publish_triggered(ACK_START),
        ] {
            endpoint.write(|w| w.set_en(false));
        }
        egu.events_triggered(RX_FAILURE).write_value(0);
        egu.events_triggered(ACK_START).write_value(0);
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
        self.error_group.clear();
        self.central_completion_mask = 0;
    }

    fn enable(&mut self) {
        // The slot-start event, rather than ISR preparation, arms completion.
        self.radio_channels.enable_except(self.central_completion_mask);
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
        let trigger = Trigger::TimeslotEnd as usize;
        self.group.include(self.radio_channels.number(1));
        // Central reuses the second group for one-shot completion. Both
        // CRCOK and timeout publish on channel 2, so one disable gates both.
        self.error_group.include(self.radio_channels.number(2));
        self.central_completion_mask = 1u32 << self.radio_channels.number(2);
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
        // GRTC already has microsecond resolution: start RX and its relative
        // TIMER together instead of the nRF52 RTC + fractional TIMER CC0 split.
        self.connect_grtc(clock, &[timer.subscribe_start(), radio.subscribe_rxen()]);
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
            &[
                egu.subscribe_trigger(Trigger::TimeslotEnd as usize),
                timer.subscribe_stop(),
                timer.subscribe_clear(),
            ],
        );
    }

    fn configure_peripheral_prepare_rx(&mut self, clock: &mut Nrf54Clock, egu: &mut Nrf54Egu) {
        self.clear_all();
        self.connect_grtc(
            clock,
            &[egu.regs().subscribe_trigger(Trigger::Start as usize)],
        );
    }

    fn configure_peripheral_next_rx(&mut self, clock: &mut Nrf54Clock, egu: &mut Nrf54Egu) {
        self.clear_all();
        let radio = pac::RADIO;
        // RX starts automatically. ACK TX ramps up in parallel, then stays
        // idle until the role has installed the ACK buffer and address.
        radio.shorts().modify(|w| {
            w.set_ready_start(false);
            w.set_rxready_start(true);
        });
        let timer = pac::TIMER10;
        let egu = egu.regs();
        // Capture and automatic ACK are valid for exactly one receive window.
        self.group.include(self.radio_channels.number(1));
        self.group.include(self.radio_channels.number(4));
        self.error_group.include(self.radio_channels.number(1));
        self.error_group.include(self.radio_channels.number(4));
        // Without STOP/CLEAR at completion, disable the timeout route at RX
        // DISABLED so it cannot interrupt the CPU while preparing the ACK.
        #[cfg(feature = "defmt")]
        {
            self.group.include(self.radio_channels.number(2));
            self.error_group.include(self.radio_channels.number(2));
        }
        self.connect_grtc(
            clock,
            &[
                timer.subscribe_start(),
                radio.subscribe_rxen(),
                self.group.subscribe_en(),
                self.error_group.subscribe_en(),
            ],
        );
        Self::connect(
            self.radio_channels.channel(1),
            &[radio.publish_rxaddress()],
            &[timer.subscribe_capture(2)],
        );
        // Errors disable auto-ACK before forwarding completion. A task can
        // subscribe to only one DPPI channel, hence the intermediate EGU event.
        Self::connect(
            self.radio_channels.channel(2),
            &[timer.publish_compare(1), radio.publish_crcerror()],
            &[
                self.error_group.subscribe_dis(),
                egu.subscribe_trigger(RX_FAILURE),
            ],
        );
        let completion_tasks = &[
            egu.subscribe_trigger(Trigger::TimeslotEnd as usize),
            radio.subscribe_disable(),
            #[cfg(feature = "defmt")]
            timer.subscribe_capture(3),
            #[cfg(not(feature = "defmt"))]
            timer.subscribe_stop(),
            #[cfg(not(feature = "defmt"))]
            timer.subscribe_clear(),
        ];
        Self::connect(
            self.radio_channels.channel(3),
            &[radio.publish_crcok(), egu.publish_triggered(RX_FAILURE)],
            completion_tasks,
        );
        Self::connect(
            self.radio_channels.channel(4),
            &[radio.publish_disabled()],
            &[
                radio.subscribe_txen(),
                self.group.subscribe_dis(),
                #[cfg(feature = "defmt")]
                timer.subscribe_capture(4),
            ],
        );
        // Separate groups let timeout/error and successful DISABLED events
        // each disable the same one-shot channels, without resubscribing DIS.
    }

    fn configure_peripheral_tx_ack(&mut self, egu: &mut Nrf54Egu) {
        self.clear_all();
        // Diagnostic captures are complete; no need to run TIMER during TX.
        #[cfg(feature = "defmt")]
        {
            pac::TIMER10.tasks_stop().write_value(1);
            pac::TIMER10.tasks_clear().write_value(1);
        }
        Self::connect(
            self.radio_channels.channel(0),
            &[pac::RADIO.publish_disabled()],
            &[egu.regs().subscribe_trigger(Trigger::TimeslotEnd as usize)],
        );

        // Install completion first. Only after all ACK data/register writes
        // are visible may TXREADY start DMA.
        cortex_m::asm::dsb();
        self.group.include(self.radio_channels.number(1));
        Self::connect(
            self.radio_channels.channel(1),
            &[
                pac::RADIO.publish_txready(),
                pac::EGU10.publish_triggered(ACK_START),
            ],
            &[pac::RADIO.subscribe_start(), self.group.subscribe_dis()],
        );
        // DPPI does not replay a READY event that preceded connection. Send
        // the fallback through the SAME self-disabling channel, rather than
        // racing a direct CPU START against a hardware START.
        if pac::RADIO.state().read().state() == pac::radio::vals::State::TxIdle {
            pac::EGU10.tasks_trigger(ACK_START).write_value(1);
        }
    }
}
