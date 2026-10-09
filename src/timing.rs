use crate::config::*;

#[derive(Debug)]
pub(crate) struct ScheduledSlot {
    pub(crate) slot_diff: u32,
    pub(crate) rtc_cc: u32,
    pub(crate) timer_cc: u32,
}

#[derive(Default)]
pub(crate) struct SlotClock {
    // 이전 슬롯 경계의 RTC 정수부
    pub(crate) rtc_tick: u32,

    // 이전 슬롯 경계의 소수부.
    // 단위는 TIMER tick × 32이며, 항상 15_625 미만.
    pub(crate) remainder: u32,
}

impl SlotClock {
    pub(crate) fn set(&mut self, timer_now: u32) {
        let offset_scaled =
            (i64::from(timer_now) - i64::from(ADDR_DELAY_TICKS)) * i64::from(TIMER_RTC_SCALE);

        let rtc_unit = i64::from(RTC_TICK_SCALED);
        let rtc_delta = offset_scaled.div_euclid(rtc_unit);
        let remainder = offset_scaled.rem_euclid(rtc_unit);

        self.rtc_tick =
            (i64::from(self.rtc_tick) + rtc_delta).rem_euclid(i64::from(RTC_MASK) + 1) as u32;

        self.remainder = remainder as u32;
    }

    pub(crate) fn next_pre_slot(&self, rtc_now: u32, slotsize_ticks: u32) -> u32 {}

    pub(crate) fn next_slot(&mut self, rtc_now: u32, slotsize_us: u32) -> ScheduledSlot {
        let slotsize_tick_scaled = us_to_timer_ticks(slotsize_us) as u32 * TIMER_RTC_SCALE;

        // RTC wrap을 고려한 정수 tick 거리
        let elapsed_rtc = rtc_now.wrapping_sub(self.rtc_tick) & RTC_MASK;

        // 이전 슬롯의 실제 경계는 rtc_tick + remainder이므로
        // 정수부에서 계산한 거리에서 remainder를 뺀다.
        //
        // margin이 RTC 3tick이라 remainder보다 항상 크므로
        // unsigned 뺄셈에서 underflow가 발생하지 않는다.
        let distance_scaled = u64::from(elapsed_rtc) * u64::from(RTC_TICK_SCALED)
            + u64::from(RTC_MARGIN_SCALED)
            - u64::from(self.remainder);

        // 보통의 짧은 거리에서는 u32 나눗셈 사용.
        // 오랫동안 슬롯을 건너뛰었을 때만 u64 나눗셈 사용.
        let slot_diff = match u32::try_from(distance_scaled) {
            Ok(distance) => distance.div_ceil(slotsize_tick_scaled),
            Err(_) => distance_scaled.div_ceil(u64::from(slotsize_tick_scaled)) as u32,
        };

        // 기존 소수부에 진행할 슬롯 시간을 더한다.
        let advance_scaled =
            u64::from(self.remainder) + u64::from(slot_diff) * u64::from(slotsize_tick_scaled);

        // RTC 정수 증가량과 새 소수부로 분리.
        // 보통은 이 나눗셈도 u32로 처리한다.
        let (rtc_delta, remainder) = match u32::try_from(advance_scaled) {
            Ok(value) => (value / RTC_TICK_SCALED, value % RTC_TICK_SCALED),
            Err(_) => (
                (advance_scaled / u64::from(RTC_TICK_SCALED)) as u32,
                (advance_scaled % u64::from(RTC_TICK_SCALED)) as u32,
            ),
        };

        self.rtc_tick = self.rtc_tick.wrapping_add(rtc_delta) & RTC_MASK;
        self.remainder = remainder;

        // 하드웨어에 출력할 때만 TIMER 정수 tick으로 반올림.
        // 저장된 remainder는 변경하지 않는다.
        let timer_cc = ((self.remainder + TIMER_RTC_SCALE / 2) / TIMER_RTC_SCALE).max(1);

        ScheduledSlot {
            slot_diff,
            rtc_cc: self.rtc_tick,
            timer_cc,
        }
    }

    pub(crate) fn next_watchdog_rtc(&self) -> u32 {
        let rtc_delta = (self.remainder + SLOTSIZE_TIMER_TICKS_SCALED) / RTC_TICK_SCALED;

        rtc_wrap(
            self.rtc_tick
                .wrapping_add(rtc_delta)
                .wrapping_sub(TIMESLOT_WDT_MARGIN_RTC_TICKS),
        )
    }
}
