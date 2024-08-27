use core::f64;
use heapless::Vec;
use num_traits::real::Real;
use rand::{rngs::SmallRng, Rng};
use smart_leds::RGB8;

use crate::{LedFramebuffer, LedMatrix};

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
    pub effect: RunEffect,
    pub color: ColorPalette,
    pub pattern_shaders: Vec<FragmentShader, 8>,
    pub screen_shaders: Vec<FragmentShader, 8>,
}

pub struct RenderManager {
    pub mtrx: LedMatrix,
    pub rng: SmallRng,
}

impl RenderManager {
    fn render_single(&mut self, command: &RenderCommand, t: f64) {
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
                    color = shader.render(t, color, *x, *y);
                }

                self.mtrx.set_pixel(*x, *y, color);
            }

            for shader in command.screen_shaders.iter() {
                let mut color = self.mtrx.get_pixel(*x, *y);
                color = shader.render(t, color, *x, *y);
                self.mtrx.set_pixel(*x, *y, color);
            }
        }
    }

    pub fn render(&mut self, command: &[RenderCommand], t: f64) {
        for c in command.iter() {
            self.render_single(c, t);
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
}

impl FragmentShader {
    fn render(&self, t: f64, color: RGB8, x: usize, y: usize) -> RGB8 {
        static mut LOWPASS: LedFramebuffer = LedFramebuffer::new();

        match self {
            FragmentShader::Breathing(speed) => {
                let t = (t * *speed as f64) % 1.0;
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
                let lowpass = unsafe { &mut LOWPASS };

                let rgb = lowpass.get_pixel(x, y);
                let (r, g, b) = (rgb.r as f32, rgb.g as f32, rgb.b as f32);

                let r = r + (color.r as f32 - r) / *tau;
                let g = g + (color.g as f32 - g) / *tau;
                let b = b + (color.b as f32 - b) / *tau;

                lowpass.set_pixel(x, y, (r as u8, g as u8, b as u8).into());

                lowpass.get_pixel(x, y)
            }

            FragmentShader::LowPassWithPeak(tau) => {
                // low pass pixel value
                // but if the pixel value is higher than the low pass value, use the pixel value

                let lowpass = unsafe { &mut LOWPASS };

                let rgb = lowpass.get_pixel(x, y);
                let (r, g, b) = (rgb.r as f32, rgb.g as f32, rgb.b as f32);

                let r = (r + (color.r as f32 - r) / *tau).max(color.r as f32);
                let g = (g + (color.g as f32 - g) / *tau).max(color.g as f32);
                let b = (b + (color.b as f32 - b) / *tau).max(color.b as f32);

                lowpass.set_pixel(x, y, (r as u8, g as u8, b as u8).into());

                lowpass.get_pixel(x, y)
            }
        }
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum ColorPalette {
    Rainbow(f32, f32), // speed, phase
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
            ColorPalette::Rainbow(speed, phase) => {
                hsl2rgb((t * *speed as f64 + *phase as f64) % 1.0, 1.0, 0.5)
            }
            ColorPalette::Solid(rgb) => *rgb,
            ColorPalette::Custom(palette, speed) => {
                let idx = (t * *speed as f64) as usize % palette.len();
                palette[idx]
            }
        }
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum RunEffect {
    SimplePattern(LedPattern),
    AnimationPattern(&'static AnimationPattern, f32), // pattern, speed
    ReverseAnimationPattern(&'static AnimationPattern, f32), // pattern, speed
    AnimationRandom(&'static AnimationPattern),       // pattern
}

impl Default for RunEffect {
    fn default() -> Self {
        RunEffect::SimplePattern(LedPattern::new(0b111111111))
    }
}

impl RunEffect {
    fn render(&self, t: f64, renderman: &mut RenderManager) -> LedPattern {
        match self {
            RunEffect::SimplePattern(pattern) => *pattern,
            RunEffect::AnimationPattern(pattern, speed) => {
                let idx = (t * *speed as f64) as usize % pattern.patterns.len();
                let pattern = &pattern.patterns[idx];
                *pattern
            }
            RunEffect::ReverseAnimationPattern(pattern, speed) => {
                let idx = (t * *speed as f64) as usize % pattern.patterns.len();
                let pattern = &pattern.patterns[pattern.patterns.len() - idx - 1];
                *pattern
            }
            RunEffect::AnimationRandom(pattern) => {
                let idx = renderman.rng.gen_range(0..pattern.patterns.len());
                let pattern = &pattern.patterns[idx];
                *pattern
            }
        }
    }
}
