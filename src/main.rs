#![deny(unsafe_code)]
#![no_main]
#![no_std]

// Halt on panic
use panic_halt as _;

use core::f32::consts::FRAC_PI_2;
use cortex_m_rt::entry;
use micromath::F32Ext;
use stm32f4xx_hal::{pac, prelude::*, gpio::PinState};

#[entry]
fn main() -> ! {
    if let Some(dp) = pac::Peripherals::take() {
        // Set up the system clock.
        let rcc = dp.RCC.constrain();
        let clocks = rcc.cfgr.use_hse(25.MHz()).freeze();

        let gpioa = dp.GPIOA.split();
        let gpiob = dp.GPIOB.split();

        let m1_h1 = gpiob.pb0.into_floating_input();
        let m1_h2 = gpiob.pb1.into_floating_input();
        let m1_h3 = gpiob.pb2.into_floating_input();

        let (_, (pwm_c1, pwm_c2, pwm_c3,..)) = dp.TIM1.pwm_us(100.micros(), &clocks);
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
        let mut delay = dp.TIM5.delay_us(&clocks);

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
        }
    }

    loop {
        cortex_m::asm::nop();
    }
}
