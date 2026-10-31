use core::marker::PhantomData;

use embassy_nrf::{
    Peri, PeripheralType,
    pac::{
        self,
        common::{RW, Reg},
        dppic::{Dppic, SubscribeChg},
        shared::regs::{Publish, Subscribe},
    },
    peripherals,
};

mod sealed {
    pub trait Domain {}
    pub trait Channel {}
    pub trait Group {}
    pub trait Bridge {}
}

pub trait Domain: sealed::Domain {
    fn regs() -> pac::dppic::Dppic;
}

pub enum Mcu {}
pub enum Radio {}
pub enum PeriDomain {}
pub enum Lp {}

macro_rules! impl_domain {
    ($($domain:ident => $regs:ident),+ $(,)?) => {
        $(
            impl sealed::Domain for $domain {}
            impl Domain for $domain {
                fn regs() -> pac::dppic::Dppic { pac::$regs }
            }
        )+
    };
}

impl_domain!(Mcu => DPPIC00, Radio => DPPIC10, PeriDomain => DPPIC20, Lp => DPPIC30);

pub trait ChannelInstance: sealed::Channel + PeripheralType + 'static {
    type Domain: Domain;
    const NUMBER: usize;
}

pub trait GroupInstance: sealed::Group + PeripheralType + 'static {
    type Domain: Domain;
    const NUMBER: usize;
}

macro_rules! impl_resources {
    ($trait:ident, $sealed:ident, $domain:ident, $prefix:ident; $($n:literal),+ $(,)?) => {
        paste::paste! {
            $(
                impl sealed::$sealed for peripherals::[<$prefix $n>] {}
                impl $trait for peripherals::[<$prefix $n>] {
                    type Domain = $domain;
                    const NUMBER: usize = $n;
                }
            )+
        }
    };
}

impl_resources!(ChannelInstance, Channel, Mcu, PPI00_CH; 0, 1, 2, 3, 4, 5, 6, 7);
impl_resources!(ChannelInstance, Channel, Radio, PPI10_CH;
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23,
);
impl_resources!(ChannelInstance, Channel, PeriDomain, PPI20_CH;
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
);
impl_resources!(ChannelInstance, Channel, Lp, PPI30_CH; 0, 1, 2, 3);
impl_resources!(GroupInstance, Group, Mcu, PPI00_GROUP; 0, 1);
impl_resources!(GroupInstance, Group, Radio, PPI10_GROUP; 0, 1, 2, 3, 4, 5);
impl_resources!(GroupInstance, Group, PeriDomain, PPI20_GROUP; 0, 1, 2, 3, 4, 5);
impl_resources!(GroupInstance, Group, Lp, PPI30_GROUP; 0, 1);

/// An exclusively owned channel, with its peripheral type erased.
/// The lifetime keeps a reborrowed peripheral borrowed while this handle exists.
pub struct Channel<'d, D: Domain> {
    number: usize,
    _owner: PhantomData<&'d mut D>,
}

impl<'d, D: Domain> Channel<'d, D> {
    pub fn new<C: ChannelInstance<Domain = D>>(_channel: Peri<'d, C>) -> Self {
        Self {
            number: C::NUMBER,
            _owner: PhantomData,
        }
    }

    pub fn number(&self) -> usize {
        self.number
    }

    pub fn domain(&self) -> Dppic {
        D::regs()
    }
}

impl<'d, C: ChannelInstance> From<Peri<'d, C>> for Channel<'d, C::Domain> {
    fn from(channel: Peri<'d, C>) -> Self {
        Self::new(channel)
    }
}

/// Channel groups are erased separately so they cannot be used as channels.
pub struct Group<'d, D: Domain> {
    number: usize,
    _owner: PhantomData<&'d mut D>,
}

impl<'d, D: Domain> Group<'d, D> {
    pub fn new<G: GroupInstance<Domain = D>>(_group: Peri<'d, G>) -> Self {
        Self {
            number: G::NUMBER,
            _owner: PhantomData,
        }
    }

    /// # Safety
    /// 해당 그룹의 독점 사용권을 호출자가 보장해야 한다.
    pub unsafe fn from_num(group: usize) -> Self {
        Self {
            number: group,
            _owner: PhantomData,
        }
    }

    pub fn number(&self) -> usize {
        self.number
    }

    pub fn subscribe_chg(&self) -> SubscribeChg {
        D::regs().subscribe_chg(self.number())
    }

    pub fn subscribe_en(&self) -> Reg<Subscribe, RW> {
        self.subscribe_chg().en()
    }

    pub fn subscribe_dis(&self) -> Reg<Subscribe, RW> {
        self.subscribe_chg().dis()
    }

    pub fn include(&mut self, channel: usize) {
        D::regs()
            .chg(self.number())
            .modify(|w| w.set_ch(channel, true));
    }

    /// Replace the group mask with both EN/DIS subscriptions disconnected.
    pub fn set_channels(&mut self, channels: &[usize]) {
        D::regs().chg(self.number()).write(|w| {
            for &channel in channels {
                w.set_ch(channel, true);
            }
        });
    }

    pub fn enable(&mut self) {
        D::regs().tasks_chg(self.number()).en().write_value(1);
    }

    pub fn disable(&mut self) {
        D::regs().tasks_chg(self.number()).dis().write_value(1);
    }

    pub fn clear(&mut self) {
        self.subscribe_en().write(|w| w.set_en(false));
        self.subscribe_dis().write(|w| w.set_en(false));
        D::regs().chg(self.number()).write(|_| ());
    }
}

impl<'d, G: GroupInstance> From<Peri<'d, G>> for Group<'d, G::Domain> {
    fn from(group: Peri<'d, G>) -> Self {
        Self::new(group)
    }
}

pub struct Channels<'d, D: Domain, const N: usize> {
    channels: [Channel<'d, D>; N],
    mask: u32,
}

impl<'d, D: Domain, const N: usize> Channels<'d, D, N> {
    pub fn new(channels: [Channel<'d, D>; N]) -> Self {
        let mask = channels
            .iter()
            .fold(0, |mask, channel| mask | (1u32 << channel.number()));

        Self { channels, mask }
    }

    pub fn number(&self, index: usize) -> usize {
        self.channels[index].number()
    }

    pub fn enable(&mut self) {
        self.enable_except(0);
    }

    pub fn enable_except(&mut self, excluded: u32) {
        D::regs()
            .chenset()
            .write_value(pac::dppic::regs::Chen(self.mask & !excluded));
    }

    pub fn disable(&mut self) {
        D::regs()
            .chenclr()
            .write_value(pac::dppic::regs::Chen(self.mask));
    }

    pub fn channel(&self, index: usize) -> &Channel<'d, D> {
        &self.channels[index]
    }
}

/// One owned half of the fixed PERI/RADIO bridge pair.
pub trait BridgeChannel: sealed::Bridge + PeripheralType + 'static {
    type Paired: BridgeChannel;
    const NUMBER: usize;
    fn regs() -> pac::ppib::Ppib;
}

macro_rules! impl_bridge_pair {
    ($($n:literal),+ $(,)?) => {
        paste::paste! {
            $(
                impl sealed::Bridge for peripherals::[<PPIB21_CH $n>] {}
                impl BridgeChannel for peripherals::[<PPIB21_CH $n>] {
                    type Paired = peripherals::[<PPIB11_CH $n>];
                    const NUMBER: usize = $n;
                    fn regs() -> pac::ppib::Ppib { pac::PPIB21 }
                }
                impl sealed::Bridge for peripherals::[<PPIB11_CH $n>] {}
                impl BridgeChannel for peripherals::[<PPIB11_CH $n>] {
                    type Paired = peripherals::[<PPIB21_CH $n>];
                    const NUMBER: usize = $n;
                    fn regs() -> pac::ppib::Ppib { pac::PPIB11 }
                }
            )+
        }
    };
}
impl_bridge_pair!(0, 1, 2, 3, 4, 5, 6, 7);

/// Keep exclusive ownership of both halves without depending on a HAL PPIB driver.
pub(crate) struct Ppib<'d> {
    send: Reg<Subscribe, RW>,
    receive: Reg<Publish, RW>,
    _owner: PhantomData<&'d mut ()>,
}

impl<'d> Ppib<'d> {
    pub fn new<S: BridgeChannel>(_source: Peri<'d, S>, _sink: Peri<'d, S::Paired>) -> Self {
        Self {
            send: S::regs().subscribe_send(S::NUMBER),
            receive: S::Paired::regs().publish_receive(S::Paired::NUMBER),
            _owner: PhantomData,
        }
    }

    pub fn subscribe_send(&self) -> Reg<Subscribe, RW> {
        self.send
    }
    pub fn publish_receive(&self) -> Reg<Publish, RW> {
        self.receive
    }

    pub fn disable(&mut self) {
        self.receive.write(|w| w.set_en(false));
        self.send.write(|w| w.set_en(false));
    }
}
