#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::{InterruptExecutor, Spawner};
use embassy_rp::adc;
use embassy_rp::dma;
use embassy_rp::gpio::Input;
use embassy_rp::gpio::Pull;
use embassy_rp::interrupt;
use embassy_rp::interrupt::{InterruptExt, Priority};

use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{
    Common, Config, FifoJoin, Instance, InterruptHandler, Pio, PioPin, ShiftConfig, ShiftDirection,
    StateMachine,
};
use embassy_rp::{bind_interrupts, clocks, into_ref, Peripheral, PeripheralRef};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_sync::channel::{Channel, Sender};
use embassy_time::Instant;
use embassy_time::{Duration, Ticker, Timer};
use fixed::types::U24F8;
use fixed_macro::fixed;
use infrared::{protocol::NecDebug, Receiver};
use num_traits::real::Real;
use panic_probe as _;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
});

use smart_leds::RGB8;

const LED_MATRIX_WIDTH: usize = 3;
const LED_MATRIX_HEIGHT: usize = 3;
const LED_MATRIX_SIZE: usize = LED_MATRIX_WIDTH * LED_MATRIX_HEIGHT;

struct LedPattern {
    pattern: u16,
}

impl From<u16> for LedPattern {
    fn from(pattern: u16) -> Self {
        Self { pattern }
    }
}

const GLIDER_PATTERN: LedPattern = LedPattern {
    pattern: 0b010001111,
};

const ALL_ON_PATTERN: LedPattern = LedPattern {
    pattern: 0b111111111,
};

struct LedMatrix {
    pub framebuffer: [RGB8; LED_MATRIX_SIZE],
    gain: f32,
}

#[allow(dead_code)]
impl LedMatrix {
    fn new() -> Self {
        Self {
            framebuffer: [(0, 0, 0).into(); LED_MATRIX_SIZE],
            gain: 0.5,
        }
    }

    fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    fn set_pixel(&mut self, x: usize, y: usize, rgb: RGB8) {
        if x < LED_MATRIX_WIDTH && y < LED_MATRIX_HEIGHT {
            self.framebuffer[y * LED_MATRIX_WIDTH + x] = rgb;
        }
    }

    fn set_all(&mut self, rgb: RGB8) {
        for i in 0..LED_MATRIX_SIZE {
            self.framebuffer[i] = rgb;
        }
    }

    fn blit(&mut self) -> impl Iterator<Item = RGB8> + '_ {
        self.framebuffer.iter().map(|rgb| {
            let r = (rgb.r as f32 * self.gain) as u8;
            let g = (rgb.g as f32 * self.gain) as u8;
            let b = (rgb.b as f32 * self.gain) as u8;
            (r, g, b).into()
        })
    }

    fn render(&mut self, pattern: LedPattern, colour: RGB8) {
        // this garbage is to rotate the pattern so the usb port is at the top
        // i hope the compiler optimizes this out

        let colour = RGB8 {
            r: (colour.r as f32 * self.gain) as u8,
            g: (colour.g as f32 * self.gain) as u8,
            b: (colour.b as f32 * self.gain) as u8,
        };

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
pub struct Ws2812<'d, P: Instance, const S: usize, const N: usize> {
    dma: PeripheralRef<'d, dma::AnyChannel>,
    sm: StateMachine<'d, P, S>,
}

impl<'d, P: Instance, const S: usize, const N: usize> Ws2812<'d, P, S, N> {
    pub fn new(
        pio: &mut Common<'d, P>,
        mut sm: StateMachine<'d, P, S>,
        dma: impl Peripheral<P = impl dma::Channel> + 'd,
        pin: impl PioPin,
    ) -> Self {
        into_ref!(dma);

        // Setup sm0

        // prepare the PIO program
        let side_set = pio::SideSet::new(false, 1, false);
        let mut a: pio::Assembler<32> = pio::Assembler::new_with_side_set(side_set);

        const T1: u8 = 2; // start bit
        const T2: u8 = 5; // data bit
        const T3: u8 = 3; // stop bit
        const CYCLES_PER_BIT: u32 = (T1 + T2 + T3) as u32;

        let mut wrap_target = a.label();
        let mut wrap_source = a.label();
        let mut do_zero = a.label();
        a.set_with_side_set(pio::SetDestination::PINDIRS, 1, 0);
        a.bind(&mut wrap_target);
        // Do stop bit
        a.out_with_delay_and_side_set(pio::OutDestination::X, 1, T3 - 1, 0);
        // Do start bit
        a.jmp_with_delay_and_side_set(pio::JmpCondition::XIsZero, &mut do_zero, T1 - 1, 1);
        // Do data bit = 1
        a.jmp_with_delay_and_side_set(pio::JmpCondition::Always, &mut wrap_target, T2 - 1, 1);
        a.bind(&mut do_zero);
        // Do data bit = 0
        a.nop_with_delay_and_side_set(T2 - 1, 0);
        a.bind(&mut wrap_source);

        let prg = a.assemble_with_wrap(wrap_source, wrap_target);
        let mut cfg = Config::default();

        // Pin config
        let out_pin = pio.make_pio_pin(pin);
        cfg.set_out_pins(&[&out_pin]);
        cfg.set_set_pins(&[&out_pin]);

        cfg.use_program(&pio.load_program(&prg), &[&out_pin]);

        // Clock config, measured in kHz to avoid overflows
        // TODO CLOCK_FREQ should come from embassy_rp
        let clock_freq = U24F8::from_num(clocks::clk_sys_freq() / 1000);
        let ws2812_freq = fixed!(800: U24F8);
        let bit_freq = ws2812_freq * CYCLES_PER_BIT;
        cfg.clock_divider = clock_freq / bit_freq;

        // FIFO config
        cfg.fifo_join = FifoJoin::TxOnly;
        cfg.shift_out = ShiftConfig {
            auto_fill: true,
            threshold: 24,
            direction: ShiftDirection::Left,
        };

        sm.set_config(&cfg);
        sm.set_enable(true);

        Self {
            dma: dma.map_into(),
            sm,
        }
    }

    pub async fn write(&mut self, colors: &[RGB8; N]) {
        // Precompute the word bytes from the colors
        let mut words = [0u32; N];
        for i in 0..N {
            let word = (u32::from(colors[i].g) << 24)
                | (u32::from(colors[i].r) << 16)
                | (u32::from(colors[i].b) << 8);
            words[i] = word;
        }

        // DMA transfer
        self.sm.tx().dma_push(self.dma.reborrow(), &words).await;

        Timer::after_micros(55).await;
    }
}

type AntaniSender = Sender<'static, CriticalSectionRawMutex, AntaniCommand, 8>;
enum AntaniCommand {
    ThermalThrottleMultiplier(f32), // 1.0 = no throttle, 0.0 = full throttle
    IrCommand(u32),
}
static CHANNEL: Channel<CriticalSectionRawMutex, AntaniCommand, 8> = Channel::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Program start");
    let p = embassy_rp::init(Default::default());

    let Pio {
        mut common, sm0, ..
    } = Pio::new(p.PIO0, Irqs);

    let adc = adc::Adc::new(p.ADC, Irqs, adc::Config::default());
    let ts = adc::Channel::new_temp_sensor(p.ADC_TEMP_SENSOR);
    unwrap!(spawner.spawn(temperature(adc, ts, CHANNEL.sender())));

    // let mut ir_blaster = pins.gpio11.into_push_pull_output();
    let ir_sensor = Input::new(p.PIN_10, Pull::None);
    // let _user_button = pins.gpio9.into_pull_up_input();

    interrupt::SWI_IRQ_1.set_priority(Priority::P3);
    let highpriority_spawner = EXECUTOR_HIGH.start(interrupt::SWI_IRQ_1);
    unwrap!(highpriority_spawner.spawn(ir_receiver(ir_sensor, CHANNEL.sender())));

    let mut mtrx = LedMatrix::new();

    let mut ws2812 = Ws2812::new(&mut common, sm0, p.DMA_CH0, p.PIN_19);

    println!("Starting loop");

    let mut t;
    loop {
        //t = timer.get_counter().ticks() as f64 / 1_000_000.0;
        t = Instant::now().as_micros() as f64 / 1_000_000.0;


        if let Ok(ch) = CHANNEL.try_receive() {
            match ch {
                AntaniCommand::ThermalThrottleMultiplier(gain) => {
                    mtrx.set_gain(gain);
                    println!("Thermal throttle multiplier: {}", gain);
                }
                AntaniCommand::IrCommand(cmd) => {
                    println!("IR command: {}", cmd);
                }
            }
        }

        let color = hsl2rgb((t * 0.25) % 1.0, 1.0, 0.5);
        let color2 = hsl2rgb((t * 0.25 + 0.5) % 1.0, 1.0, 0.5);

        mtrx.render(ALL_ON_PATTERN, color.into());
        mtrx.render(GLIDER_PATTERN, color2.into());
        ws2812.write(&mtrx.framebuffer).await;

        Timer::after_millis(1).await;

        //ir_blaster.toggle().unwrap();
    }
}

static EXECUTOR_HIGH: InterruptExecutor = InterruptExecutor::new();

#[interrupt]
unsafe fn SWI_IRQ_1() {
    EXECUTOR_HIGH.on_interrupt()
}

#[embassy_executor::task]
async fn ir_receiver(ir_sensor: Input<'static>, control: AntaniSender) {
    let mut int_receiver: Receiver<NecDebug, embassy_rp::gpio::Input> = Receiver::builder()
        .rc5()
        .frequency(1_000_000)
        .pin(ir_sensor)
        .protocol()
        .build();

    loop {
        int_receiver.pin_mut().wait_for_any_edge().await;

        if let Ok(Some(cmd)) = int_receiver.event_instant(Instant::now().as_ticks() as u32) {
            control.send(AntaniCommand::IrCommand(cmd.bits)).await;
        }
    }
}

#[embassy_executor::task]
async fn temperature(
    mut adc: adc::Adc<'static, adc::Async>,
    mut ts: adc::Channel<'static>,
    control: AntaniSender,
) {
    let mut ticker = Ticker::every(Duration::from_secs(1));

    loop {
        let temp = adc.read(&mut ts).await.unwrap();

        // TODO: yeah let's waste precious CPU cycles to calculate the temperature before checking if we need to throttle
        let adc_voltage = (3.3 / 4096.0) * temp as f64;
        let temp_degrees_c = 27.0 - (adc_voltage - 0.706) / 0.001721;

        if temp_degrees_c > 40.0 {
            // lerp from 55 to 65 degrees maps to gain from 1.0 to 0.1
            let gain: f64 = 1.0 - (temp_degrees_c - 55.0) / 10.0;
            let gain = gain.clamp(0.0, 1.0);
            control
                .send(AntaniCommand::ThermalThrottleMultiplier(gain as f32))
                .await;
        }

        ticker.next().await;
    }
}
