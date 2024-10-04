use embassy_sync::lazy_lock::LazyLock;
use heapless::Vec;

use crate::rgbeffects::{ColorPalette, FragmentShader, LedPattern, Pattern, RenderCommand};

pub struct Patterns {
    pub power_100: LedPattern,
    pub power_75: LedPattern,
    pub power_50: LedPattern,
    pub power_25: LedPattern,
    pub glider: LedPattern,
    pub all_on: LedPattern,
    pub vertical_stripe_1: LedPattern,
    pub vertical_stripe_2: LedPattern,
    pub vertical_stripe_3: LedPattern,
    pub dice: &'static [LedPattern],
    pub font: &'static [LedPattern],
    pub everything_once: &'static [LedPattern],
    pub boot_animation: &'static [LedPattern],
}

pub static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| Patterns {
    // patterns for light power
    power_100: 0b111111111,
    power_75: 0b000111111,
    power_50: 0b000000111,
    power_25: 0b000000001,

    glider: 0b010001111,
    all_on: 0b111111111,
    vertical_stripe_1: 0b100100100,
    vertical_stripe_2: 0b010010010,
    vertical_stripe_3: 0b001001001,

    dice: &[
        0b000010000,
        0b100000001,
        0b100010001,
        0b101000101,
        0b101010101,
        0b101101101,
    ],

    font: &[
        0b010111101, // 'A'
        0b110111111,
        0b011100111,
        0b110101110,
        0b111110111,
        0b111110100,
        0b110101111,
        0b101111101,
        0b111010111,
        0b111010110,
        0b101110101,
        0b100100111,
        0b111111101,
        0b111101101,
        0b111101111,
        0b111111100,
        0b111101110,
        0b110111101,
        0b011010110,
        0b111010010,
        0b101101111,
        0b101101010,
        0b101111111,
        0b101010101,
        0b101010010,
        0b110010011, // 'Z'
    ],

    everything_once: &[
        0b100000000,
        0b010000000,
        0b001000000,
        0b000100000,
        0b000010000,
        0b000001000,
        0b000000100,
        0b000000010,
        0b000000001,
    ],
    boot_animation: &[
        0b010000000,
        0b010010000,
        0b111111000,
        0b000111111,
        0b000000111,
        0b000000010,
        0b000000000,
        0b000000000,
        0b000000000,
        0b000000000,
    ],
});

pub type Scenes = Vec<Vec<RenderCommand, 8>, 20>;
pub fn scenes() -> Scenes {
    let patterns = PATTERNS.get();

    Vec::from_slice(&[
        // normal glider
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.glider),
            color: ColorPalette::Solid((0, 0, 255).into()),
            ..Default::default()
        }])
        .unwrap(),
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.glider),
            color: ColorPalette::Solid((0, 255, 0).into()),
            ..Default::default()
        }])
        .unwrap(),
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.glider),
            color: ColorPalette::Solid((255, 0, 0).into()),
            ..Default::default()
        }])
        .unwrap(),
        // breathing glider
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.glider),
            color: ColorPalette::Solid((0, 0, 255).into()),
            pattern_shaders: Vec::from_slice(&[FragmentShader::Breathing(0.7)]).unwrap(),
            ..Default::default()
        }])
        .unwrap(),
        // italy flag
        Vec::from_slice(&[
            RenderCommand {
                effect: Pattern::Simple(patterns.vertical_stripe_1),
                color: ColorPalette::Solid((0, 255, 0).into()),
                ..Default::default()
            },
            RenderCommand {
                effect: Pattern::Simple(patterns.vertical_stripe_2),
                color: ColorPalette::Solid((255, 255, 255).into()),
                ..Default::default()
            },
            RenderCommand {
                effect: Pattern::Simple(patterns.vertical_stripe_3),
                color: ColorPalette::Solid((255, 0, 0).into()),
                ..Default::default()
            },
        ])
        .unwrap(),
        // single rainbow glider
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.glider),
            pattern_shaders: Vec::from_slice(&[FragmentShader::Rainbow2D(0.5)]).unwrap(),
            ..Default::default()
        }])
        .unwrap(),
        // rainbow 2d
        Vec::from_slice(&[RenderCommand {
            screen_shaders: Vec::from_slice(&[FragmentShader::Rainbow2D(0.5)]).unwrap(),
            ..Default::default()
        }])
        .unwrap(),
        // solid red
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.all_on),
            color: ColorPalette::Solid((255, 0, 0).into()),
            ..Default::default()
        }])
        .unwrap(),
        // solid green
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.all_on),
            color: ColorPalette::Solid((0, 255, 0).into()),
            ..Default::default()
        }])
        .unwrap(),
        // solid blue
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.all_on),
            color: ColorPalette::Solid((0, 0, 255).into()),
            ..Default::default()
        }])
        .unwrap(),
        // solid white
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.all_on),
            color: ColorPalette::Solid((255, 255, 255).into()),
            ..Default::default()
        }])
        .unwrap(),
        // police lights
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(patterns.all_on),
            color: ColorPalette::Custom(
                Vec::from_slice(&[
                    (0, 0, 0).into(),
                    (255, 0, 0).into(),
                    (0, 0, 0).into(),
                    (255, 0, 0).into(),
                    (0, 0, 0).into(),
                    (0, 0, 0).into(),
                    (0, 0, 0).into(),
                    (0, 0, 0).into(),
                    (0, 0, 255).into(),
                    (0, 0, 0).into(),
                    (0, 0, 255).into(),
                    (0, 0, 0).into(),
                    (0, 0, 0).into(),
                    (0, 0, 0).into(),
                    (0, 0, 0).into(),
                ])
                .unwrap(),
                15.0,
            ),
            ..Default::default()
        }])
        .unwrap(),
        // dice
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Animation(patterns.dice, 0.5),
            color: ColorPalette::Solid((255, 0, 0).into()),
            ..Default::default()
        }])
        .unwrap(),
        // "ESC"
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Text("ESC ", 2.0),
            color: ColorPalette::Rainbow(0.5),
            ..Default::default()
        }])
        .unwrap(),
        // alphabet
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Animation(patterns.font, 2.0),
            color: ColorPalette::Rainbow(0.5),
            ..Default::default()
        }])
        .unwrap(),
        // off
        Vec::from_slice(&[RenderCommand {
            effect: Pattern::Simple(0),
            color: ColorPalette::Solid((0, 0, 0).into()),
            ..Default::default()
        }])
        .unwrap(),
    ])
    .unwrap()
}
