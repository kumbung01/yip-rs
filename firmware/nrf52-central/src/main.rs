#![no_std]
#![no_main]
use defmt_rtt as _;
use embassy_futures::select::{Either, select};

use embassy_nrf::bind_interrupts;
use embassy_nrf::config::{Config, HfclkSource, LfclkSource};
use embassy_nrf::gpio::{Input, Pull};

use embassy_sync::channel::Channel;
use yip::DeviceContext;
use yip::YipSession;

use yip::ppi_channels;
use yip::{YipReader, YipWriter};

use embassy_executor::Spawner;

use yip::central::YipInterruptHandler;
const LINKS: usize = 2;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    config.hfclk_source = HfclkSource::ExternalXtal;
    config.lfclk_source = LfclkSource::ExternalXtal;

    let p = embassy_nrf::init(config);

    let seed = 0;

    bind_interrupts!(
        struct Irqs {
            EGU2_SWI2 => YipInterruptHandler;
        }
    );

    let device = DeviceContext::new(
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
    )
    .with_central_completion_group(p.PPI_GROUP1);

    let sessions = yip::central::init::<LINKS>(device);

    // nRF52840 DK button pins, active-low with pull-ups.
    let b1 = Input::new(p.P0_11, Pull::Up);
    let b2 = Input::new(p.P0_12, Pull::Up);
    let b3 = Input::new(p.P0_24, Pull::Up);
    let b4 = Input::new(p.P0_25, Pull::Up);

    spawner.spawn(gpio_task(b1, b2, b3, b4).unwrap());

    defmt::info!("nRF52 central initialized: {} links", LINKS);

    for (id, session) in sessions.into_iter().enumerate() {
        spawner.spawn(session_task(session, id).unwrap());
    }

    yip::central::start();
}

#[embassy_executor::task(pool_size = LINKS)]
async fn session_task(session: YipSession, id: usize) {
    defmt::info!("session {} start", id);
    let (mut rx, mut tx, conn) = session.split();

    loop {
        conn.wait_connected().await;
        defmt::info!("id {} connected", id);

        match select(
            conn.wait_disconnected(),
            process_message(&mut rx, &mut tx, id),
        )
        .await
        {
            Either::First(_) => {
                defmt::info!("id {} disconnected", id);
            }
            Either::Second(()) => {
                defmt::info!("id {} rx message", id);
            }
        }
    }
}

async fn process_message(rx: &mut YipReader, tx: &mut YipWriter, id: usize) {
    loop {
        let button = async {
            if id == 0 {
                COUNTS.receive().await
            } else {
                core::future::pending().await
            }
        };

        match select(button, rx.read()).await {
            Either::First(pressed) => {
                let mut arr: [u8; 200] = [0; 200];
                let bytes = pressed.to_le_bytes();
                arr[..bytes.len()].copy_from_slice(&bytes);

                let _ = tx.try_write(arr.as_slice());
            }
            Either::Second(_packet) => {
                // let bt = u32::from_le_bytes(packet.as_slice()[..4].try_into().unwrap());
                // defmt::info!("id {}: 버튼 {} 눌림", id, bt);
                // defmt::info!("id: {}, data: {}", id, _packet.as_slice());
            }
        }
    }
}

use embassy_futures::select::{Either4, select4};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

pub static COUNTS: Channel<CriticalSectionRawMutex, u32, 16> = Channel::new();

#[embassy_executor::task]
async fn gpio_task(
    mut b1: Input<'static>,
    mut b2: Input<'static>,
    mut b3: Input<'static>,
    mut b4: Input<'static>,
) {
    loop {
        let id = match select4(
            b1.wait_for_low(),
            b2.wait_for_low(),
            b3.wait_for_low(),
            b4.wait_for_low(),
        )
        .await
        {
            Either4::First(_) => 1,
            Either4::Second(_) => 2,
            Either4::Third(_) => 3,
            Either4::Fourth(_) => 4,
        };

        defmt::info!("버튼 {} 눌림", id);
        // 여기서 Channel로 id 전달 가능
        if COUNTS.try_send(id).is_err() {
            defmt::debug!("send {} failed!", id);
        }

        // 누르고 있는 동안 반복 처리되지 않도록 해제 대기
        match id {
            1 => b1.wait_for_high().await,
            2 => b2.wait_for_high().await,
            3 => b3.wait_for_high().await,
            4 => b4.wait_for_high().await,
            _ => unreachable!(),
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    defmt::error!("panic: {}", defmt::Display2Format(info));
    loop {
        core::hint::spin_loop();
    }
}
