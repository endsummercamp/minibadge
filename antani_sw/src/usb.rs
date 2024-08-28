use defmt::panic;
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, Instance, InterruptHandler};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use log::info;
use static_cell::StaticCell;

use crate::AppSender;
use embassy_usb::class::midi::MidiClass;
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config};

use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

static STATE: StaticCell<State> = StaticCell::new();
static LOGGER_STATE: StaticCell<State> = StaticCell::new();
static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

#[embassy_executor::task]
pub async fn usb_main(usb: USB, control: AppSender) {
    // Create the driver, from the HAL.
    let driver = Driver::new(usb, Irqs);

    // Create embassy-usb Config
    let mut config = Config::new(0x0000, 0x0000);
    config.manufacturer = Some("ESC");
    config.product = Some("Mini Badge");
    config.serial_number = Some("12345678");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    // Required for windows compatibility.
    // https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    let config_descriptor = CONFIG_DESCRIPTOR.init([0; 256]);
    let bos_descriptor = BOS_DESCRIPTOR.init([0; 256]);
    let control_buf = CONTROL_BUF.init([0; 64]);

    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        &mut [], // no msos descriptors
        control_buf,
    );

    let mut midi_class = MidiClass::new(&mut builder, 1, 1, 64);

    let state = STATE.init(State::new());
    let logger_state = LOGGER_STATE.init(State::new());

    let mut cdc_class = CdcAcmClass::new(&mut builder, state, 64);
    let logger_class = CdcAcmClass::new(&mut builder, logger_state, 64);

    let log_fut = embassy_usb_logger::with_class!(1024, log::LevelFilter::Info, logger_class);

    let mut usb = builder.build();

    let usb_fut = usb.run();

    let midi_fut = async {
        loop {
            midi_class.wait_connection().await;
            info!("Connected");
            let _ = midi_echo(&mut midi_class, control).await;
            info!("Disconnected");
        }
    };

    let control_fut = async {
        loop {
            cdc_class.wait_connection().await;
            log::info!("Connected");
            let _ = usb_control(&mut cdc_class).await;
            log::info!("Disconnected");
        }
    };

    join(usb_fut, join(control_fut, join(log_fut, midi_fut))).await;
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn midi_echo<'d, T: Instance + 'd>(
    class: &mut MidiClass<'d, Driver<'d, T>>,
    control: AppSender,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    loop {
        let n = class.read_packet(&mut buf).await?;

        // read at chunk of 4 bytes
        for i in (0..n).step_by(4) {
            //let data = &buf[i..i+4];
            let [_, _, button, value] = buf[i..i + 4]
                .try_into()
                .expect("slice with incorrect length");

            info!("midi pixel: {}, value: {}", button, value);

            // button 0 = pixel 0 red
            // button 1 = pixel 0 green
            // button 2 = pixel 0 blue
            // button 3 = pixel 1 red
            // etc etc

            let width = 3;

            let pixel = button / 3;
            let x = pixel % width;
            let y = pixel / width;
            let channel = button % 3;

            if x >= width || y >= width {
                continue;
            }

            // the (0,0) should be the top left pixel
            // TODO: x and y are probably wrong / inconsistent
            let x = width - x - 1;

            // warning: midi values are 0-127, we need to double them to get 0-255
            control
                .send(crate::TaskCommand::MidiSetPixel(x, y, channel, value * 2))
                .await;
        }
    }
}

async fn usb_control<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    loop {
        let n = class.read_packet(&mut buf).await?;
        let data = &buf[..n];
        info!("usb cdc data: {:?}", data);
        class.write_packet("unimplemented!".as_bytes()).await?;
    }
}
