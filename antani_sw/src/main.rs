#![no_std]
#![no_main]

use core::cell::RefCell;

use cortex_m::interrupt::Mutex;
use defmt::*;
use defmt_rtt as _;
use embedded_hal::digital::StatefulOutputPin;
use infrared::{protocol::NecDebug, Receiver};
use panic_probe as _;

use num_traits::real::Real;

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

struct LedPattern {
    pattern: u16,
}

impl LedPattern {

    const fn from(pattern: u16) -> Self {
        Self { pattern }
    }
}

const GLIDER_PATTERN: LedPattern = LedPattern::from(0b010001111);

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

    fn render(&mut self, pattern: LedPattern, colour: RGB8) {
        // this garbage is to rotate the pattern so the usb port is at the top
        // i hope the compiler optimizes this out

        if pattern.pattern & 0b100000000 != 0 {
            self.set_pixel(2, 0, colour);
        }

        if pattern.pattern & 0b010000000 != 0 {
            self.set_pixel(2, 1, colour);
        }

        if pattern.pattern & 0b001000000 != 0 {
            self.set_pixel(2, 2, colour);
        }

        if pattern.pattern & 0b000100000 != 0 {
            self.set_pixel(1, 0, colour);
        }

        if pattern.pattern & 0b000010000 != 0 {
            self.set_pixel(1, 1, colour);
        }

        if pattern.pattern & 0b000001000 != 0 {
            self.set_pixel(1, 2, colour);
        }

        if pattern.pattern & 0b000000100 != 0 {
            self.set_pixel(0, 0, colour);
        }

        if pattern.pattern & 0b000000010 != 0 {
            self.set_pixel(0, 1, colour);
        }

        if pattern.pattern & 0b000000001 != 0 {
            self.set_pixel(0, 2, colour);
        }
    }
}

static G_TIMER: Mutex<RefCell<Option<rp2040_hal::Timer>>> = Mutex::new(RefCell::new(None));

static G_INTERRUPT_RECEIVER: Mutex<
    RefCell<
        Option<Receiver<NecDebug, rp2040_hal::gpio::Pin<Gpio10, FunctionSio<SioInput>, PullNone>>>,
    >,
> = Mutex::new(RefCell::new(None));

fn hsl2rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let h = h * 360.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = match h {
        0.0..=60.0 => (c, x, 0.0),
        60.0..=120.0 => (x, c, 0.0),
        120.0..=180.0 => (0.0, c, x),
        180.0..=240.0 => (0.0, x, c),
        240.0..=300.0 => (x, 0.0, c),
        300.0..=360.0 => (c, 0.0, x),
        _ => (0.0, 0.0, 0.0), // This should not happen in a properly constrained input.
    };

    let r = ((r + m) * 255.0).round() as u8;
    let g = ((g + m) * 255.0).round() as u8;
    let b = ((b + m) * 255.0).round() as u8;

    (r, g, b)
}

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

    use smart_leds_trait::SmartLedsWrite;

    let mut t;
    loop {
        t = timer.get_counter().ticks() as f64 / 1_000_000.0;

        let gain = 0.5;

        let color = hsl2rgb((t * 0.25) % 1.0, 1.0, 0.5 * gain);
        let color2 = hsl2rgb((t * 0.25 + 0.5) % 1.0, 1.0, 0.5 * gain);

        mtrx.set_all(color.into());
        mtrx.render(GLIDER_PATTERN, color2.into());
        ws.write(mtrx.blit()).unwrap();

        delay.delay_ms(1);

        //ir_blaster.toggle().unwrap();
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
