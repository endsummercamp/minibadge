use embassy_sync::lazy_lock::LazyLock;
use heapless::Vec;

use crate::rgbeffects::{
    AnimationPattern, ColorPalette, FragmentShader, LedPattern, Pattern, RenderCommand,
};

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
    pub everything_once: AnimationPattern,
    pub boot_animation: AnimationPattern,
}

pub static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| Patterns {
    // patterns for light power
    power_100: LedPattern::new(0b111111111),
    power_75: LedPattern::new(0b000111111),
    power_50: LedPattern::new(0b000000111),
    power_25: LedPattern::new(0b000000001),

    glider: LedPattern::new(0b010001111),
    all_on: LedPattern::new(0b111111111),
    vertical_stripe_1: LedPattern::new(0b100100100),
    vertical_stripe_2: LedPattern::new(0b010010010),
    vertical_stripe_3: LedPattern::new(0b001001001),

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
    boot_animation: AnimationPattern::new(&[
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
    ]),
});

pub fn scenes() -> Vec<Vec<RenderCommand, 8>, 20> {
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
        // glider with glitter
        Vec::from_slice(&[
            RenderCommand {
                effect: Pattern::Simple(patterns.glider),
                color: ColorPalette::Solid((0, 0, 255).into()),
                ..Default::default()
            },
            RenderCommand {
                effect: Pattern::AnimationRandom(&patterns.everything_once, 300),
                color: ColorPalette::Rainbow(0.25),
                screen_shaders: Vec::from_slice(&[FragmentShader::LowPassWithPeak(10000.0)])
                    .unwrap(),
                ..Default::default()
            },
        ])
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
        // double rainbow glider
        Vec::from_slice(&[
            RenderCommand {
                effect: Pattern::Simple(patterns.all_on),
                color: ColorPalette::Rainbow(0.25),
                ..Default::default()
            },
            RenderCommand {
                effect: Pattern::Simple(patterns.glider),
                color: ColorPalette::Rainbow(0.25),
                time_offset: 2.0,
                ..Default::default()
            },
        ])
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
    ])
    .unwrap()
}
