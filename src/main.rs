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
extern crate usb_device;
extern crate usbd_hid;

//use cortex_m::asm;
use cortex_m::delay;
use cortex_m_rt::entry;
//use cortex_m::peripheral::{Peripherals, syst};

use hal::pac;
use hal::prelude::*;
use hal::otg_fs::{USB, UsbBus};

use usb_device::prelude::*;
use usbd_hid::descriptor::MouseReport;
use usbd_hid::hid_class::HIDClass;

static mut EP_MEMORY: [u32; 1024] = [0; 1024];

#[entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();
    let cp = cortex_m::peripheral::Peripherals::take().unwrap();

    let rcc = dp.RCC.constrain();
    
    let clocks = rcc
        .cfgr
        .use_hse(25.MHz())
        .sysclk(48.MHz())
        .pclk1(24.MHz())
        .pclk2(24.MHz())
        .freeze();

    let gpioc = dp.GPIOC.split();
    
    let mut led_blue = gpioc.pc13.into_push_pull_output();
    led_blue.set_low();

    let mut cp_delay = delay::Delay::new(cp.SYST, 48000000_u32);

    let gpioa = dp.GPIOA.split();

    let usb = USB {
        usb_global: dp.OTG_FS_GLOBAL,
        usb_device: dp.OTG_FS_DEVICE,
        usb_pwrclk: dp.OTG_FS_PWRCLK,
        pin_dm: gpioa.pa11.into_alternate(),
        pin_dp: gpioa.pa12.into_alternate(),
        hclk: clocks.hclk(),
    };

    let usb_bus = UsbBus::new(usb, unsafe { &mut EP_MEMORY });

    let mut usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x045e, 0x028e))
        .manufacturer("©Microsoft")
        .product("Controller")
        .serial_number("TEST")
        .device_class(0)
        .build();
    
    loop {
        if usb_dev.poll(&mut []) {
        }

        led_blue.set_high();
        cp_delay.delay_ms(500_u32);
        led_blue.set_low();
        cp_delay.delay_ms(500_u32);
    }
}
