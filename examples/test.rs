#![no_std]
#![no_main]
use defmt_rtt as _;
use embassy_futures::select::{Either, select};

use embassy_nrf::bind_interrupts;
use yip::YipDevice;
use yip::YipInterruptHandler;
use yip::YipSession;
use yip::ppi_channels;
use yip::random_u32;
use yip::{YipReader, YipWriter};

use embassy_executor::Spawner;

const POOL_SIZE: usize = 16;
const LINKS: usize = 2;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut p = embassy_nrf::init(Default::default());

    let seed = random_u32(&mut p.RNG);

    bind_interrupts!(struct Irqs {
        EGU2_SWI2 => YipInterruptHandler;
    });

    let device = YipDevice::new(
        p.RADIO,
        p.TIMER2,
        p.RTC2,
        ppi_channels![
            p.PPI_CH0, p.PPI_CH1, p.PPI_CH2, p.PPI_CH3, p.PPI_CH4, p.PPI_CH5, p.PPI_CH6, p.PPI_CH7,
        ],
        p.PPI_GROUP0,
        p.EGU2,
        Irqs,
        seed,
    );

    let sessions = yip::init_yip! {
        device: device,
        pool_size: POOL_SIZE,
        links: LINKS,
        role: yip::YipCentral,
    };

    defmt::debug!("Yip initialized");

    for session in sessions {
        spawner.spawn(session_task(session).unwrap());
    }
}

#[embassy_executor::task(pool_size = LINKS)]
async fn session_task(session: YipSession<'static, 'static, LINKS>) {
    let (mut rx, mut tx, conn) = session.split();

    loop {
        conn.wait_connected().await;

        match select(conn.wait_disconnected(), process_message(&mut rx, &mut tx)).await {
            Either::First(_) => {}
            Either::Second(()) => {}
        }
    }
}

async fn process_message<'q, 'p, const W: usize>(
    rx: &mut YipReader<'q, 'p, W>,
    _tx: &mut YipWriter<'q, 'p, W>,
) {
    loop {
        let packet = rx.read().await;
        drop(packet);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
