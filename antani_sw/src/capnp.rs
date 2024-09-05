use capnp::{message::ReaderOptions, serialize};
use smart_leds::RGB8;

use crate::{
    rgbeffects::{ColorPalette, RenderCommand},
    usb_messages_capnp, RawFramebuffer, TaskCommand,
};

pub fn deserialize_message(data: &mut &[u8]) -> Result<TaskCommand, capnp::Error> {
    log::info!("Deserializing message of length {}", data.len());

    let reader = serialize::read_message_from_flat_slice_no_alloc(data, ReaderOptions::new())?;

    let badgebound = reader.get_root::<usb_messages_capnp::badge_bound::Reader>()?;

    match badgebound.which()? {
        usb_messages_capnp::badge_bound::SetFrameBuffer(set_fb) => {
            let mut ret: RawFramebuffer<RGB8> = RawFramebuffer::new();

            let pixels = set_fb?.get_pixels()?;
            for i in 0..9 {
                let pixel = pixels.get(i);

                let x = i % 3;
                let y = i / 3;

                ret.set_pixel(
                    x as usize,
                    y as usize,
                    RGB8 {
                        r: pixel.get_r(),
                        g: pixel.get_g(),
                        b: pixel.get_b(),
                    },
                );
            }

            return Ok(TaskCommand::SetWorkingMode(
                crate::WorkingMode::RawFramebuffer(ret),
            ));
        }
        usb_messages_capnp::badge_bound::SetSolidColor(color) => {
            let color = color?;

            let color = RGB8 {
                r: color.get_r(),
                g: color.get_g(),
                b: color.get_b(),
            };

            let scene = RenderCommand {
                color: ColorPalette::Solid(color),
                ..Default::default()
            };

            return Ok(TaskCommand::SetWorkingMode(crate::WorkingMode::Special(
                scene,
            )));
        }
        usb_messages_capnp::badge_bound::Which::SendNecCommand(command) => {
            let command = command?;

            let address = command.get_address();
            let _command = command.get_command();
            let repeat = command.get_repeat();

            return Ok(TaskCommand::SendIrNec(address, _command, repeat));
        }

        usb_messages_capnp::badge_bound::Which::Null(_) => {}
    }

    Ok(TaskCommand::None)
}
