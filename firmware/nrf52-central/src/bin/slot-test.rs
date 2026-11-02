#![no_std]
#![no_main]
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::bind_interrupts;
use yip::{DeviceContext, YipSession, central::YipInterruptHandler, ppi_channels};
const PIPES: usize = 2;
bind_interrupts!(struct Irqs { EGU2_SWI2 => YipInterruptHandler; });
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = embassy_nrf::config::Config::default();
    config.hfclk_source = embassy_nrf::config::HfclkSource::ExternalXtal;
    config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    let p = embassy_nrf::init(config);
    let device = DeviceContext::new(
        p.RADIO,
        p.TIMER2,
        p.RTC2,
        ppi_channels![
            p.PPI_CH0, p.PPI_CH1, p.PPI_CH2, p.PPI_CH3, p.PPI_CH4, p.PPI_CH5, p.PPI_CH6, p.PPI_CH7
        ],
        p.PPI_GROUP0,
        p.EGU2,
        Irqs,
        0,
    )
    .with_central_completion_group(p.PPI_GROUP1);
    for session in yip::central::init::<PIPES>(device) {
        spawner.spawn(reader(session).unwrap());
    }
    yip::central::start();
}
#[embassy_executor::task(pool_size = PIPES)]
async fn reader(session: YipSession) {
    let (rx, _, _) = session.split();
    loop {
        let _ = rx.read().await;
    }
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("{}", defmt::Display2Format(info));
    loop {
        core::hint::spin_loop();
    }
}
