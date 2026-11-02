#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_nrf::bind_interrupts;
use yip::peripheral::YipInterruptHandler;
use yip::{DeviceContext, YipSession, ppi_channels};

const PIPE: u8 = cfg!(feature = "pipe1") as u8;

bind_interrupts!(struct Irqs {
    EGU2_SWI2 => YipInterruptHandler;
});

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
    );
    spawner.spawn(traffic(yip::peripheral::init(device, PIPE)).unwrap());
    yip::peripheral::start();
}

#[embassy_executor::task]
async fn traffic(session: YipSession) {
    let (_rx, mut tx, conn) = session.split();
    let mut seq = 0u32;
    loop {
        conn.wait_connected().await;
        let fill = async {
            loop {
                let mut payload = [0; 8];
                payload[..4].copy_from_slice(&seq.to_le_bytes());
                payload[4..].copy_from_slice(
                    &(embassy_time::Instant::now().as_micros() as u32).to_le_bytes(),
                );
                if tx.write(&payload).await.is_err() {
                    break;
                }
                seq = seq.wrapping_add(1);
            }
        };
        select(conn.wait_disconnected(), fill).await;
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
