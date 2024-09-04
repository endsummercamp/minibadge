use defmt::{panic, warn};
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, Instance, InterruptHandler};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use heapless::Vec;
use log::{error, info};
use static_cell::StaticCell;

use crate::MegaPublisher;
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
pub async fn usb_main(usb: USB, publisher: MegaPublisher) {
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

    let log_fut = embassy_usb_logger::with_custom_style!(
        1024,
        log::LevelFilter::Info,
        logger_class,
        |record, writer| {
            use core::fmt::Write;
            let level = record.level().as_str();
            write!(writer, "[{level}] {}\r\n", record.args()).unwrap();
        }
    );

    let mut usb = builder.build();

    let usb_fut = usb.run();

    let midi_fut = async {
        loop {
            midi_class.wait_connection().await;
            info!("Connected");
            let _ = midi_echo(&mut midi_class, &publisher).await;
            info!("Disconnected");
        }
    };

    let control_fut = async {
        loop {
            cdc_class.wait_connection().await;
            log::info!("Connected");
            let _ = usb_control(&mut cdc_class, &publisher).await;
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
    publisher: &MegaPublisher,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    loop {
        let n = class.read_packet(&mut buf).await?;

        // read at chunk of 4 bytes
        for i in (0..n).step_by(4) {
            //let data = &buf[i..i+4];
            let buf: &[u8; 4] = match buf[i..i + 4].try_into() {
                Ok(buf) => buf,
                Err(_) => {
                    warn!("got bad midi data");
                    continue;
                }
            };

            let [_, _, button, value] = buf;

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
            publisher
                .publish(crate::TaskCommand::MidiSetPixel(x, y, channel, value * 2))
                .await;
        }
    }
}

struct AlignedVec {
    x: Vec<u8, 256>,
    _alignment: [u64; 0],
}

impl AlignedVec {
    fn new() -> Self {
        Self {
            x: Vec::<u8, 256>::new(),
            _alignment: [0; 0],
        }
    }
}

async fn usb_control<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
    publisher: &MegaPublisher,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    let mut mega_deserialization_buf = AlignedVec::new();
    loop {
        let n = class.read_packet(&mut buf).await?;
        let data = &buf[..n];
        info!("usb cdc data: {:?}", data);

        // append to the mega deserialization buffer
        // we don't really care if it fails, we'll just clear it later
        mega_deserialization_buf.x.extend_from_slice(data).ok();

        let e = crate::capnp::deserialize_message(&mut mega_deserialization_buf.x.as_slice());

        match e {
            Ok(command) => {
                info!("Deserialized message");

                mega_deserialization_buf.x.clear();

                publisher.publish(command).await;
            }
            Err(e) => match e.kind {
                capnp::ErrorKind::MessageEndsPrematurely(_, _) => {
                    continue;
                }

                e => {
                    error!("Error deserializing message: {:?}", e);

                    mega_deserialization_buf.x.clear();
                }
            },
        }
    }
}
