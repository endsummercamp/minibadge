#![no_std]
#![no_main]

use core::cell::RefCell;

use cortex_m::interrupt::Mutex;
use defmt::*;
use defmt_rtt as _;
use embedded_hal::digital::StatefulOutputPin;
use infrared::{protocol::NecDebug, Receiver};
use panic_probe as _;

#[link_section = ".boot2"]
#[used]
pub static BOOT_LOADER: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

use rp2040_hal::{
    clocks::init_clocks_and_plls,
    entry,
    gpio::{bank0::Gpio10, FunctionSio, Pins, PullNone, SioInput},
    pac::{self, interrupt},
    pio::PIOExt,
    Clock, Sio, Timer, Watchdog,
};
use smart_leds::RGB8;
use ws2812_pio::Ws2812;

use rp2040_hal::gpio::Interrupt::EdgeHigh;
use rp2040_hal::gpio::Interrupt::EdgeLow;

const LED_MATRIX_WIDTH: usize = 3;
const LED_MATRIX_HEIGHT: usize = 3;
const LED_MATRIX_SIZE: usize = LED_MATRIX_WIDTH * LED_MATRIX_HEIGHT;
struct LedMatrix {
    framebuffer: [(u8, u8, u8); LED_MATRIX_SIZE],
    gain: f32,
}

#[allow(dead_code)]
impl LedMatrix {
    fn new() -> Self {
        Self {
            framebuffer: [(0, 0, 0); LED_MATRIX_SIZE],
            gain: 0.5,
        }
    }

    fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    fn set_pixel(&mut self, x: usize, y: usize, rgb: RGB8) {
        if x < LED_MATRIX_WIDTH && y < LED_MATRIX_HEIGHT {
            self.framebuffer[y * LED_MATRIX_WIDTH + x] = (rgb.r, rgb.g, rgb.b);
        }
    }

    fn set_all(&mut self, rgb: RGB8) {
        for i in 0..LED_MATRIX_SIZE {
            self.framebuffer[i] = (rgb.r, rgb.g, rgb.b);
        }
    }

    fn blit(&mut self) -> impl Iterator<Item = RGB8> + '_ {
        self.framebuffer.iter().map(|(r, g, b)| {
            let r = (*r as f32 * self.gain) as u8;
            let g = (*g as f32 * self.gain) as u8;
            let b = (*b as f32 * self.gain) as u8;
            (r, g, b).into()
        })
    }
}

static G_TIMER: Mutex<RefCell<Option<rp2040_hal::Timer>>> = Mutex::new(RefCell::new(None));

static G_INTERRUPT_RECEIVER: Mutex<
    RefCell<
        Option<Receiver<NecDebug, rp2040_hal::gpio::Pin<Gpio10, FunctionSio<SioInput>, PullNone>>>,
    >,
> = Mutex::new(RefCell::new(None));

#[entry]
fn main() -> ! {
    info!("Program start");
    let mut pac = pac::Peripherals::take().unwrap();
    let core = pac::CorePeripherals::take().unwrap();
    let mut watchdog = Watchdog::new(pac.WATCHDOG);
    let sio = Sio::new(pac.SIO);

    // External high-speed crystal on the pico board is 12Mhz
    let external_xtal_freq_hz = 12_000_000u32;
    let clocks = init_clocks_and_plls(
        external_xtal_freq_hz,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let mut delay = cortex_m::delay::Delay::new(core.SYST, clocks.system_clock.freq().to_Hz());

    let pins = Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let timer = Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);

    #[allow(unsafe_code)]
    unsafe {
        pac::NVIC::unmask(pac::Interrupt::TIMER_IRQ_0);
        pac::NVIC::unmask(pac::Interrupt::IO_IRQ_BANK0);
    }

    let (mut pio, sm0, _, _, _) = pac.PIO0.split(&mut pac.RESETS);

    let mut ir_blaster = pins.gpio11.into_push_pull_output();
    let ir_sensor = pins.gpio10.into_floating_input();
    let _user_button = pins.gpio9.into_pull_up_input();

    let int_receiver = Receiver::builder()
        .rc5()
        .frequency(1_000_000)
        .pin(ir_sensor)
        .protocol()
        .build();

    cortex_m::interrupt::free(|cs| {
        G_TIMER.borrow(cs).replace(Some(timer));
        G_INTERRUPT_RECEIVER.borrow(cs).replace(Some(int_receiver));

        if let Some(int_receiver) = G_INTERRUPT_RECEIVER.borrow(cs).borrow_mut().as_mut() {
            let pin = int_receiver.pin_mut();
            pin.set_interrupt_enabled(EdgeLow, true);
            pin.set_interrupt_enabled(EdgeHigh, true);
        }
    });

    println!("Starting loop");

    let mut ws = Ws2812::new(
        pins.gpio19.into_function(),
        &mut pio,
        sm0,
        clocks.peripheral_clock.freq(),
        timer.count_down(),
    );

    let mut mtrx = LedMatrix::new();

    mtrx.set_gain(0.1);

    use smart_leds::RGB8;
    use smart_leds_trait::SmartLedsWrite;

    let red: RGB8 = (255, 0, 0).into();
    let green: RGB8 = (0, 255, 0).into();
    let blue: RGB8 = (0, 0, 255).into();
    let black: RGB8 = (0, 0, 0).into();

    // clear all leds (if we are using a strip for debug)
    ws.write(core::iter::repeat(black).take(400)).unwrap();

    loop {
        mtrx.set_all(red);
        ws.write(mtrx.blit()).unwrap();

        delay.delay_ms(150);

        mtrx.set_all(green);
        ws.write(mtrx.blit()).unwrap();

        delay.delay_ms(150);

        mtrx.set_all(blue);
        ws.write(mtrx.blit()).unwrap();

        delay.delay_ms(150);

        ir_blaster.toggle().unwrap();
    }
}

#[interrupt]
fn IO_IRQ_BANK0() {
    cortex_m::interrupt::free(|cs| {
        if let Some(int_receiver) = G_INTERRUPT_RECEIVER.borrow(cs).borrow_mut().as_mut() {
            if let Some(timer) = G_TIMER.borrow(cs).borrow_mut().as_mut() {
                if let Ok(Some(cmd)) = int_receiver.event_instant(timer.get_counter_low()) {
                    println!("Action: {:?} ", cmd.bits);
                }
            }

            let pin = int_receiver.pin_mut();
            pin.clear_interrupt(EdgeLow);
            pin.clear_interrupt(EdgeHigh);
        }
    });
}
