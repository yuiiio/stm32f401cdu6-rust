#![no_std]
#![no_main]

// pick a panicking behavior
extern crate panic_halt; // you can put a breakpoint on `rust_begin_unwind` to catch panics
// extern crate panic_abort; // requires nightly
// extern crate panic_itm; // logs messages over ITM; requires ITM support
// extern crate panic_semihosting; // logs messages to the host stderr; requires a debugger
extern crate cortex_m_rt;
extern crate stm32f4;

//use cortex_m::asm;
use cortex_m_rt::entry;
use stm32f4::stm32f401;
//use cortex_m::peripheral::{Peripherals, syst};

#[entry]
fn main() -> ! {

    // 初期化
    let peri = stm32f401::Peripherals::take().unwrap();

    // システムクロック　48MHz
    // PLLCFGR設定
    // bit22: PLLSRC=hsi
    // bit17-16: PLLP=8
    // bit14-06: PLLN=192
    // bit05-00: PLLM=8
    {
        let pllcfgr = &peri.RCC.pllcfgr;
        pllcfgr.modify(|_,w| w.pllsrc().hsi());
        pllcfgr.modify(|_,w| w.pllp().div8());
        pllcfgr.modify(|_,w| unsafe { w.plln().bits(192u16) });
        pllcfgr.modify(|_,w| unsafe { w.pllm().bits(8u8) });
    }

    // PLL起動
    peri.RCC.cr.modify(|_,w| w.pllon().on());
    while peri.RCC.cr.read().pllrdy().is_not_ready() {
        // PLLの安定を待つ
    }

    // フラッシュ読み出し遅延の変更
    peri.FLASH.acr.modify(|_,w| w.latency().bits(1u8));
    // システムクロックをPLLに切り替え
    peri.RCC.cfgr.modify(|_,w| w.sw().pll());
    while !peri.RCC.cfgr.read().sws().is_pll() { 
        //　システムクロックの切り替え完了を待つ
    }

    // GPIO 電源ON
    peri.RCC.ahb1enr.modify(|_,w| w.gpiocen().enabled());
    // TIM11 電源ON
    peri.RCC.apb2enr.modify(|_,w| w.tim11en().enabled());

    // GPIOC セットアップ
    let gpioc = &peri.GPIOC;
    gpioc.moder.modify(|_,w| w.moder13().output());
    // TIM11 セットアップ
    let tim11 = &peri.TIM11;
    tim11.psc.modify(|_,w| w.psc().bits(48_000u16 - 1));   // 1ms
    tim11.arr.modify(|_,w| unsafe {w.arr().bits(500u16)}); // 500ms
    tim11.cr1.modify(|_,w| w.cen().enabled());

    //main loop
    gpioc.bsrr.write(|w| w.bs13().set());
    loop {
            if tim11.sr.read().uif().is_update_pending() {
                tim11.sr.modify(|_,w| w.uif().clear());
                gpioc.odr.modify(|r,w| w.odr13().bit(r.odr13().is_low()));
            }
    }
}
