use embassy_nrf::{Peri, peripherals::RNG};

pub fn random_u32(rng: &mut Peri<'_, RNG>) -> u32 {
    let mut rng = embassy_nrf::rng::Rng::new_blocking(rng.reborrow());
    rng.blocking_next_u32()
}
