#![deny(unsafe_code)]
#![no_main]
#![no_std]

// Halt on panic
use panic_halt as _;

use core::f32::consts::PI;
use cortex_m_rt::entry;
use micromath::F32Ext;
use stm32f4xx_hal::{
    pac,
    prelude::*,
    gpio::{PinState, PushPull, Output, alt::TimCPin, Pin},
    timer::{pwm::PwmChannel, Polarity},
};

use core::fmt::Write; // for pretty formatting of the serial output

// i16 ( 2byte ) * 360 = 720 bytes <= 96KBytes
const SINE_RESOLUTION: usize = 360;
const SINE_RES_1_DIV_3: usize = SINE_RESOLUTION / 3;
const SINE_RES_2_DIV_3: usize = 2 * SINE_RESOLUTION / 3;

fn multfix15(a: i16, b: i16) -> i16 {
    ((a as i32 * b as i32) >> 15) as i16
}

#[entry]
fn main() -> ! {
    if let Some(dp) = pac::Peripherals::take() {
        // Set up the system clock.
        let rcc = dp.RCC.constrain();
        let clocks = rcc.cfgr.use_hse(25.MHz())
            .pclk1(42.MHz())
            .pclk2(84.MHz())
            .freeze();

        // should calc once
        let mut sinewave_with_third_harmonic_inj: [i16; SINE_RESOLUTION] = [0; SINE_RESOLUTION];
        for i in 0..SINE_RESOLUTION {
            sinewave_with_third_harmonic_inj[i] = (((PI * 2.0 * (i as f32 / SINE_RESOLUTION as f32)).sin() as f32 
                                                    + (1.0/6.0) * (3.0 * PI * 2.0 * (i as f32 / SINE_RESOLUTION as f32)).sin()
                                                    ) * (2.0 / 3.0.sqrt()) * 32768.0 as f32) as i16; // float2fix15 //2^15
        }

        let gpioa = dp.GPIOA.split();
        let gpiob = dp.GPIOB.split();
        let gpioc = dp.GPIOC.split();
        
        // define RX/TX pins
        /*
        let tx_pin = gpiob.pb6;
        let mut tx = dp.USART1.tx(tx_pin, 9600.bps(), &clocks).unwrap();
        for i in 0..SINE_RESOLUTION {
            writeln!(tx, "{}\r", sinewave_with_third_harmonic_inj[i]).unwrap();
        }
        */

        let m1_h1 = gpiob.pb0.into_floating_input();
        let m1_h2 = gpiob.pb1.into_floating_input();
        let m1_h3 = gpiob.pb2.into_floating_input();

        let (mut pwm_mngr, (pwm_c1, pwm_c2, pwm_c3,..)) = dp.TIM1.pwm_hz(20.kHz(), &clocks);

        /* N-ch, P-ch */
        let mut m1_u_pwm_n = pwm_c1.with(gpioa.pa8).with_complementary(gpiob.pb13);
        let mut m1_v_pwm_n = pwm_c2.with(gpioa.pa9).with_complementary(gpiob.pb14);
        let mut m1_w_pwm_n = pwm_c3.with(gpioa.pa10).with_complementary(gpiob.pb15);

        let max_duty: u16 = m1_u_pwm_n.get_max_duty();
        let half_duty: u16 = max_duty / 2;
        //writeln!(tx, "get_max_duty: {}\r", max_duty).unwrap();
        // 20 kHz pwm has max_duty 1250 
        
        m1_u_pwm_n.set_polarity(Polarity::ActiveHigh);
        m1_u_pwm_n.set_complementary_polarity(Polarity::ActiveHigh);
        m1_v_pwm_n.set_polarity(Polarity::ActiveHigh);
        m1_v_pwm_n.set_complementary_polarity(Polarity::ActiveHigh);
        m1_w_pwm_n.set_polarity(Polarity::ActiveHigh);
        m1_w_pwm_n.set_complementary_polarity(Polarity::ActiveHigh);

        pwm_mngr.set_dead_time(200);
        
        m1_u_pwm_n.enable();
        m1_u_pwm_n.enable_complementary();
        m1_v_pwm_n.enable();
        m1_v_pwm_n.enable_complementary();
        m1_w_pwm_n.enable();
        m1_w_pwm_n.enable_complementary();
        /* Nch max_duty, Pch 0*/
        m1_u_pwm_n.set_duty(max_duty);
        m1_v_pwm_n.set_duty(max_duty);
        m1_w_pwm_n.set_duty(max_duty);

        let mut led1 = gpioa.pa6.into_push_pull_output_in_state(PinState::Low);
        let mut led2 = gpioa.pa7.into_push_pull_output_in_state(PinState::Low);
        let mut led3 = gpioa.pa14.into_push_pull_output_in_state(PinState::Low);
        let mut error_led = gpioc.pc13.into_push_pull_output_in_state(PinState::High);
        error_led.set_low();
        let mut delay = dp.TIM5.delay_us(&clocks);

        let mut pre_hole_sensor_state: usize = 6; // 0~5, 6 is invalid

        let mut req_bridge_state: usize = 0;
        let mut target_state_diff: usize = 0;

        let mut speed: usize = 1; // min = 1
        loop {
            let rotate_dir: bool = true;
            
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

            let m1_hole_sensor = [m1_h3.is_high(), m1_h2.is_high(), m1_h1.is_high()];
            let now_hole_sensor_state = match m1_hole_sensor {
                [false, false, false] => { 0 },
                [true, false, false] => { 1 },
                [true, true, false] => { 2 },
                [true, true, true] => { 3 },
                [false, true, true] => { 4 },
                [false, false, true] => { 5 },
                _ => {
                    pre_hole_sensor_state  // or 6: invalid ?
                    /* NSN or SNS is invalid */
                },
            };

            // 360 < 8bit, so can shift max 32-8 = 24
            const SCALE: usize = 7; // <= 24 // > MIN_SPEED ?
            const COUNTER_MAX: usize = (SINE_RESOLUTION << SCALE) - 1;
            const COUNTER_MAX_DIV_6: usize = COUNTER_MAX / 6;
           
            let delay_roter_need_speed_down = target_state_diff > COUNTER_MAX_DIV_6;
            if (now_hole_sensor_state != pre_hole_sensor_state) || delay_roter_need_speed_down {
                if delay_roter_need_speed_down {
                    speed = speed.saturating_sub(1);
                    if speed == 0 {
                        speed = 1; // min = 1
                    }
                } else {
                    // need speed up?
                    speed += 1;
                }

                /* 観測した時点で考えられる２つのパターンのうち */
                req_bridge_state =
                    if rotate_dir == false {
                        if now_hole_sensor_state == 5 {
                            0
                        } else {
                            COUNTER_MAX_DIV_6 * (now_hole_sensor_state + 1)
                        }
                    } else {
                        COUNTER_MAX_DIV_6 * now_hole_sensor_state
                    };
                target_state_diff = 0;
            }

            /* maybe not need MAX, 0 if because sensor feedback and re scale */
            if rotate_dir == true {
                if req_bridge_state >= COUNTER_MAX {
                    req_bridge_state = 0;
                } else {
                    req_bridge_state += speed;
                    /* to avoid out of access */
                    if req_bridge_state > COUNTER_MAX {
                        req_bridge_state = COUNTER_MAX;
                    }
                }
            } else {
                if req_bridge_state < speed {
                    req_bridge_state = COUNTER_MAX;
                } else {
                    req_bridge_state -= speed;
                }
            }

            target_state_diff += 1;

            let shift_in_sine_res = req_bridge_state >> SCALE;

            let u: u16 = (half_duty as i32 + multfix15(sinewave_with_third_harmonic_inj[shift_in_sine_res], half_duty as i16) as i32) as u16;
            let v: u16 = (half_duty as i32 + multfix15(sinewave_with_third_harmonic_inj[(shift_in_sine_res + SINE_RES_1_DIV_3) % SINE_RESOLUTION], half_duty as i16) as i32) as u16;
            let w: u16 = (half_duty as i32 + multfix15(sinewave_with_third_harmonic_inj[(shift_in_sine_res + SINE_RES_2_DIV_3) % SINE_RESOLUTION], half_duty as i16) as i32) as u16;
            
            /* change bridge state */
            m1_u_pwm_n.set_duty(u);
            m1_v_pwm_n.set_duty(v);
            m1_w_pwm_n.set_duty(w);

            /* update cur state for next loop iter */
            pre_hole_sensor_state = now_hole_sensor_state;

            //delay.delay_us(100);
        }
    }

    loop {
        cortex_m::asm::nop();
    }
}
