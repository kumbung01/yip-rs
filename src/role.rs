use core::marker::Sized;
use embassy_nrf::interrupt::InterruptExt;

use crate::config::YIP_MAX_LINK_COUNT;
use crate::egu::YipTrigger;
use crate::isr::{self, InterruptTarget};
use crate::{enable_interrupt, trigger_start};

use crate::central::YipCentral;
use crate::device::YipDevice;
use crate::link::YipLink;
use crate::packet::YipLinkEndpoints;
use crate::packetbuffer::YipPacketBuffer;
use crate::peripheral::YipPeripheral;

pub(crate) trait YipRole {}

pub(crate) trait YipRoleStack<'q, 'p, const W: usize>: Sized + YipRole {
    fn new() -> Self;
    fn setup(&mut self, device: &mut YipDevice, id: u8);
    fn on_interrupt(
        &mut self,
        device: &mut YipDevice,
        links: &mut [YipLink<'q, 'p, W>],
        trigger: YipTrigger,
        rx_buffer: &mut YipPacketBuffer<Self>,
        tx_buffer: &mut YipPacketBuffer<Self>,
    );
}

pub struct YipStack<'q, 'p, const W: usize, R> {
    device: YipDevice,
    links: [YipLink<'q, 'p, W>; W],
    role: R,
    rx_buffer: YipPacketBuffer<R>,
    tx_buffer: YipPacketBuffer<R>,
}

#[allow(private_bounds)]
impl<'q, 'p, const W: usize, R> YipStack<'q, 'p, W, R>
where
    R: YipRoleStack<'q, 'p, W>,
{
    // pub fn new(endpoints: [YipLinkEndpoints<'q, 'p, W>; W], device: YipDevice) -> Self {
    //     Self {
    //         device: device,
    //         links: endpoints.map(YipLink::new),
    //         role: R::new(),
    //     }
    // }

    pub(crate) fn on_interrupt(&mut self) {
        let egu = &mut self.device.egu;

        let Some(trigger) = egu.get_enabled() else {
            return;
        };

        if !egu.is_triggered(trigger) {
            return;
        }

        egu.disable_interrupt(trigger);
        egu.clear_triggered(trigger);

        let Self {
            device,
            links,
            role,
            rx_buffer,
            tx_buffer,
        } = self;
        role.on_interrupt(device, links, trigger, rx_buffer, tx_buffer);
    }

    pub fn setup(&mut self, id: u8) {
        self.device.radio.init();
        self.role.setup(&mut self.device, id);

        defmt::debug!("stack setup complete");
    }
}

impl<'q, 'p, const W: usize> YipStack<'q, 'p, W, YipCentral> {
    pub fn new(endpoints: [YipLinkEndpoints<'q, 'p, W>; W], device: YipDevice) -> Self {
        const {
            assert!(W > 0, "link count must be greater than 0");
            assert!(W <= YIP_MAX_LINK_COUNT, "link size too big");
        }

        Self {
            device: device,
            links: endpoints.map(YipLink::new),
            role: <YipCentral as YipRoleStack<'q, 'p, W>>::new(),
            rx_buffer: YipPacketBuffer::<YipCentral>::new(),
            tx_buffer: YipPacketBuffer::<YipCentral>::new(),
        }
    }
}

impl<const W: usize> YipStack<'static, 'static, W, YipCentral> {
    pub fn register_and_enable_irq(&'static mut self) {
        let irq = self.device.egu.irq();

        isr::register(self);
        enable_interrupt();

        defmt::debug!("IRQ registered");

        unsafe {
            // SAFETY: 생성자의 Binding으로 벡터를 검증했고,
            // stack 초기화와 ISR 등록이 완료됨.
            irq.enable();
        }

        trigger_start();
    }
}

impl<'q, 'p> YipStack<'q, 'p, 1, YipPeripheral> {
    pub fn new(endpoints: [YipLinkEndpoints<'q, 'p, 1>; 1], device: YipDevice) -> Self {
        Self {
            device: device,
            links: endpoints.map(YipLink::new),
            role: <YipPeripheral as YipRoleStack<'q, 'p, 1>>::new(),
            rx_buffer: YipPacketBuffer::<YipPeripheral>::new(),
            tx_buffer: YipPacketBuffer::<YipPeripheral>::new(),
        }
    }
}

impl YipStack<'static, 'static, 1, YipPeripheral> {
    pub fn register_and_enable_irq(&'static mut self) {
        let irq = self.device.egu.irq();

        isr::register(self);
        enable_interrupt();

        unsafe {
            // SAFETY: 생성자의 Binding으로 벡터를 검증했고,
            // stack 초기화와 ISR 등록이 완료됨.
            irq.enable();
        }
    }
}

impl<const N: usize, R> InterruptTarget for YipStack<'static, 'static, N, R>
where
    R: YipRoleStack<'static, 'static, N>,
    YipStack<'static, 'static, N, R>: Send,
{
    fn on_interrupt(&mut self) {
        YipStack::on_interrupt(self);
    }
}
