use core::f64;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex};
use heapless::Vec;
use num_traits::real::Real;
use rand::{rngs::SmallRng, Rng};
use smart_leds::RGB8;

use crate::{LedMatrix, RawFramebuffer};

#[derive(Clone, Copy)]
pub struct LedPattern {
    pub pattern: u16,
}

impl LedPattern {
    pub const fn new(pattern: u16) -> Self {
        Self { pattern }
    }
}

impl From<u16> for LedPattern {
    fn from(pattern: u16) -> Self {
        Self { pattern }
    }
}

pub struct AnimationPattern {
    patterns: Vec<LedPattern, 20>,
}

impl AnimationPattern {
    pub fn new(patterns: &[u16]) -> Self {
        Self {
            patterns: patterns.iter().map(|&p| LedPattern::new(p)).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }
}

#[derive(Clone, Default)]
pub struct RenderCommand {
    pub effect: Pattern,
    pub color: ColorPalette,
    pub pattern_shaders: Vec<FragmentShader, 8>,
    pub screen_shaders: Vec<FragmentShader, 8>,
    pub time_offset: f64,
}

#[derive(Clone, Default)]
pub struct ShaderPersistentData {
    pub frame_counter: u32,
    pub lowpass: RawFramebuffer<RGB8>,
}

pub struct RenderManager {
    pub mtrx: LedMatrix,
    pub rng: SmallRng,
    pub persistent_data: ShaderPersistentData,
}

impl RenderManager {
    async fn render_single(&mut self, command: &RenderCommand, t: f64) {
        let t = t + command.time_offset;
        let startcolor = command.color.render(t);

        let pattern = command.effect.render(t, self);

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
            // if a pixel is outside of the pattern, I still expect screen-space shaders to be applied to it
            if pattern.pattern & (1 << i) != 0 {
                let mut color = startcolor;

                for shader in command.pattern_shaders.iter() {
                    color = shader.render(t, color, *x, *y, self).await;
                }

                self.mtrx.set_pixel(*x, *y, color);
            }

            for shader in command.screen_shaders.iter() {
                let mut color = self.mtrx.get_pixel(*x, *y);
                color = shader.render(t, color, *x, *y, self).await;
                self.mtrx.set_pixel(*x, *y, color);
            }
        }
    }

    pub async fn render(&mut self, command: &[RenderCommand], t: f64) {
        for c in command.iter() {
            self.render_single(c, t).await;
        }
    }
}

fn hsl2rgb(h: f64, s: f64, l: f64) -> RGB8 {
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

    (r, g, b).into()
}

#[derive(Clone)]
pub enum FragmentShader {
    Breathing(f32),       // speed
    Blinking(f32),        // speed
    LowPass(f32),         // tau
    LowPassWithPeak(f32), // tau
    Rainbow2D(f32),       // speed
}

impl FragmentShader {
    async fn render(
        &self,
        t: f64,
        color: RGB8,
        x: usize,
        y: usize,
        renderman: &mut RenderManager,
    ) -> RGB8 {
        match self {
            FragmentShader::Breathing(speed) => {
                let t = t * *speed as f64;
                let l = 0.5 + 0.5 * (2.0 * f64::consts::PI * t).sin();
                let c = (color.r as f64 * l, color.g as f64 * l, color.b as f64 * l);
                (c.0 as u8, c.1 as u8, c.2 as u8).into()
            }
            FragmentShader::Blinking(speed) => {
                let t = (t * *speed as f64) % 1.0;
                if t < 0.5 {
                    color
                } else {
                    (0, 0, 0).into()
                }
            }

            FragmentShader::LowPass(tau) => {
                // low pass pixel value

                let rgb = renderman.persistent_data.lowpass.get_pixel(x, y);
                let (r, g, b) = (rgb.r as f32, rgb.g as f32, rgb.b as f32);

                let r = r + (color.r as f32 - r) / *tau;
                let g = g + (color.g as f32 - g) / *tau;
                let b = b + (color.b as f32 - b) / *tau;

                let col = (r as u8, g as u8, b as u8).into();
                renderman.persistent_data.lowpass.set_pixel(x, y, col);

                assert!(renderman.persistent_data.lowpass.get_pixel(x, y) == col);

                col
            }

            FragmentShader::LowPassWithPeak(tau) => {
                // low pass pixel value
                // but if the pixel value is higher than the low pass value, use the pixel value

                let rgb = renderman.persistent_data.lowpass.get_pixel(x, y);
                let (r, g, b) = (rgb.r as f32, rgb.g as f32, rgb.b as f32);

                let r = (r + (color.r as f32 - r) / *tau).max(color.r as f32);
                let g = (g + (color.g as f32 - g) / *tau).max(color.g as f32);
                let b = (b + (color.b as f32 - b) / *tau).max(color.b as f32);

                renderman.persistent_data.lowpass.set_pixel(
                    x,
                    y,
                    (r as u8, g as u8, b as u8).into(),
                );

                renderman.persistent_data.lowpass.get_pixel(x, y)
            }

            FragmentShader::Rainbow2D(speed) => {
                // rainbow effect that moves in 2D space

                let t = t * *speed as f64;
                let h = (x as f64 + y as f64) / 16.0 + t;
                hsl2rgb(h % 1.0, 1.0, 0.5)
            }
        }
    }
}

#[derive(Clone)]
pub enum ColorPalette {
    Rainbow(f32), // speed
    Solid(RGB8),
    Custom(Vec<RGB8, 16>, f32), // palette, speed
}

impl Default for ColorPalette {
    fn default() -> Self {
        ColorPalette::Solid((255, 255, 255).into())
    }
}

impl ColorPalette {
    fn render(&self, t: f64) -> RGB8 {
        match self {
            ColorPalette::Rainbow(speed) => hsl2rgb((t * *speed as f64) % 1.0, 1.0, 0.5),
            ColorPalette::Solid(rgb) => *rgb,
            ColorPalette::Custom(palette, speed) => {
                let idx = (t * *speed as f64).floor() as usize % palette.len();
                palette[idx]
            }
        }
    }
}

#[derive(Clone)]
pub enum Pattern {
    Simple(LedPattern),
    Animation(&'static AnimationPattern, f32), // pattern, speed
    AnimationReverse(&'static AnimationPattern, f32), // pattern, speed
    AnimationRandom(&'static AnimationPattern, u16), // pattern, decimation
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern::Simple(LedPattern::new(0b111111111))
    }
}

impl Pattern {
    fn render(&self, t: f64, renderman: &mut RenderManager) -> LedPattern {
        match self {
            Pattern::Simple(pattern) => *pattern,
            Pattern::Animation(pattern, speed) => {
                let idx = (t * *speed as f64) as usize % pattern.patterns.len();
                let pattern = &pattern.patterns[idx];
                *pattern
            }
            Pattern::AnimationReverse(pattern, speed) => {
                let idx = (t * *speed as f64) as usize % pattern.patterns.len();
                let pattern = &pattern.patterns[pattern.patterns.len() - idx - 1];
                *pattern
            }
            Pattern::AnimationRandom(pattern, decimation) => {
                // since picking a random pattern every frame will look like noise,
                // we pick a random pattern every decimation frames

                renderman.persistent_data.frame_counter += 1;

                if renderman.persistent_data.frame_counter % *decimation as u32 == 0 {
                    let idx = renderman.rng.gen_range(0..pattern.patterns.len());
                    let pattern = &pattern.patterns[idx];
                    *pattern
                } else {
                    LedPattern::new(0)
                }
            }
        }
    }
}
