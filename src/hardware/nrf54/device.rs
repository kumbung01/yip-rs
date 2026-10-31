use crate::hardware::api::{EguDriver, RadioDriver};
use crate::hardware::clock::{GrtcChannel, Nrf54Clock};
use crate::hardware::context::SlotContext;
use crate::hardware::egu::Nrf54Egu;
use crate::hardware::ppi::{
    Channel, Group as PpiGroup, Nrf54Ppi, PeriDomain, RADIO_CHANNEL_COUNT, Radio as RadioDomain,
};
use crate::hardware::radio::Nrf54Radio;
use crate::register_egu;

use crate::hardware::ppi::BridgeChannel;
use core::marker::PhantomData;
use embassy_nrf::{
    Peri,
    interrupt::typelevel::{Binding, EGU10, Handler},
    peripherals::RADIO,
};

/// Hardware resources with a validated interrupt binding to handler `H`.
pub struct DeviceContext<H> {
    pub(crate) radio: Nrf54Radio,
    pub(crate) clock: Nrf54Clock,
    pub(crate) ppi: Nrf54Ppi,
    pub(crate) egu: Nrf54Egu,
    pub(crate) context: SlotContext,
    _handler: PhantomData<H>,
}

impl<H> DeviceContext<H> {
    pub fn new<R, B>(
        radio: Peri<'static, RADIO>,
        timer: Peri<'static, embassy_nrf::peripherals::TIMER10>,
        grtc: Peri<'static, R>,
        radio_channels: [Channel<'static, RadioDomain>; RADIO_CHANNEL_COUNT],
        peri_channels: [Channel<'static, PeriDomain>; 1],
        bridge: (Peri<'static, B>, Peri<'static, B::Paired>),
        groups: [PpiGroup<'static, RadioDomain>; 4],
        _irq: impl Binding<EGU10, H>,
        seed: u32,
    ) -> Self
    where
        R: GrtcChannel,
        B: BridgeChannel,
        H: Handler<EGU10>,
    {
        let context = SlotContext {
            refslot: seed,
            ..SlotContext::default()
        };

        // we set egu register here for triggering from session.
        let egu = Nrf54Egu::new();
        register_egu(egu.regs());

        let mut radio = Nrf54Radio::new(radio);
        radio.init();

        Self {
            radio,
            clock: Nrf54Clock::new(grtc, timer),
            ppi: Nrf54Ppi::new(radio_channels, peri_channels, bridge, groups),
            egu,
            context,
            _handler: PhantomData,
        }
    }
}

impl<H> DeviceContext<H> {
    /// Measure the effective GRTC/HF rate before handing the device to a role.
    ///
    /// The executor must be allowed to sleep during this approximately 33 ms
    /// measurement. A busy-polling task or a policy that forces GRTC active
    /// measures the active-clock rate instead of its idle-clock rate. Only
    /// this device's CC, TIMER and DPPI resources are changed; Embassy's
    /// global GRTC configuration and time-driver CC remain untouched.
    /// Returns false on a missed capture or an implausible measurement.
    pub async fn calibrate_clock(&mut self) -> bool {
        use crate::hardware::clock::ratio::CALIBRATION_US;
        use embassy_time::{Duration, Instant, Timer};

        // Cleanup also runs if this future is cancelled before completion.
        struct Calibration<'a> {
            clock: &'a mut Nrf54Clock,
            ppi: &'a mut Nrf54Ppi,
            shorts: embassy_nrf::pac::timer::regs::Shorts,
        }
        impl Drop for Calibration<'_> {
            fn drop(&mut self) {
                self.ppi.finish_calibration();
                self.clock.finish_calibration(0);
                let timer = self.clock.timer.regs();
                timer.tasks_stop().write_value(1);
                timer.tasks_clear().write_value(1);
                timer.prescaler().write(|w| w.set_prescaler(5));
                timer.shorts().write_value(self.shorts);
            }
        }

        let timer = self.clock.timer.regs();
        let measurement = Calibration {
            shorts: timer.shorts().read(),
            clock: &mut self.clock,
            ppi: &mut self.ppi,
        };
        timer.tasks_stop().write_value(1);
        timer.tasks_clear().write_value(1);
        timer.shorts().write(|_| ());
        timer.prescaler().write(|w| w.set_prescaler(1)); // 32 MHz / 2
        timer.cc(2).write_value(0);
        timer.cc(3).write_value(0);
        measurement.ppi.calibration_capture(measurement.clock, 2);
        timer.tasks_start().write_value(1);

        let first = Instant::now() + Duration::from_micros(1000);
        measurement.clock.set_calibration_compare(first.as_micros());
        Timer::at(first + Duration::from_micros(500)).await;
        let start = timer.cc(2).read();
        let end_at = first + Duration::from_micros(CALIBRATION_US as u64);
        if start == 0 || Instant::now() >= end_at {
            return false;
        }
        measurement.ppi.calibration_capture(measurement.clock, 3);
        measurement
            .clock
            .set_calibration_compare(end_at.as_micros());
        Timer::at(end_at + Duration::from_micros(500)).await;
        let end = timer.cc(3).read();
        let ticks = end.wrapping_sub(start);
        end != 0 && measurement.clock.finish_calibration(ticks)
    }

    pub fn set_pipes(&mut self, pipes: usize) {
        debug_assert!(pipes > 0, "pipes must be bigger than 0");

        self.context.pipes = pipes;
    }
}
