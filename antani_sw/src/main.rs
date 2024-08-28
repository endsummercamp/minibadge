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
use embassy_time::with_timeout;
use embassy_time::Instant;
use embassy_time::{Duration, Ticker, Timer};

use embassy_rp::bind_interrupts;
use heapless::Vec;
use infrared::{protocol::Nec, Receiver};
use panic_probe as _;

mod rgbeffects;
mod scenes;
mod ws2812;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
});

use rand::rngs::SmallRng;
use rand::SeedableRng;
use rgbeffects::ColorPalette;
use rgbeffects::FragmentShader;
use rgbeffects::Pattern;
use rgbeffects::RenderCommand;
use rgbeffects::RenderManager;
use smart_leds::RGB8;
use ws2812::Ws2812;

const LED_MATRIX_WIDTH: usize = 3;
const LED_MATRIX_HEIGHT: usize = 3;
const LED_MATRIX_SIZE: usize = LED_MATRIX_WIDTH * LED_MATRIX_HEIGHT;

#[derive(Clone, Copy, Default)]
struct RawFramebuffer<T>
where
    T: Default + Copy,
{
    framebuffer: [T; LED_MATRIX_SIZE],
}

impl<T> RawFramebuffer<T>
where
    T: Default + Copy,
{
    fn new() -> Self {
        Self {
            framebuffer: [T::default(); LED_MATRIX_SIZE],
        }
    }

    fn set_pixel(&mut self, x: usize, y: usize, colour: T) {
        if x < LED_MATRIX_WIDTH && y < LED_MATRIX_HEIGHT {
            self.framebuffer[y * LED_MATRIX_WIDTH + x] = colour;
        }
    }

    fn get_pixel(&self, x: usize, y: usize) -> T {
        if x < LED_MATRIX_WIDTH && y < LED_MATRIX_HEIGHT {
            self.framebuffer[y * LED_MATRIX_WIDTH + x]
        } else {
            T::default()
        }
    }

    fn set_all(&mut self, rgb: T) {
        for i in 0..LED_MATRIX_SIZE {
            self.framebuffer[i] = rgb;
        }
    }

    fn get_raw(&self) -> &[T; LED_MATRIX_SIZE] {
        &self.framebuffer
    }
}

struct LedMatrix {
    raw_framebuffer: RawFramebuffer<RGB8>,
    gamma_corrected_framebuffer: RawFramebuffer<RGB8>,
    corrected_gain: f32,
    raw_gain: f32,
}

impl LedMatrix {
    fn new() -> Self {
        Self {
            raw_framebuffer: RawFramebuffer::new(),
            gamma_corrected_framebuffer: RawFramebuffer::new(),
            corrected_gain: 1.0,
            raw_gain: 1.0,
        }
    }

    fn set_gain(&mut self, gain: f32) {
        self.corrected_gain = gain;
    }

    fn set_raw_gain(&mut self, gain: f32) {
        self.raw_gain = gain;
    }

    fn get_pixel(&self, x: usize, y: usize) -> RGB8 {
        self.raw_framebuffer.get_pixel(x, y)
    }

    fn set_pixel(&mut self, x: usize, y: usize, colour: RGB8) {
        self.raw_framebuffer.set_pixel(x, y, colour);
    }

    fn update_gamma_correction_and_gain(&mut self) {
        static GAMMA_CORRECTION: [u8; 256] = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 4, 4,
            4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10, 10, 11, 11, 11,
            12, 12, 13, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22,
            22, 23, 24, 24, 25, 25, 26, 27, 27, 28, 29, 29, 30, 31, 32, 32, 33, 34, 35, 35, 36, 37,
            38, 39, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 50, 51, 52, 54, 55, 56, 57, 58,
            59, 60, 61, 62, 63, 64, 66, 67, 68, 69, 70, 72, 73, 74, 75, 77, 78, 79, 81, 82, 83, 85,
            86, 87, 89, 90, 92, 93, 95, 96, 98, 99, 101, 102, 104, 105, 107, 109, 110, 112, 114,
            115, 117, 119, 120, 122, 124, 126, 127, 129, 131, 133, 135, 137, 138, 140, 142, 144,
            146, 148, 150, 152, 154, 156, 158, 160, 162, 164, 167, 169, 171, 173, 175, 177, 180,
            182, 184, 186, 189, 191, 193, 196, 198, 200, 203, 205, 208, 210, 213, 215, 218, 220,
            223, 225, 228, 231, 233, 236, 239, 241, 244, 247, 249, 252, 255,
        ];

        for i in 0..LED_MATRIX_SIZE {
            let colour = self.raw_framebuffer.framebuffer[i];

            let colour = RGB8 {
                r: (colour.r as f32 * self.corrected_gain) as u8,
                g: (colour.g as f32 * self.corrected_gain) as u8,
                b: (colour.b as f32 * self.corrected_gain) as u8,
            };

            let colour = RGB8 {
                r: GAMMA_CORRECTION[colour.r as usize],
                g: GAMMA_CORRECTION[colour.g as usize],
                b: GAMMA_CORRECTION[colour.b as usize],
            };

            let colour = RGB8 {
                r: (colour.r as f32 * self.raw_gain) as u8,
                g: (colour.g as f32 * self.raw_gain) as u8,
                b: (colour.b as f32 * self.raw_gain) as u8,
            };

            self.gamma_corrected_framebuffer.framebuffer[i] = colour;
        }
    }

    fn set_all(&mut self, rgb: RGB8) {
        self.raw_framebuffer.set_all(rgb);
    }

    fn get_gamma_corrected(&mut self) -> &[RGB8; LED_MATRIX_SIZE] {
        self.update_gamma_correction_and_gain();

        self.gamma_corrected_framebuffer.get_raw()
    }

    fn clear(&mut self) {
        self.set_all((0, 0, 0).into());
    }
}

type AppSender = Sender<'static, CriticalSectionRawMutex, TaskCommand, 8>;
enum TaskCommand {
    ThermalThrottleMultiplier(f32), // 1.0 = no throttle, 0.0 = full throttle
    IrCommand(u8, u8, bool),        // add, cmd, repeat
    ShortButtonPress,
    LongButtonPress,
}
static CHANNEL: Channel<CriticalSectionRawMutex, TaskCommand, 8> = Channel::new();

enum HighLevelCommand {
    NextPattern,
    IncreaseBrightness,
    DecreaseBrightness,
}

// if we need to override the normal rendering with a special effect, we use this enum
#[derive(Clone)]
enum WorkingMode {
    Normal,                             // normal rendering, user selecting the patterns etc
    Special(RenderCommand), // override normal rendering until the user presses the button
    SpecialTimeout(RenderCommand, f64), // override normal rendering until the timeout
}
#[derive(Clone)]
enum OutputPower {
    High,
    Medium,
    Low,
    NighMode,
}

impl OutputPower {
    fn increase(&self) -> Self {
        match self {
            OutputPower::High => OutputPower::NighMode,
            OutputPower::Medium => OutputPower::High,
            OutputPower::Low => OutputPower::Medium,
            OutputPower::NighMode => OutputPower::Low,
        }
    }

    fn decrease(&self) -> Self {
        match self {
            OutputPower::High => OutputPower::Medium,
            OutputPower::Medium => OutputPower::Low,
            OutputPower::Low => OutputPower::NighMode,
            OutputPower::NighMode => OutputPower::High,
        }
    }
}

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

    let mut renderman = RenderManager {
        mtrx: LedMatrix::new(),
        rng: SmallRng::seed_from_u64(69420),
        persistent_data: Default::default(),
    };
    let mut ws2812 = Ws2812::new(&mut common, sm0, p.DMA_CH0, p.PIN_19);

    let patterns = scenes::PATTERNS.get();

    // override normal rendering with a special effect, if needed
    let mut working_mode = WorkingMode::SpecialTimeout(
        RenderCommand {
            effect: Pattern::Animation(
                patterns.boot_animation,
                (patterns.boot_animation.len() as f32) * 2.0,
            ),
            color: ColorPalette::Rainbow(1.0),
            pattern_shaders: Vec::from_slice(&[FragmentShader::LowPassWithPeak(50.0)]).unwrap(),
            ..Default::default()
        },
        0.5,
    );

    let mut scene_id = 0;
    let mut out_power = OutputPower::Medium;

    // let mut ir_blaster = pins.gpio11.into_push_pull_output();
    let ir_sensor = Input::new(p.PIN_10, Pull::None);
    let mut user_button = Input::new(p.PIN_9, Pull::Up);

    // if we start with the button pressed, function as a torch light
    if user_button.is_low() {
        Timer::after_millis(100).await;
        out_power = OutputPower::High; // just to not forget to put this at the max value

        working_mode = WorkingMode::Special(RenderCommand {
            effect: Pattern::Simple(patterns.all_on),
            color: ColorPalette::Solid((255, 255, 255).into()),
            ..Default::default()
        });

        user_button.wait_for_high().await;
    }

    unwrap!(spawner.spawn(button_driver(user_button, CHANNEL.sender())));

    interrupt::SWI_IRQ_1.set_priority(Priority::P3);
    let highpriority_spawner = EXECUTOR_HIGH.start(interrupt::SWI_IRQ_1);
    unwrap!(highpriority_spawner.spawn(ir_receiver(ir_sensor, CHANNEL.sender())));

    let scenes = scenes::scenes();

    println!("Starting loop");

    let mut timer_offset = 0.0;
    loop {
        //t = timer.get_counter().ticks() as f64 / 1_000_000.0;
        let t = Instant::now().as_micros() as f64 / 1_000_000.0 - timer_offset;

        match out_power {
            OutputPower::High => renderman.mtrx.set_gain(1.0),
            OutputPower::Medium => renderman.mtrx.set_gain(0.7),
            OutputPower::Low => renderman.mtrx.set_gain(0.5),
            OutputPower::NighMode => renderman.mtrx.set_gain(0.25),
        }

        let mut hlcommand = None;

        if let Ok(ch) = CHANNEL.try_receive() {
            match ch {
                TaskCommand::ThermalThrottleMultiplier(gain) => {
                    renderman.mtrx.set_raw_gain(gain);
                    println!("Thermal throttle multiplier: {}", gain);
                }
                TaskCommand::IrCommand(addr, cmd, repeat) => {
                    println!("IR command: {} {} {}", addr, cmd, repeat);

                    match (addr, cmd, repeat) {
                        (0, 70, false) => {
                            hlcommand = Some(HighLevelCommand::DecreaseBrightness);
                        }
                        (0, 69, false) => {
                            hlcommand = Some(HighLevelCommand::IncreaseBrightness);
                        }

                        (0, 71, false) => { // off
                        }

                        (0, 67, false) => {
                            // on
                            // this is used to sync clocks between multiple devices
                            timer_offset = Instant::now().as_micros() as f64 / 1_000_000.0;
                        }

                        (0, 68, false) => {
                            // animations
                            hlcommand = Some(HighLevelCommand::NextPattern);
                        }

                        _ => {}
                    }
                }
                TaskCommand::ShortButtonPress => {
                    println!("Short button press");
                    hlcommand = Some(HighLevelCommand::NextPattern);
                }
                TaskCommand::LongButtonPress => {
                    println!("Long button press");
                    hlcommand = Some(HighLevelCommand::DecreaseBrightness);
                }
            }
        }

        match hlcommand {
            Some(HighLevelCommand::NextPattern) => {
                if let WorkingMode::Normal = working_mode {
                    scene_id = (scene_id + 1) % scenes.len();
                } else {
                    working_mode = WorkingMode::Normal;
                }
            }

            Some(HighLevelCommand::IncreaseBrightness)
            | Some(HighLevelCommand::DecreaseBrightness) => {
                if let Some(HighLevelCommand::DecreaseBrightness) = hlcommand {
                    out_power = out_power.decrease();
                } else {
                    out_power = out_power.increase();
                }

                let patt = match out_power {
                    OutputPower::High => patterns.power_100,
                    OutputPower::Medium => patterns.power_75,
                    OutputPower::Low => patterns.power_50,
                    OutputPower::NighMode => patterns.power_25,
                };

                working_mode = WorkingMode::SpecialTimeout(
                    RenderCommand {
                        effect: Pattern::Simple(patt),
                        color: ColorPalette::Solid((255, 255, 255).into()),
                        ..Default::default()
                    },
                    t + 1.0,
                );
            }

            None => {}
        }

        match &working_mode {
            WorkingMode::Normal => {
                renderman.render(&scenes[scene_id], t);
            }
            WorkingMode::SpecialTimeout(scene, timeout) => {
                renderman.render(&[scene.clone()], t);

                if t > *timeout {
                    working_mode = WorkingMode::Normal;
                }
            }
            WorkingMode::Special(scene) => {
                renderman.render(&[scene.clone()], t);
            }
        }

        ws2812.write(renderman.mtrx.get_gamma_corrected()).await;
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
    let mut int_receiver: Receiver<Nec, embassy_rp::gpio::Input> = Receiver::builder()
        .rc5()
        .frequency(1_000_000)
        .pin(ir_sensor)
        .protocol()
        .build();

    loop {
        int_receiver.pin_mut().wait_for_any_edge().await;

        if let Ok(Some(cmd)) = int_receiver.event_instant(Instant::now().as_ticks() as u32) {
            control
                .send(TaskCommand::IrCommand(cmd.addr, cmd.cmd, cmd.repeat))
                .await;
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
                .send(TaskCommand::ThermalThrottleMultiplier(gain as f32))
                .await;
        }

        ticker.next().await;
    }
}

#[embassy_executor::task]
async fn button_driver(mut button: Input<'static>, control: AppSender) {
    let mut press_start;

    loop {
        button.wait_for_low().await;
        press_start = Instant::now();

        match with_timeout(Duration::from_millis(1000), button.wait_for_high()).await {
            // no timeout
            Ok(_) => {}
            // timeout
            Err(_) => {
                control.send(TaskCommand::LongButtonPress).await;
                button.wait_for_high().await;
            }
        }

        let press_duration = Instant::now() - press_start;

        if press_duration >= Duration::from_millis(50)
            && press_duration < Duration::from_millis(1000)
        {
            control.send(TaskCommand::ShortButtonPress).await;
        }
    }
}
