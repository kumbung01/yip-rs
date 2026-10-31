//! Link and exercise the nRF54 peripheral backend with the central board's resources.
#![no_std]
#![no_main]

use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::{
    bind_interrupts,
    config::{Config, HfclkSource, LfclkSource},
};
use yip::peripheral::YipInterruptHandler;
use yip::{DeviceContext, ppi_channels};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = Config::default();
    config.clock_speed = embassy_nrf::config::ClockSpeed::CK128;
    config.hfclk_source = HfclkSource::ExternalXtal;
    config.lfclk_source = LfclkSource::ExternalXtal;
    let p = embassy_nrf::init(config);
    bind_interrupts!(struct Irqs { EGU10 => YipInterruptHandler; });
    let mut device = DeviceContext::new(
        p.RADIO,
        p.TIMER10,
        p.GRTC_CH0,
        ppi_channels![
            p.PPI10_CH0,
            p.PPI10_CH1,
            p.PPI10_CH2,
            p.PPI10_CH3,
            p.PPI10_CH4,
            p.PPI10_CH5,
            p.PPI10_CH6,
            p.PPI10_CH7,
            p.PPI10_CH8,
            p.PPI10_CH9,
            p.PPI10_CH10,
        ],
        ppi_channels![p.PPI20_CH0],
        (p.PPIB21_CH0, p.PPIB11_CH0),
        [
            core::convert::Into::into(p.PPI10_GROUP0),
            core::convert::Into::into(p.PPI10_GROUP1),
            core::convert::Into::into(p.PPI10_GROUP2),
            core::convert::Into::into(p.PPI10_GROUP3),
        ],
        Irqs,
        0,
    );
    let calibrated = device.calibrate_clock().await;
    debug_assert!(calibrated, "GRTC/HF calibration failed");

    let session = yip::peripheral::init(device, 0);
    let (reader, _writer, _connection) = session.split();
    yip::peripheral::start();
    loop {
        let packet = reader.read().await;
        defmt::info!("nRF54 peripheral RX: {}", packet.as_slice());
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    defmt::error!("panic: {}", defmt::Display2Format(info));
    loop {
        core::hint::spin_loop();
    }
}
