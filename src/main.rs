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
        
        /*
        // define RX/TX pins
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

        let mut pre_hole_sensor_state: u16 = 6; // 0~5, 6 is invalid

        let mut debug_counter: i32 = 0;
        let mut pre_debug_counter: i32 = 0;
        let mut stop_counter: i32 = 0;

        let mut count_timer: usize = 0;

        let mut speed: usize = 1;

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

            let m1_hole_sensor = [m1_h1.is_low(), m1_h2.is_low(), m1_h3.is_low()];

            let rotate_dir: bool = false;

            /* 観測した時点で考えられる２つのパターンのうち */
            /* 望む回転方向が逆の場合反転して進ませる必要がある(-1して反転(-3?) */
            
            let now_hole_sensor_state: u16 = match m1_hole_sensor {
                [false, false, false] => { 0 },
                [true, false, false] => { 1 },
                [true, true, false] => { 2 },
                [true, true, true] => { 3 },
                [false, true, true] => { 4 },
                [false, false, true] => { 5 },
                _ => {
                    /* NSN or SNS is invalid */
                    pre_hole_sensor_state
                    //6
                },
            };

            /* *now - pre
                               5  4  3  2  1  *  5  4  3
                               4  3  2  1  * -1  4  3
                               3  2  1  * -1 -2  3
                           -3  2  1  * -1 -2 -3
                        -3 -4  1  * -1 -2
                     -3 -4 -5  * -1 -2
            ~0  1  2  3  4  5  0  1  2  3  4  5  0  1  2  3  4~

            3なら無視、-4以下なら+6, 4以上なら-6
            有効範囲(+2, +1, 0, -1, -2)
            */
            let mut relative_diff: i32 = match now_hole_sensor_state {
                6 => { 0 },
                _ => { 
                    if pre_hole_sensor_state == 6 {
                        0
                    } else {
                        let diff = now_hole_sensor_state as i32 - pre_hole_sensor_state as i32;
                        if diff == 3 || diff == -3 {
                            0
                        } else {
                            if diff >= 4 {
                                diff - 6
                            } else if diff <= -4 {
                                diff + 6
                            } else {
                                diff
                            }
                        }
                    }
                },
            };

            /* -1 , +1 だけ使う */
            /*
            if relative_diff >= 2 || relative_diff <= -2 {
                relative_diff = 0;
            }
            */

            debug_counter += relative_diff;

            /*
            if count_timer % (1 << 17) == 0 { // every time then drop sensor count
                writeln!(tx, "{}\r", debug_counter).unwrap();
            }
            */

            /* test rotate without sensor */
            // 360 < 8bit, so can shift max 32-8 = 24
            const SCALE: usize = 8; // <= 24
            const COUNTER_MAX: usize = (SINE_RESOLUTION << SCALE) - 1;
            const COUNTER_MAX_DIV6: usize = COUNTER_MAX / 6;

            count_timer += 1;
            if count_timer % (COUNTER_MAX_DIV6 * 3) == 0 {
                let diff = if rotate_dir == true {
                    pre_debug_counter - debug_counter
                } else {
                    debug_counter - pre_debug_counter
                };

                if diff >= 3 {
                    speed += 2;
                    if speed >= 50 {
                        speed = 50;
                    }
                } else if diff == 2 {
                    /*
                    speed += 1;
                    if speed >= 30 {
                        speed = 30;
                    }
                    */
                    speed = speed.saturating_sub(10);
                    if speed == 0 {
                        speed = 1;
                    }
                } else if diff == 1 {
                    speed = speed.saturating_sub(20);
                    if speed == 0 {
                        speed = 1;
                    }
                } else {
                    speed = speed.saturating_sub(30);
                    //speed = speed >> 1;// / 2
                    if speed == 0 {
                        speed = 1;
                    }
                }
                // 回転方向が反転した場合はカウンターをリセットする必要がある?
                // もしくは、回転の切り替えは count_timer % (COUNTER_MAX_DIV6 * 3) ==
                // 0　のタイミングで行うか?
                pre_debug_counter = debug_counter;
            }

            if count_timer % (COUNTER_MAX_DIV6 * 6) == 0 {
                let diff = if rotate_dir == true {
                    stop_counter - debug_counter
                } else {
                    debug_counter - stop_counter
                };
                if diff <= 1 {
                    speed = 1;
                }
                stop_counter = debug_counter;
            }

            //speed = 10;

            req_bridge_state += speed;
            req_bridge_state = req_bridge_state % COUNTER_MAX;

            let shift_in_sine_res = 
                if rotate_dir == true { 
                    req_bridge_state >> SCALE
                } else {
                    (SINE_RESOLUTION - 1) - (req_bridge_state >> SCALE)
                };

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
