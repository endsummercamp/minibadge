#![no_std]
#![no_main]

use core::f64;

use defmt::*;
use defmt_rtt as _;
use embassy_executor::{InterruptExecutor, Spawner};
use embassy_rp::adc;
use embassy_rp::gpio::Input;
use embassy_rp::gpio::Pull;
use embassy_rp::interrupt;
use embassy_rp::interrupt::{InterruptExt, Priority};

use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_sync::channel::{Channel, Sender};
use embassy_sync::lazy_lock::LazyLock;
use embassy_time::Instant;
use embassy_time::{Duration, Ticker, Timer};

use embassy_rp::bind_interrupts;
use heapless::Vec;
use infrared::{protocol::NecDebug, Receiver};
use panic_probe as _;

mod rgbeffects;
mod ws2812;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
});

use rand::rngs::SmallRng;
use rand::SeedableRng;
use rgbeffects::AnimationPattern;
use rgbeffects::ColorPalette;
use rgbeffects::FragmentShader;
use rgbeffects::LedPattern;
use rgbeffects::RenderCommand;
use rgbeffects::RenderManager;
use rgbeffects::RunEffect;
use smart_leds::RGB8;
use ws2812::Ws2812;

const LED_MATRIX_WIDTH: usize = 3;
const LED_MATRIX_HEIGHT: usize = 3;
const LED_MATRIX_SIZE: usize = LED_MATRIX_WIDTH * LED_MATRIX_HEIGHT;

struct LedMatrix {
    pub framebuffer: [RGB8; LED_MATRIX_SIZE],
    gain: f32,
    override_gain: f32,
}

#[allow(dead_code)]
impl LedMatrix {
    fn new() -> Self {
        Self {
            framebuffer: [(0, 0, 0).into(); LED_MATRIX_SIZE],
            gain: 0.5,
            override_gain: 1.0,
        }
    }

    fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    fn set_override_gain(&mut self, gain: f32) {
        self.override_gain = gain;
    }

    fn get_gain(&self) -> f32 {
        self.gain * self.override_gain
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

    fn clear(&mut self) {
        self.set_all((0, 0, 0).into());
    }

    fn blit(&mut self) -> impl Iterator<Item = RGB8> + '_ {
        self.framebuffer.iter().map(|rgb| {
            let r = (rgb.r as f32 * self.get_gain()) as u8;
            let g = (rgb.g as f32 * self.get_gain()) as u8;
            let b = (rgb.b as f32 * self.get_gain()) as u8;
            (r, g, b).into()
        })
    }

    fn render(&mut self, pattern: &LedPattern, colour: RGB8) {
        let colour = RGB8 {
            r: (colour.r as f32 * self.get_gain()) as u8,
            g: (colour.g as f32 * self.get_gain()) as u8,
            b: (colour.b as f32 * self.get_gain()) as u8,
        };

        // this maps bits in the pattern bitfield to the corresponding led in the matrix
        let bit_offsets = [
            (0, 2), // bit 0, first led
            (0, 1),
            (0, 0),
            (1, 2),
            (1, 1),
            (1, 0),
            (2, 2),
            (2, 1),
            (2, 0), // bit 8, the last led
        ];

        for (i, (x, y)) in bit_offsets.iter().enumerate() {
            if pattern.pattern & (1 << i) != 0 {
                self.set_pixel(*x, *y, colour);
            }
        }
    }
}

type AppSender = Sender<'static, CriticalSectionRawMutex, AppCommand, 8>;
enum AppCommand {
    ThermalThrottleMultiplier(f32), // 1.0 = no throttle, 0.0 = full throttle
    IrCommand(u32),
}
static CHANNEL: Channel<CriticalSectionRawMutex, AppCommand, 8> = Channel::new();

struct Patterns {
    pub glider: LedPattern,
    pub all_on: LedPattern,
    pub everything_once: AnimationPattern,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| Patterns {
    glider: LedPattern::new(0b010001111),
    all_on: LedPattern::new(0b111111111),
    everything_once: AnimationPattern::new(&[
        0b100000000,
        0b010000000,
        0b001000000,
        0b000100000,
        0b000010000,
        0b000001000,
        0b000000100,
        0b000000010,
        0b000000001,
    ]),
});

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

    let mut renderman = RenderManager {
        mtrx: LedMatrix::new(),
        rng: SmallRng::seed_from_u64(69420),
    };

    let mut ws2812 = Ws2812::new(&mut common, sm0, p.DMA_CH0, p.PIN_19);

    let patterns = PATTERNS.get();

    println!("Starting loop");

    let scenes: [Vec<RenderCommand, 8>; 3] = [
        // strobing glider
        Vec::from_slice(&[RenderCommand {
            effect: RunEffect::SimplePattern(patterns.glider),
            color: ColorPalette::Solid((0, 0, 255).into()),
            color_shaders: Vec::from_slice(&[
                FragmentShader::Breathing(0.7),
                FragmentShader::Blinking(10.0),
            ])
            .unwrap(),
        }])
        .unwrap(),
        // glider with particles
        Vec::from_slice(&[
            RenderCommand {
                effect: RunEffect::SimplePattern(patterns.glider),
                color: ColorPalette::Solid((0, 0, 255).into()),
                color_shaders: Vec::from_slice(&[FragmentShader::Breathing(0.7)]).unwrap(),
            },
            RenderCommand {
                effect: RunEffect::AnimationPattern(&patterns.everything_once, 6.0),
                color: ColorPalette::Rainbow(0.25, 0.0),
                color_shaders: Vec::new(),
            },
            RenderCommand {
                effect: RunEffect::ReverseAnimationPattern(&patterns.everything_once, 6.0),
                color: ColorPalette::Rainbow(0.25, 0.5),
                color_shaders: Vec::new(),
            },
        ])
        .unwrap(),
        // double rainbow glider
        Vec::from_slice(&[
            RenderCommand {
                effect: RunEffect::SimplePattern(patterns.all_on),
                color: ColorPalette::Rainbow(0.25, 0.0),
                color_shaders: Vec::new(),
            },
            RenderCommand {
                effect: RunEffect::SimplePattern(patterns.glider),
                color: ColorPalette::Rainbow(0.25, 0.5),
                color_shaders: Vec::new(),
            },
        ])
        .unwrap(),
    ];

    loop {
        //t = timer.get_counter().ticks() as f64 / 1_000_000.0;
        let t = Instant::now().as_micros() as f64 / 1_000_000.0;

        renderman.mtrx.set_gain(0.5);

        if let Ok(ch) = CHANNEL.try_receive() {
            match ch {
                AppCommand::ThermalThrottleMultiplier(gain) => {
                    renderman.mtrx.set_override_gain(gain);
                    println!("Thermal throttle multiplier: {}", gain);
                }
                AppCommand::IrCommand(cmd) => {
                    println!("IR command: {}", cmd);
                }
            }
        }

        // change scene every 5 seconds
        let scene_id = (t as u64 / 5) % scenes.len() as u64;
        renderman.render(&scenes[scene_id as usize], t);

        ws2812.write(&renderman.mtrx.framebuffer).await;
        Timer::after_millis(1).await;
        renderman.mtrx.clear();
    }
}

static EXECUTOR_HIGH: InterruptExecutor = InterruptExecutor::new();

#[interrupt]
unsafe fn SWI_IRQ_1() {
    EXECUTOR_HIGH.on_interrupt()
}

#[embassy_executor::task]
async fn ir_receiver(ir_sensor: Input<'static>, control: AppSender) {
    let mut int_receiver: Receiver<NecDebug, embassy_rp::gpio::Input> = Receiver::builder()
        .rc5()
        .frequency(1_000_000)
        .pin(ir_sensor)
        .protocol()
        .build();

    loop {
        int_receiver.pin_mut().wait_for_any_edge().await;

        if let Ok(Some(cmd)) = int_receiver.event_instant(Instant::now().as_ticks() as u32) {
            control.send(AppCommand::IrCommand(cmd.bits)).await;
        }
    }
}

#[embassy_executor::task]
async fn temperature(
    mut adc: adc::Adc<'static, adc::Async>,
    mut ts: adc::Channel<'static>,
    control: AppSender,
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
                .send(AppCommand::ThermalThrottleMultiplier(gain as f32))
                .await;
        }

        ticker.next().await;
    }
}
