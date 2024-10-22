#![deny(unsafe_code)]
#![no_main]
#![no_std]

// Halt on panic
use panic_halt as _;

use core::f32::consts::FRAC_PI_2;
use cortex_m_rt::entry;
use micromath::F32Ext;
use stm32f4xx_hal::{
    pac,
    prelude::*,
    gpio::{PinState, PushPull, Output, alt::TimCPin, Pin},
    timer::pwm::PwmChannel,
};

/* 14 magnetic center, 12 coil brushless motor*/

/* ~ U U' V' V W W' U' U V V' W' W ~ */

/* hole sensor in (*) <-> (*)' */

/* so can take ~(S|N) N  N   (N|S) S S~
 *              ~H1   H2 H3           ~  
 *
 * if  H1:S, H2:S, H3:S 
 *      (N|S), S, S 
 *  or  S, S, (S|N)
 *
 * ex)
 * magnet rotate clock-wise
 * H1    H2    H3
 * (N|S) S     S
 * N     (N|S) S
 * N     N     (N|S)
 * (S|N) N     N
 * S     (S|N) N
 * S     S     (S|N)
 *
 * def (U, V, W) -> = N
 * if want clock-wise and get (H1:N, H2:S, H3,S), then could two pattern
 * H1    H2    H3
 * HOLE  S     S     :) V->W
 * or
 * N     HOLE  S     :) U->W
 * 
 * second get (H1:N, H2:N, H3:S), then could two pattern
 * H1    H2    H3
 * N     HOLE  S     :) U->W
 * or 
 * N     N     (HOLE):) U->V
 *  
 *  しかし、以前に切り替わったのが H2 なため、H2がホールであると判断すべきか?
 *  だが、もうH2を通過しているため、一つ進んだH3がHOLEとして制御すべきか?
 *  (VとWの中間にHOLEが来るばあい、U->WでもU->Vでも変わらない)
 *  多分先に進ませたほうが良い(センサの反応が遅れるため)
 *
 * */

const BRIDGE_DEAD_TIME_US: u32 = 20;

/* [U_P, V_P, W_P,
 *  U_N, V_N, W_N] */
const BRIDGE_STATE :[[bool; 6]; 6] = [
    /* 0 */
    [ false, true, false, 
    false, false, true ],
    /* 1 */
    [ true, false, false, 
    false, false, true ],
    /* 2 */
    [ true, false, false, 
    false, true, false ],
    /* 3 */
    [ false, false, true,
    false, true, false ],
    /* 4 */
    [ false, false, true,
    true, false, false ],
    /* 5 */
    [ false, true, false,
    true, false, false ],
];
// 隣り合う同士(差-1 or 5<->0) はデッドタイムがいらない
// ほかは、前にonになってたピンをオフにしてデッドタイムを挟む必要がある

#[entry]
fn main() -> ! {
    if let Some(dp) = pac::Peripherals::take() {
        // Set up the system clock.
        let rcc = dp.RCC.constrain();
        let clocks = rcc.cfgr.use_hse(25.MHz()).freeze();

        let gpioa = dp.GPIOA.split();
        let gpiob = dp.GPIOB.split();
        let gpioc = dp.GPIOC.split();

        let m1_h1 = gpiob.pb0.into_floating_input();
        let m1_h2 = gpiob.pb1.into_floating_input();
        let m1_h3 = gpiob.pb2.into_floating_input();

        let (_, (pwm_c1, pwm_c2, pwm_c3,..)) = dp.TIM1.pwm_us(5.micros(), &clocks);
        /* N-ch */
        let mut m1_u_pwm_n = pwm_c1.with(gpioa.pa8);
        let mut m1_v_pwm_n = pwm_c2.with(gpioa.pa9);
        let mut m1_w_pwm_n = pwm_c3.with(gpioa.pa10);

        /* P-ch */
        let mut m1_u_p = gpioa.pa11.into_push_pull_output_in_state(PinState::Low);
        let mut m1_v_p = gpioa.pa12.into_push_pull_output_in_state(PinState::Low);
        let mut m1_w_p = gpioa.pa13.into_push_pull_output_in_state(PinState::Low);

        m1_u_p.set_low();
        m1_v_p.set_low();
        m1_w_p.set_low();

        /*
        let (_, (pwm_c4, pwm_c5, pwm_c6,..)) = dp.TIM2.pwm_us(100.micros(), &clocks);
        let mut pwm_c4 = pwm_c4.with(gpioa.pa0);
        let mut pwm_c5 = pwm_c5.with(gpioa.pa1);
        let mut pwm_c6 = pwm_c6.with(gpioa.pa2);
        */

        //let mut counter = dp.TIM3.counter_us(&clocks);
        //let max_duty = m1_u.get_max_duty();
        //counter.start(100.micros()).unwrap();
        
        m1_u_pwm_n.enable();
        m1_v_pwm_n.enable();
        m1_w_pwm_n.enable();
        m1_u_pwm_n.set_duty(0);
        m1_v_pwm_n.set_duty(0);
        m1_w_pwm_n.set_duty(0);

        let mut led1 = gpioa.pa6.into_push_pull_output_in_state(PinState::Low);
        let mut led2 = gpioa.pa7.into_push_pull_output_in_state(PinState::Low);
        let mut led3 = gpioa.pa14.into_push_pull_output_in_state(PinState::Low);
        let mut error_led = gpioc.pc13.into_push_pull_output_in_state(PinState::High);
        error_led.set_low();
        let mut delay = dp.TIM5.delay_us(&clocks);

        let mut cur_bridge_state: usize = 0;
        let mut req_bridge_state: usize = 0;
        loop {
            if m1_h1.is_high() {
                led1.set_high()
            } else {
                led1.set_low()
            }
            if m1_h2.is_high() {
                led2.set_high()
            } else {
                led2.set_low()
            }
            if m1_h3.is_high() {
                led3.set_high()
            } else {
                led3.set_low()
            }
            /*
            led1.set_high();
            delay.delay_ms(1000);
            led1.set_low();
            led2.set_high();
            delay.delay_ms(1000);
            led2.set_low();
            led3.set_high();
            delay.delay_ms(1000);
            led3.set_low();
            */

            /* test rotate without sensor */
            if req_bridge_state == 5 {
                req_bridge_state = 0;
            } else {
                req_bridge_state += 1;
            }
            
            /* change bridge state */
            let bridge_state_diff: usize = req_bridge_state.abs_diff(cur_bridge_state);
            if (bridge_state_diff <= 1) || bridge_state_diff == 5 {
            } else { /* need DEADTIME */
                m1_u_p.set_low();
                m1_v_p.set_low();
                m1_w_p.set_low();
                m1_u_pwm_n.set_duty(0);
                m1_v_pwm_n.set_duty(0);
                m1_w_pwm_n.set_duty(0);

                delay.delay_us(BRIDGE_DEAD_TIME_US);
            }
            let selected_bridge_state = BRIDGE_STATE[req_bridge_state];
            /* Pch */
            if selected_bridge_state[0] == true {
                m1_u_p.set_high();
            } else {
                m1_u_p.set_low();
            }
            if selected_bridge_state[1] == true {
                m1_v_p.set_high();
            } else {
                m1_v_p.set_low();
            }
            if selected_bridge_state[2] == true {
                m1_w_p.set_high();
            } else {
                m1_w_p.set_low();
            }
            /* Nch */
            if selected_bridge_state[3] == true {
                m1_u_pwm_n.set_duty(5);
            } else {
                m1_u_pwm_n.set_duty(0);
            }
            if selected_bridge_state[4] == true {
                m1_v_pwm_n.set_duty(5);
            } else {
                m1_v_pwm_n.set_duty(0);
            }
            if selected_bridge_state[5] == true {
                m1_w_pwm_n.set_duty(5);
            } else {
                m1_w_pwm_n.set_duty(0);
            }

            /* update cur state for next loop iter */
            cur_bridge_state = req_bridge_state;

            delay.delay_ms(10);
        }
    }

    loop {
        cortex_m::asm::nop();
    }
}
