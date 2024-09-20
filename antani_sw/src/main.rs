#![no_std]
#![no_main]

use core::f64;

use defmt::println;
use defmt::unwrap;
use embassy_executor::Executor;
use embassy_rp::adc;
use embassy_rp::gpio::Input;
use embassy_rp::gpio::Output;
use embassy_rp::gpio::Pin;
use embassy_rp::gpio::Pull;
use embassy_rp::multicore::spawn_core1;
use embassy_rp::multicore::Stack;
use embassy_sync::pubsub::PubSubChannel;
use embassy_sync::pubsub::Publisher;
use embassy_sync::signal::Signal;
use log::{info, warn};

use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::pwm;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_time::with_timeout;
use embassy_time::Instant;
use embassy_time::{Duration, Ticker, Timer};

use embassy_rp::bind_interrupts;
use heapless::Vec;
use infrared::{protocol::Nec, protocol::SamsungNec, Receiver};
use panic_probe as _;

mod capnp;
mod rgbeffects;
mod scenes;
mod usb;
mod ws2812;

pub mod usb_messages_capnp {
    include!(concat!(env!("OUT_DIR"), "/usb_messages_capnp.rs"));
}

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
use scenes::Scenes;
use static_cell::StaticCell;
use ws2812::Ws2812;

// global constants
const LED_MATRIX_WIDTH: usize = 3;
const LED_MATRIX_HEIGHT: usize = 3;
const LED_MATRIX_SIZE: usize = LED_MATRIX_WIDTH * LED_MATRIX_HEIGHT;
/// set to true if RGBW leds, false if RGB
pub const HAS_WHITE_LED: bool = false;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct LedPixel {
    r: u8,
    g: u8,
    b: u8,
    w: u8,
}

impl LedPixel {
    fn set_white(&mut self) {
        // create white channel from rgb
        if self.r == self.g && self.g == self.b {
            self.w = self.r;
            self.r = 0;
            self.g = 0;
            self.b = 0;
        }
    }
}

impl From<(u8, u8, u8)> for LedPixel {
    fn from(rgb: (u8, u8, u8)) -> Self {
        Self {
            r: rgb.0,
            g: rgb.1,
            b: rgb.2,
            w: 0,
        }
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct RawFramebuffer {
    framebuffer: [LedPixel; LED_MATRIX_SIZE],
}

impl RawFramebuffer {
    fn new() -> Self {
        Self {
            framebuffer: [LedPixel::default(); LED_MATRIX_SIZE],
        }
    }

    fn set_pixel(&mut self, x: usize, y: usize, colour: LedPixel) {
        if x < LED_MATRIX_WIDTH && y < LED_MATRIX_HEIGHT {
            let color = LedPixel {
                r: colour.r,
                g: colour.g,
                b: colour.b,
                w: 0,
            };
            self.framebuffer[y * LED_MATRIX_WIDTH + x] = color;
        }
    }

    fn get_pixel(&self, x: usize, y: usize) -> LedPixel {
        if x < LED_MATRIX_WIDTH && y < LED_MATRIX_HEIGHT {
            self.framebuffer[y * LED_MATRIX_WIDTH + x]
        } else {
            LedPixel::default()
        }
    }

    fn set_all(&mut self, rgb: LedPixel) {
        self.framebuffer.iter_mut().for_each(|led| *led = rgb);
    }
    fn update_rgbw(&mut self) {
        self.framebuffer.iter_mut().for_each(|led| led.set_white());
    }

    fn get_raw(&self) -> &[LedPixel; LED_MATRIX_SIZE] {
        &self.framebuffer
    }
}

struct LedMatrix {
    raw_framebuffer: RawFramebuffer,
    gamma_corrected_framebuffer: RawFramebuffer,
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

    fn get_pixel(&self, x: usize, y: usize) -> LedPixel {
        self.raw_framebuffer.get_pixel(x, y)
    }

    fn set_pixel(&mut self, x: usize, y: usize, colour: LedPixel) {
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

            let colour = LedPixel {
                r: (GAMMA_CORRECTION[(colour.r as f32 * self.corrected_gain) as usize] as f32
                    * self.raw_gain) as u8,
                g: (GAMMA_CORRECTION[(colour.g as f32 * self.corrected_gain) as usize] as f32
                    * self.raw_gain) as u8,
                b: (GAMMA_CORRECTION[(colour.b as f32 * self.corrected_gain) as usize] as f32
                    * self.raw_gain) as u8,
                w: (GAMMA_CORRECTION[(colour.w as f32 * self.corrected_gain) as usize] as f32
                    * self.raw_gain) as u8,
            };

            self.gamma_corrected_framebuffer.framebuffer[i] = colour;
        }
    }

    fn set_all(&mut self, rgb: LedPixel) {
        self.raw_framebuffer.set_all(rgb);
    }

    fn get_gamma_corrected(&mut self) -> &[LedPixel; LED_MATRIX_SIZE] {
        self.update_gamma_correction_and_gain();

        if HAS_WHITE_LED {
            self.gamma_corrected_framebuffer.update_rgbw();
        }
        self.gamma_corrected_framebuffer.get_raw()
    }

    fn clear(&mut self) {
        self.set_all((0, 0, 0).into());
    }
}

#[derive(Clone, Debug)]
enum TaskCommand {
    ThermalThrottleMultiplier(f32), // 1.0 = no throttle, 0.0 = full throttle
    ReceivedIrNec(u8, u8, bool),    // add, cmd, repeat
    ShortButtonPress,
    LongButtonPress,
    MidiSetPixel(u8, u8, u8, u8), // x y channel (0=r 1=g 2=b) value
    SetWorkingMode(WorkingMode),
    SendIrNec(u8, u8, bool),
    IrTxDone,
    NextPattern,
    IncreaseBrightness,
    DecreaseBrightness,
    SetBrightness(OutputPower),
    ResetTime,
    UsbActivity,
    SendHidKeyboard(usbd_hid::descriptor::KeyboardUsage),
    Error,
    None,
}

static MEGA_CHANNEL: PubSubChannel<CriticalSectionRawMutex, TaskCommand, 8, 8, 8> =
    PubSubChannel::new();
type MegaPublisher = Publisher<'static, CriticalSectionRawMutex, TaskCommand, 8, 8, 8>;
type MegaSubscriber =
    embassy_sync::pubsub::Subscriber<'static, CriticalSectionRawMutex, TaskCommand, 8, 8, 8>;

// if we need to override the normal rendering with a special effect, we use this enum
#[derive(Clone, Debug)]
enum WorkingMode {
    Normal,                             // normal rendering, user selecting the patterns etc
    Special(RenderCommand), // override normal rendering until the user presses the button
    SpecialTimeout(RenderCommand, f64), // override normal rendering until the timeout
    RawFramebuffer(RawFramebuffer),
}
#[derive(Clone, Debug)]
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

enum WhiteLedCommand {
    Communication,
    Error,
}

static WHITE_LED_SIGNAL: Signal<CriticalSectionRawMutex, WhiteLedCommand> = Signal::new();

static mut CORE1_STACK: Stack<8192> = Stack::new();
static EXECUTOR0: StaticCell<Executor> = StaticCell::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

#[cortex_m_rt::entry]
fn main() -> ! {
    let p = embassy_rp::init(Default::default());

    let executor0 = EXECUTOR0.init(Executor::new());

    // ADC / temperature sensor
    let adc = adc::Adc::new(p.ADC, Irqs, adc::Config::default());
    let ts = adc::Channel::new_temp_sensor(p.ADC_TEMP_SENSOR);

    // button

    let user_btn = Input::new(p.PIN_8, Pull::Up);

    // white led
    let white_led = Output::new(p.PIN_20, embassy_rp::gpio::Level::Low);

    // infrared stuff
    let _ir_sens_0 = Input::new(p.PIN_9, Pull::None);

    let mut pwm_cfg: pwm::Config = Default::default();
    pwm_cfg.enable = false;
    let ir_blaster = pwm::Pwm::new_output_b(p.PWM_SLICE5, p.PIN_11, pwm_cfg);

    // leds
    let Pio {
        mut common, sm0, ..
    } = Pio::new(p.PIO0, Irqs);

    let ws2812: Ws2812<'_, PIO0, 0, 9> = Ws2812::new(&mut common, sm0, p.DMA_CH0, p.PIN_19);

    // scenes
    let scenes = scenes::scenes();
    // this is safe because this thread will always be running
    // it's still an hack and it should be changed in some way
    // the problem is that the scene array is GIANT and it's difficult to process in a task
    let scenes = unsafe { core::mem::transmute::<&Scenes, &'static Scenes>(&scenes) };

    spawn_core1(
        p.CORE1,
        unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) },
        move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1.run(|spawner| unwrap!(spawner.spawn(main_tsk(ws2812, scenes))));
        },
    );

    executor0.run(|spawner| {
        unwrap!(spawner.spawn(temperature(adc, ts, MEGA_CHANNEL.publisher().unwrap())));
        unwrap!(spawner.spawn(usb::usb_main(
            p.USB,
            MEGA_CHANNEL.publisher().unwrap(),
            MEGA_CHANNEL.subscriber().unwrap()
        )));
        unwrap!(spawner.spawn(button_tsk(user_btn, MEGA_CHANNEL.publisher().unwrap())));
        unwrap!(spawner.spawn(white_led_task(white_led)));
        unwrap!(spawner.spawn(ir_receiver(
            p.PIN_10.pin(),
            MEGA_CHANNEL.publisher().unwrap()
        )));

        unwrap!(spawner.spawn(ir_blaster_tsk(
            ir_blaster,
            MEGA_CHANNEL.subscriber().unwrap(),
            MEGA_CHANNEL.publisher().unwrap()
        )));
    });
}

#[embassy_executor::task]
async fn main_tsk(mut ws2812: Ws2812<'static, PIO0, 0, 9>, scenes: &'static Scenes) {
    info!("Program start");
    println!("Program start");

    let mut midi_framebuffer = RawFramebuffer::new();

    let mut renderman = RenderManager {
        mtrx: LedMatrix::new(),
        rng: SmallRng::seed_from_u64(69420),
        persistent_data: Default::default(),
    };

    let patterns = scenes::PATTERNS.get();

    let boot_animation = RenderCommand {
        effect: Pattern::Animation(
            patterns.boot_animation,
            (patterns.boot_animation.len() as f32) * 2.0,
        ),
        color: ColorPalette::Rainbow(1.0),
        pattern_shaders: Vec::from_slice(&[FragmentShader::LowPassWithPeak(50.0)]).unwrap(),
        ..Default::default()
    };
    // override normal rendering with a special effect, if needed
    let mut working_mode = WorkingMode::SpecialTimeout(boot_animation.clone(), 0.5);

    let mut scene_id = 0;
    let mut out_power = OutputPower::High;

    let mut is_transmitting = false;

    let mega_publisher = match MEGA_CHANNEL.publisher() {
        Ok(p) => p,
        Err(e) => {
            println!("Error getting publisher: {:?}", e);
            panic!("Error getting publisher");
        }
    };

    let mut mega_subscriber = match MEGA_CHANNEL.subscriber() {
        Ok(p) => p,
        Err(e) => {
            println!("Error getting subscriber: {:?}", e);
            panic!("Error getting subscriber");
        }
    };

    info!("Starting loop");
    mega_publisher
        .publish(TaskCommand::SendIrNec(0, 66, false))
        .await;

    let mut ticker = Ticker::every(Duration::from_hz(100));

    let mut timer_offset = 0.0;
    loop {
        let t = Instant::now().as_micros() as f64 / 1_000_000.0 - timer_offset;

        match out_power {
            OutputPower::High => renderman.mtrx.set_gain(1.0),
            OutputPower::Medium => renderman.mtrx.set_gain(0.7),
            OutputPower::Low => renderman.mtrx.set_gain(0.5),
            OutputPower::NighMode => renderman.mtrx.set_gain(0.25),
        }

        if let Some(message) = mega_subscriber.try_next_message_pure() {
            info!("Handling message: {:?}", message);
            match message {
                TaskCommand::ThermalThrottleMultiplier(gain) => {
                    renderman.mtrx.set_raw_gain(gain);
                    if gain < 1.0 {
                        warn!("Thermal throttling! {}", gain);
                    }
                }
                TaskCommand::ReceivedIrNec(addr, cmd, repeat) => {
                    if is_transmitting {
                        warn!("Ignoring IR command, we are transmitting");
                        continue;
                    }

                    match (addr, cmd, repeat) {
                        // all those are commands of the chinese ir rgb remote
                        (0, 70, false) => {
                            mega_publisher
                                .publish(TaskCommand::DecreaseBrightness)
                                .await;
                        }
                        (0, 69, false) => {
                            mega_publisher
                                .publish(TaskCommand::IncreaseBrightness)
                                .await;
                        }

                        (0, 71, false) => { // off
                        }

                        (0, 67, false) => {
                            // on
                            // this is used to sync clocks between multiple devices
                            mega_publisher.publish(TaskCommand::ResetTime).await;
                        }

                        (0, 68, false) => {
                            // animations
                            mega_publisher.publish(TaskCommand::NextPattern).await;
                        }
                        // END of ir command from the chinese remote

                        // startup ir command sent by another badge
                        // say hi to the other badge
                        (0, 66, false) => {
                            // we do this so the animation starts in the correct time
                            mega_publisher.publish(TaskCommand::ResetTime).await;

                            mega_publisher
                                .publish(TaskCommand::SetWorkingMode(WorkingMode::SpecialTimeout(
                                    boot_animation.clone(),
                                    0.5,
                                )))
                                .await;
                        }

                        // samsung tv remote
                        // volume up
                        (7, 7, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardVolumeUp,
                                ))
                                .await;
                        }
                        // volume down
                        (7, 11, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardVolumeDown,
                                ))
                                .await;
                        }
                        //arrow right
                        (7, 98, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardRightArrow,
                                ))
                                .await;
                        }
                        // left
                        (7, 101, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardLeftArrow,
                                ))
                                .await;
                        }
                        // up
                        (7, 96, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardUpArrow,
                                ))
                                .await;
                        }
                        // down
                        (7, 97, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardDownArrow,
                                ))
                                .await;
                        }
                        // exit
                        (7, 102, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardEscape,
                                ))
                                .await;
                        }
                        // enter
                        (7, 104, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardEnter,
                                ))
                                .await;
                        }
                        // 1
                        (7, 4, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard1Exclamation,
                                ))
                                .await;
                        }
                        // 2
                        (7, 5, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard2At,
                                ))
                                .await;
                        }
                        // 3
                        (7, 6, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard3Hash,
                                ))
                                .await;
                        }
                        // 4
                        (7, 8, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard4Dollar,
                                ))
                                .await;
                        }
                        // 5
                        (7, 9, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard5Percent,
                                ))
                                .await;
                        }
                        // 6
                        (7, 10, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard6Caret,
                                ))
                                .await;
                        }
                        // 7
                        (7, 12, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard7Ampersand,
                                ))
                                .await;
                        }
                        // 8
                        (7, 13, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard8Asterisk,
                                ))
                                .await;
                        }
                        // 9
                        (7, 14, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::Keyboard9OpenParens,
                                ))
                                .await;
                        }
                        // mute
                        (7, 15, false) => {
                            mega_publisher
                                .publish(TaskCommand::SendHidKeyboard(
                                    usbd_hid::descriptor::KeyboardUsage::KeyboardMute,
                                ))
                                .await;
                        }

                        _ => {}
                    }
                    WHITE_LED_SIGNAL.signal(WhiteLedCommand::Communication);
                }
                TaskCommand::ShortButtonPress => {
                    mega_publisher.publish(TaskCommand::NextPattern).await;
                }
                TaskCommand::LongButtonPress => {
                    mega_publisher
                        .publish(TaskCommand::DecreaseBrightness)
                        .await;
                }

                TaskCommand::MidiSetPixel(x, y, channel, value) => {
                    let px = midi_framebuffer.get_pixel(x as usize, y as usize);

                    let rgb = match channel {
                        0 => (value, px.g, px.b).into(),
                        1 => (px.r, value, px.b).into(),
                        2 => (px.r, px.g, value).into(),
                        _ => px,
                    };

                    midi_framebuffer.set_pixel(x as usize, y as usize, rgb);

                    working_mode = WorkingMode::RawFramebuffer(midi_framebuffer);
                    WHITE_LED_SIGNAL.signal(WhiteLedCommand::Communication);
                }

                TaskCommand::SendIrNec(_, _, _) => {
                    is_transmitting = true;
                }

                TaskCommand::IrTxDone => {
                    is_transmitting = false;
                }

                TaskCommand::NextPattern => {
                    if let WorkingMode::Normal = working_mode {
                        scene_id = (scene_id + 1) % scenes.len();
                    } else {
                        working_mode = WorkingMode::Normal;
                    }
                }

                TaskCommand::IncreaseBrightness | TaskCommand::DecreaseBrightness => {
                    if let TaskCommand::DecreaseBrightness = message {
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

                    // do not ruin the midi framebuffer
                    if !matches!(working_mode, WorkingMode::RawFramebuffer(_)) {
                        working_mode = WorkingMode::SpecialTimeout(
                            RenderCommand {
                                effect: Pattern::Simple(patt),
                                color: ColorPalette::Solid((255, 255, 255).into()),
                                ..Default::default()
                            },
                            t + 1.0,
                        );
                    }
                }

                TaskCommand::SetWorkingMode(wm) => {
                    working_mode = wm;
                }

                TaskCommand::ResetTime => {
                    timer_offset = Instant::now().as_micros() as f64 / 1_000_000.0;
                }

                TaskCommand::SetBrightness(b) => {
                    out_power = b;
                }

                TaskCommand::UsbActivity => {
                    WHITE_LED_SIGNAL.signal(WhiteLedCommand::Communication);
                }

                TaskCommand::Error => {
                    WHITE_LED_SIGNAL.signal(WhiteLedCommand::Error);
                }

                TaskCommand::None | TaskCommand::SendHidKeyboard(_) => {}
            }
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
            WorkingMode::RawFramebuffer(fb) => {
                renderman.mtrx.raw_framebuffer = *fb;
            }
        }

        ws2812.write(renderman.mtrx.get_gamma_corrected()).await;
        ticker.next().await;
        renderman.mtrx.clear();
    }
}

#[embassy_executor::task]
async fn ir_receiver(ir_sensor: u8, publisher: MegaPublisher) {
    // this is a mega hack to support the reception of two different IR protocols
    // we unsafely use the same pin for both receivers

    let mut nec_receiver: Receiver<Nec, embassy_rp::gpio::Input> = Receiver::builder()
        .rc5()
        .frequency(1_000_000)
        .pin(Input::new(
            unsafe { embassy_rp::gpio::AnyPin::steal(ir_sensor) },
            Pull::None,
        ))
        .protocol()
        .build();

    let mut samsung_receiver: Receiver<SamsungNec, embassy_rp::gpio::Input> = Receiver::builder()
        .rc5()
        .frequency(1_000_000)
        .pin(Input::new(
            unsafe { embassy_rp::gpio::AnyPin::steal(ir_sensor) },
            Pull::None,
        ))
        .protocol()
        .build();

    loop {
        samsung_receiver.pin_mut().wait_for_any_edge().await;
        let now = Instant::now().as_ticks() as u32;

        if let Ok(Some(cmd)) = samsung_receiver.event_instant(now) {
            publisher
                .publish(TaskCommand::ReceivedIrNec(cmd.addr, cmd.cmd, cmd.repeat))
                .await;
        }

        if let Ok(Some(cmd)) = nec_receiver.event_instant(now) {
            publisher
                .publish(TaskCommand::ReceivedIrNec(cmd.addr, cmd.cmd, cmd.repeat))
                .await;
        }
    }
}

#[embassy_executor::task]
async fn ir_blaster_tsk(
    mut ir_blaster: pwm::Pwm<'static>,
    mut subscriber: MegaSubscriber,
    publisher: MegaPublisher,
) {
    use infrared::sender::Status;

    fn enable_pwm(pwm: &mut pwm::Pwm, pwm_cfg: &mut pwm::Config, enable: bool) {
        pwm_cfg.enable = enable;
        pwm.set_config(pwm_cfg);

        // why the hell does the pwm pin stay high when we disable the pwm?
        unsafe {
            *((0x40014000 + 11 * 8 + 0x04) as *mut u32) = if enable { 4 } else { 0x1f };
        }
    }

    loop {
        if let TaskCommand::SendIrNec(addr, cmd, repeat) = subscriber.next_message_pure().await {
            const FREQUENCY: u32 = 20000;

            let mut buffer: infrared::sender::PulsedataSender<128> =
                infrared::sender::PulsedataSender::new();

            let cmd = infrared::protocol::nec::NecCommand { addr, cmd, repeat };
            buffer.load_command::<Nec, FREQUENCY>(&cmd);
            let mut counter = 0;

            let mut pwm_cfg: pwm::Config = Default::default();
            pwm_cfg.enable = false;
            // system clock is 125MHz
            // we need to do 38khz, so 125_000_000 / 38_000 = 3289
            pwm_cfg.top = (125_000_000 / 38_000) as u16;
            pwm_cfg.compare_b = pwm_cfg.top / 2;

            let mut ticker = Ticker::every(Duration::from_hz(FREQUENCY as u64));
            loop {
                let status: infrared::sender::Status = buffer.tick(counter);
                counter = counter.wrapping_add(1);

                match status {
                    Status::Transmit(v) => {
                        enable_pwm(&mut ir_blaster, &mut pwm_cfg, v);
                    }
                    Status::Idle => {
                        enable_pwm(&mut ir_blaster, &mut pwm_cfg, false);
                        break;
                    }
                    Status::Error => {
                        log::error!("Error in IR blaster");
                        enable_pwm(&mut ir_blaster, &mut pwm_cfg, false);
                        publisher.publish(crate::TaskCommand::Error).await;
                        break;
                    }
                };

                ticker.next().await;
            }
            log::info!("tx done");
            enable_pwm(&mut ir_blaster, &mut pwm_cfg, false);
            publisher.publish(TaskCommand::IrTxDone).await;
        }
    }
}

#[embassy_executor::task]
async fn white_led_task(mut white_led: Output<'static>) {
    loop {
        let led_state = WHITE_LED_SIGNAL.wait().await;

        match led_state {
            WhiteLedCommand::Communication => {
                white_led.set_high();
                Timer::after(Duration::from_millis(200)).await;
                white_led.set_low();
                Timer::after(Duration::from_millis(200)).await;
            }
            WhiteLedCommand::Error => {
                for _ in 0..4 {
                    white_led.set_high();
                    Timer::after(Duration::from_millis(50)).await;
                    white_led.set_low();
                    Timer::after(Duration::from_millis(50)).await;
                }
            }
        }
    }
}

#[embassy_executor::task]
async fn temperature(
    mut adc: adc::Adc<'static, adc::Async>,
    mut ts: adc::Channel<'static>,
    publisher: MegaPublisher,
) {
    let mut ticker = Ticker::every(Duration::from_secs(1));

    loop {
        let temp = match adc.read(&mut ts).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error reading temperature: {:?}", e);
                continue;
            }
        };

        // TODO: yeah let's waste precious CPU cycles to calculate the temperature before checking if we need to throttle
        let adc_voltage = (3.3 / 4096.0) * temp as f64;
        let temp_degrees_c = 27.0 - (adc_voltage - 0.706) / 0.001721;

        if temp_degrees_c > 50.0 {
            // lerp from 55 to 65 degrees maps to gain from 1.0 to 0.1
            let gain: f64 = 1.0 - (temp_degrees_c - 55.0) / 10.0;
            let gain = gain.clamp(0.0, 1.0);
            publisher
                .publish(TaskCommand::ThermalThrottleMultiplier(gain as f32))
                .await;
        }

        ticker.next().await;
    }
}

#[embassy_executor::task]
async fn button_tsk(mut button: Input<'static>, publisher: MegaPublisher) {
    // if we start with the button pressed, function as a torch light
    if button.is_low() {
        Timer::after_millis(100).await;

        publisher
            .publish(TaskCommand::SetWorkingMode(WorkingMode::Special(
                RenderCommand {
                    effect: Pattern::Simple(scenes::PATTERNS.get().all_on),
                    color: ColorPalette::Solid((255, 255, 255).into()),
                    ..Default::default()
                },
            )))
            .await;

        publisher
            .publish(TaskCommand::SetBrightness(OutputPower::High))
            .await;

        button.wait_for_high().await;
    }

    let mut press_start;

    loop {
        button.wait_for_low().await;
        press_start = Instant::now();

        match with_timeout(Duration::from_millis(1000), button.wait_for_high()).await {
            // no timeout
            Ok(_) => {}
            // timeout
            Err(_) => {
                publisher.publish(TaskCommand::LongButtonPress).await;
                button.wait_for_high().await;
            }
        }

        let press_duration = Instant::now() - press_start;

        if press_duration >= Duration::from_millis(50)
            && press_duration < Duration::from_millis(1000)
        {
            publisher.publish(TaskCommand::ShortButtonPress).await;
        }
    }
}
