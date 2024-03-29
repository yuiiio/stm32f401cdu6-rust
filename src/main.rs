#![no_std]
#![no_main]

// pick a panicking behavior
extern crate panic_halt; // you can put a breakpoint on `rust_begin_unwind` to catch panics
// extern crate panic_abort; // requires nightly
// extern crate panic_itm; // logs messages over ITM; requires ITM support
// extern crate panic_semihosting; // logs messages to the host stderr; requires a debugger
extern crate cortex_m;
extern crate cortex_m_rt;
extern crate stm32f4xx_hal as hal;

use core::f32::consts::{PI};
use micromath::F32Ext;

use hal::{
    block,
    pac::{self, ADC1},
    gpio::{Speed, PinState},
    prelude::*,
    spi::*,
    i2c::*,
    adc::{
        config::{AdcConfig, Clock, Dma, Resolution, SampleTime, Scan, Sequence},
        Adc,
    },
    dma::{config::DmaConfig, PeripheralToMemory, Stream0, StreamsTuple, Transfer},
};

//use cortex_m::asm;
use cortex_m_rt::entry;
//use cortex_m::peripheral::{Peripherals, syst};
use cortex_m::prelude::{
    _embedded_hal_spi_FullDuplex, 
    _embedded_hal_blocking_spi_Transfer, 
    _embedded_hal_blocking_spi_Write, };

use fugit::RateExtU32;
use display_interface_spi::SPIInterfaceNoCS;
use embedded_graphics::image::*;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use st7789::{Orientation, ST7789};

// reference from https://github.com/vha3/Hunter-Adams-RP2040-Demos/blob/master/Audio/g_Audio_FFT/fft.c
const NUM_SAMPLES: usize = 128;
const LOG2_NUM_SAMPLES: u16 = 7;// 256 = 2^8
// Length of short (16 bits) minus log2 number of samples (6)
const SHIFT_AMOUNT: u16 = 16 - LOG2_NUM_SAMPLES;

fn multfix15(a: i16, b: i16) -> i16 {
    ((a as i32 * b as i32) >> 15) as i16
}

fn fftfix(fr: &mut [i16; NUM_SAMPLES], fi: &mut [i16; NUM_SAMPLES], sinewave: &[i16; NUM_SAMPLES]) -> () {
    //bit order reverse
    for m in 1..(NUM_SAMPLES - 1) {
        // swap odd and even bits
        let mut mr = ((m >> 1) & 0x5555) | ((m & 0x5555) << 1);
        // swap consecutive pairs
        mr = ((mr >> 2) & 0x3333) | ((mr & 0x3333) << 2);
        // swap nibbles ... 
        mr = ((mr >> 4) & 0x0F0F) | ((mr & 0x0F0F) << 4);
        // swap bytes
        mr = ((mr >> 8) & 0x00FF) | ((mr & 0x00FF) << 8);
        // shift down mr
        mr >>= SHIFT_AMOUNT ;
        // don't swap that which has already been swapped
        if mr<=m { continue; }
        // swap the bit-reveresed indices
        let tr = fr[m] ;
        fr[m] = fr[mr] ;
        fr[mr] = tr ;
        let ti = fi[m] ;
        fi[m] = fi[mr] ;
        fi[mr] = ti ;
    }
    //println!("{:?}", fr);
    // Adapted from code by:
    // Tom Roberts 11/8/89 and Malcolm Slaney 12/15/94 malcolm@interval.com
    // Length of the FFT's being combined (starts at 1)
    let mut l: usize = 1 ;
    // Log2 of number of samples, minus 1
    let mut k: u16 = LOG2_NUM_SAMPLES - 1 ;
    // While the length of the FFT's being combined is less than the number 
    // of gathered samples . . .
    while l < NUM_SAMPLES {
        // Determine the length of the FFT which will result from combining two FFT's
        let istep: usize = l << 1 ;
        // For each element in the FFT's that are being combined . . .
        for m in 0..l {
            let j = m << k;
            let mut wr: i16 =  sinewave[j + (NUM_SAMPLES / 4)] ; // cos(2pi m/N)
            let mut wi: i16 = -sinewave[j] ;                 // sin(2pi m/N)
            wr >>= 1 ;                          // divide by two
            wi >>= 1 ;                          // divide by two
            // i gets the index of one of the FFT elements being combined
            let mut i: usize = m;
            while i < NUM_SAMPLES {
                // j gets the index of the FFT element being combined with i
                let j = i + l ;
                // compute the trig terms (bottom half of the above matrix)
                let tr = multfix15(wr, fr[j]) - multfix15(wi, fi[j]) ;
                let ti = multfix15(wr, fi[j]) + multfix15(wi, fr[j]) ;
                // divide ith index elements by two (top half of above matrix)
                let qr = fr[i] >> 1 ;
                let qi = fi[i] >> 1 ;
                // compute the new values at each index
                fr[j] = qr - tr ;
                fi[j] = qi - ti ;
                fr[i] = qr + tr ;
                fi[i] = qi + ti ;

                i += istep;
            }
        }
        k = k - 1;
        l = istep;
    }
}

#[entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();
    let cp = cortex_m::peripheral::Peripherals::take().unwrap();

    // SPI1 (max 42 Mbit/s) (SCK: PB3, MISO: PB4, MOSI: PB5)
    // SPI2 (max 21 Mbit/s)

    let rcc = dp.RCC.constrain();
    
    let clocks = rcc
        .cfgr
        .use_hse(25.MHz())
        .sysclk(84.MHz())
        .hclk(84.MHz())
        .pclk1(42.MHz())
        .pclk2(84.MHz())
        .freeze();

    let gpioc = dp.GPIOC.split();
    
    let mut led_blue = gpioc.pc13.into_push_pull_output();
    led_blue.set_low();

    let gpiob = dp.GPIOB.split();

    let mut cp_delay = cortex_m::delay::Delay::new(cp.SYST, clocks.sysclk().to_Hz());
    
    // for dir sensor
    let mut i2c = I2c::new(dp.I2C1, (gpiob.pb8, gpiob.pb9), 400.kHz(), &clocks);

    // for st7789 display
    let rst = gpiob.pb10.into_push_pull_output_in_state(PinState::Low); // reset pin
    let dc = gpiob.pb12.into_push_pull_output_in_state(PinState::Low); // dc pin
    // Note. We set GPIO speed as VeryHigh to it corresponds to SPI frequency 3MHz.
    // Otherwise it may lead to the 'wrong last bit in every received byte' problem.
    let spi2_mosi = gpiob
        .pb15
        .into_alternate()
        .speed(Speed::VeryHigh)
        .internal_pull_up(true);

    let spi2_sclk = gpiob.pb13.into_alternate().speed(Speed::VeryHigh);

    
    let spi2 = Spi::new(dp.SPI2, (spi2_sclk, NoMiso::new(), spi2_mosi), embedded_hal::spi::MODE_3, 21.MHz(), &clocks);
    
    // display interface abstraction from SPI and DC
    let di = SPIInterfaceNoCS::new(spi2, dc);
    
    // create driver
    let mut display = ST7789::new(di, rst, 240, 240);

    // initialize
    display.init(&mut cp_delay).unwrap();
    // set default orientation
    display.set_orientation(Orientation::LandscapeSwapped).unwrap();

    let raw_image_data = ImageRawLE::new(include_bytes!("../assets/rust.raw"), 240);
    let ferris = Image::new(&raw_image_data, Point::new(80, 0));

    // draw image on black background
    display.clear(Rgb565::BLACK).unwrap();
    ferris.draw(&mut display).unwrap();
    //cp_delay.delay_ms(500_u32);
    
    led_blue.set_high(); // disable led

    display.clear(Rgb565::BLACK).unwrap();

    // setup GPIOA
    let gpioa = dp.GPIOA.split();

    let dma = StreamsTuple::new(dp.DMA2);
    let config = DmaConfig::default()
        .transfer_complete_interrupt(true)
        .memory_increment(true)
        .double_buffer(false);

    // Configure pa0 as an analog input
    let adc_ch0 = gpioa.pa0.into_analog();
    let adc_ch1 = gpioa.pa1.into_analog();
    let adc_ch2 = gpioa.pa2.into_analog();
    let adc_ch3 = gpioa.pa3.into_analog();
    let adc_ch4 = gpioa.pa4.into_analog();
    let adc_ch5 = gpioa.pa5.into_analog();
    let adc_ch6 = gpioa.pa6.into_analog();
    let adc_ch7 = gpioa.pa7.into_analog();
    let adc_ch8 = gpiob.pb0.into_analog();
    let adc_ch9 = gpiob.pb1.into_analog();

    let adc_config = AdcConfig::default()
            .dma(Dma::Continuous)
            //Scan mode is also required to convert a sequence
            .scan(Scan::Enabled)
            .resolution(Resolution::Twelve)
            .clock(Clock::Pclk2_div_2); // 84 / 4 = 21MHz need more down ? (adc max datasheet says
                                        // 36MHz)
                                        // 12-bit resolution single ADC 2 Msps

    // setup ADC
    let mut adc = Adc::adc1(dp.ADC1, true, adc_config);

    adc.configure_channel(&adc_ch0, Sequence::One, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch1, Sequence::Two, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch2, Sequence::Three, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch3, Sequence::Four, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch4, Sequence::Five, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch5, Sequence::Six, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch6, Sequence::Seven, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch7, Sequence::Eight, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch8, Sequence::Nine, SampleTime::Cycles_3);
    adc.configure_channel(&adc_ch9, Sequence::Ten, SampleTime::Cycles_3);

    let first_buffer = cortex_m::singleton!(: [u16; 10] = [0; 10]).unwrap();
    let mut second_buffer: Option<&'static mut [u16; 10]> = Some(cortex_m::singleton!(: [u16; 10] = [0; 10]).unwrap());

    let mut transfer = Transfer::init_peripheral_to_memory(dma.0, adc, first_buffer, None, config);

    // should calc once
    let mut sinewave: [i16; NUM_SAMPLES] = [0; NUM_SAMPLES];
    for i in 0..NUM_SAMPLES {
        sinewave[i] = ((6.283 * (i as f32 / NUM_SAMPLES as f32)).sin() as f32 * 32768.0 as f32) as i16; // float2fix15 //2^15
    }

    let mut rsqrt_table: [u8; ((u16::MAX as u32 + 1) / 2) as usize] = [0; ((u16::MAX as u32 + 1) / 2) as usize]; // /2 resolution: 65536 * 16 bit = 131... KBytes / 2 = 65.5... KBytes
    for i in 0..((u16::MAX as u32 + 1) / 2) {
        let x = i << 1; // *2
        rsqrt_table[i as usize] = ((1.0 / (x as f32).sqrt()) * u8::MAX as f32) as u8;
    }

    let buffer1: &mut [u8; 240] = &mut [0; 240];
    let buffer2: &mut [u8; 240] = &mut [0; 240];
    let mut flip: bool = true;
    loop {
        let mut buffer: &mut [u8; 240] = &mut [0; 240];
        let mut prev: &mut [u8; 240] = &mut [0; 240];
        if flip == true {
            buffer = buffer1;
            prev = buffer2;
            flip = false;
        } else {
            buffer = buffer2;
            prev = buffer1;
            flip = true;
        }

        let adc_results: &mut [[u16; NUM_SAMPLES]; 10] = &mut [[0; NUM_SAMPLES]; 10];
        for i in 0..NUM_SAMPLES {
            transfer.start(|adc| {
                adc.start_conversion();
            });

            transfer.wait();

            let (dma_buf, _) = transfer
                .next_transfer(second_buffer.take().unwrap())
                .unwrap();
            adc_results[0][i] = dma_buf[0];
            adc_results[1][i] = dma_buf[1];
            adc_results[2][i] = dma_buf[2];
            adc_results[3][i] = dma_buf[3];
            adc_results[4][i] = dma_buf[4];
            adc_results[5][i] = dma_buf[5];
            adc_results[6][i] = dma_buf[6];
            adc_results[7][i] = dma_buf[7];
            adc_results[8][i] = dma_buf[8];
            adc_results[9][i] = dma_buf[9];

            second_buffer = Some(dma_buf);
        /*
            adc_results[0][i] = adc.convert(&adc_ch0, SampleTime::Cycles_3);
            adc_results[1][i] = adc.convert(&adc_ch1, SampleTime::Cycles_3);
            adc_results[2][i] = adc.convert(&adc_ch2, SampleTime::Cycles_3);
            adc_results[3][i] = adc.convert(&adc_ch3, SampleTime::Cycles_3);
            adc_results[4][i] = adc.convert(&adc_ch4, SampleTime::Cycles_3);
            adc_results[5][i] = adc.convert(&adc_ch5, SampleTime::Cycles_3);
            adc_results[6][i] = adc.convert(&adc_ch6, SampleTime::Cycles_3);
            adc_results[7][i] = adc.convert(&adc_ch7, SampleTime::Cycles_3);
            adc_results[8][i] = adc.convert(&adc_ch8, SampleTime::Cycles_3);
            adc_results[9][i] = adc.convert(&adc_ch9, SampleTime::Cycles_3);
        */
        }

        let mut fr: [i16; NUM_SAMPLES] = [0; NUM_SAMPLES];
        let mut fi: [i16; NUM_SAMPLES] = [0; NUM_SAMPLES];
        for i in 0..NUM_SAMPLES {
            fr[i] = adc_results[0][i] as i16;
        }

        fftfix(&mut fr, &mut fi, &sinewave);

        let mut amplitudes: [u16; NUM_SAMPLES/2] = [0; NUM_SAMPLES/2];
        for i in 0..NUM_SAMPLES/2 { // 128 ~ 240 should 00
            amplitudes[i] = (fr[i].abs() + fi[i].abs()) as u16;
        }

        for i in 0..NUM_SAMPLES/2 { // show fft result
            buffer[i] = if amplitudes[i] > 239 { 239 } else { amplitudes[i] as u8 };
        }

        for i in 0..NUM_SAMPLES { // show raw input in free space
            // raw adc_in is 12bit >> 4 => 8bit
            // u8 max is 255
            // need clamp or scale to 240
            let adc_8bit: u8 = (adc_results[0][i] >> 4) as u8;
            buffer[(NUM_SAMPLES/2) + i] = if adc_8bit > 239 { 239 } else { adc_8bit };
        }

        let pulse_strength: u16 = amplitudes[5] as u16; // depend ball pulse

        for i in ((NUM_SAMPLES/2) + NUM_SAMPLES)..240-20 {
            buffer[i] = if (pulse_strength >> 0) > 239 { 239 } else { (pulse_strength >> 0) as u8 };
        }

        let ball_dist: u8 = rsqrt_table[(pulse_strength >> 1) as usize];

        for i in 240-20..240 {
            buffer[i] = if (ball_dist >> 0) > 239 { 239 } else { (ball_dist >> 0) as u8 };
        }

        /*
        // dir sensor
        let addr: u8 = 0x50 >> 1;
        let addr2: u8 = 0x51 >> 1;
        let reg: u8 = 0x20;

        let _ = i2c.write(addr, &[reg]);

        let _ = i2c.write(addr2, &[reg]);

        let mut read_data: [u8; 2] = [0x00; 2];

        let _ = i2c.read(addr, &mut read_data);

        let dir: u16 = ((read_data[0] as u16) << 8 ) | read_data[1] as u16;

        for i in 0..240 {
            buffer[i] = if (dir >> 0) > 239 { 239 } else { (dir >> 0) as u8 };
        }
        */
        
        // clear
        for i in 0..240 {
            let value = prev[i as usize] as u8;
            //for pos in 0.. value { 
                display.set_pixel(i+80, value as u16, 0b0000000000000000).ok();
            //}
        }

        // draw
        for i in 0..240 {
            let value = buffer[i as usize] as u8;
            //for pos in 0.. value { 
                display.set_pixel(i+80, value as u16, 0b1111111111111111).ok();
            //}
        }

        cp_delay.delay_ms(500_u32);
    }
}
