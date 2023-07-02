use std::{
    ffi::c_void,
    ptr::null_mut,
    {cell::RefCell, time::*},
};

use anyhow::{bail, Result};

use esp_idf_hal::ledc::{
    config::TimerConfig,
    {LedcDriver, LedcTimerDriver},
};
use log::*;

use embedded_svc::{ipv4, wifi::*};

use esp_idf_svc::{eventloop::*, ping, wifi::*};

use esp_idf_hal::{delay::Delay, peripheral, prelude::*};

use esp_idf_sys::{
    self, esp_lcd_new_rgb_panel, esp_lcd_panel_del, esp_lcd_panel_draw_bitmap,
    esp_lcd_panel_handle_t, esp_lcd_panel_init, esp_lcd_panel_reset, esp_lcd_rgb_panel_config_t,
    esp_lcd_rgb_panel_config_t__bindgen_ty_1, esp_lcd_rgb_panel_get_frame_buffer,
    esp_lcd_rgb_timing_t, esp_lcd_rgb_timing_t__bindgen_ty_1,
    soc_periph_lcd_clk_src_t_LCD_CLK_SRC_PLL160M, ESP_OK,
};

const SSID: &str = include_str!("../wifi_name.txt");
const PASS: &str = include_str!("../wifi_pass.txt");

thread_local! {
    static TLS: RefCell<u32> = RefCell::new(13);
}

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    #[allow(unused)]
    let peripherals = Peripherals::take().unwrap();
    // #[allow(unused)]
    let pins = peripherals.pins;

    let timings = esp_lcd_rgb_timing_t {
        pclk_hz: 6000000,
        h_res: 800,
        v_res: 480,
        hsync_pulse_width: 30,
        hsync_back_porch: 16,
        hsync_front_porch: 210,
        vsync_pulse_width: 13,
        vsync_back_porch: 10,
        vsync_front_porch: 22,
        flags: {
            let mut flags = esp_lcd_rgb_timing_t__bindgen_ty_1::default();
            flags.set_vsync_idle_low(1);
            flags.set_hsync_idle_low(1);
            flags.set_de_idle_high(0);
            flags.set_pclk_active_neg(1);
            flags.set_pclk_idle_high(0);
            flags
        },
    };

    let panel_config = esp_lcd_rgb_panel_config_t {
        clk_src: soc_periph_lcd_clk_src_t_LCD_CLK_SRC_PLL160M,
        timings,
        data_width: 16,
        bits_per_pixel: 0,
        num_fbs: 1,
        bounce_buffer_size_px: 0,
        sram_trans_align: 8,
        psram_trans_align: 64,
        hsync_gpio_num: 39,
        vsync_gpio_num: 40,
        de_gpio_num: 41,
        pclk_gpio_num: 42,
        disp_gpio_num: -1,
        data_gpio_nums: [15, 7, 6, 5, 4, 9, 46, 3, 8, 16, 1, 14, 21, 47, 48, 45],
        flags: {
            let mut flags = esp_lcd_rgb_panel_config_t__bindgen_ty_1::default();
            flags.set_disp_active_low(0);
            flags.set_fb_in_psram(1);
            // can't find relax_on_idle flag?
            flags
        },
    };

    let mut panel = null_mut() as esp_lcd_panel_handle_t;

    info!("Calling esp_lcd_new_rgb_panel with config {panel_config:?}");
    let err = unsafe { esp_lcd_new_rgb_panel(&panel_config, &mut panel) };

    if err != ESP_OK {
        error!("Initializing LCD panel failed with error {err}.");
        panel = null_mut();
    }

    let err = unsafe { esp_lcd_panel_reset(panel) };
    if err != ESP_OK {
        error!("esp_lcd_panel_reset error {err}");
    } else {
        let err = unsafe { esp_lcd_panel_init(panel) };

        if err != ESP_OK {
            error!("esp_lcd_panel_init error {err}");
        } else {
            info!("Enabling backlight...");
            let mut channel = LedcDriver::new(
                peripherals.ledc.channel0,
                LedcTimerDriver::new(
                    peripherals.ledc.timer0,
                    &TimerConfig::new().frequency(25.kHz().into()),
                )?,
                pins.gpio2,
            )
            .unwrap();
            channel.set_duty(channel.get_max_duty() / 2).unwrap();
            info!("Backlight turned on.");

            let bitmap = [0xf800u16; 32 * 32]; // red
            let mut x = 0;

            loop {
                let mut fb = null_mut();
                let err = unsafe { esp_lcd_rgb_panel_get_frame_buffer(panel, 1, &mut fb) };

                if err != ESP_OK {
                    error!("esp_lcd_rgb_panel_get_frame_buffer error {err}");
                } else {
                    let buffer = fb as *mut [u16; 800 * 480];
                    for idx in 0..(800 * 480) {
                        unsafe {
                            (*buffer)[idx] = 0x1f; // blue
                        }
                    }
                }

                let err = unsafe {
                    esp_lcd_panel_draw_bitmap(
                        panel,
                        x,
                        0,
                        x + 32,
                        32,
                        bitmap.as_ptr() as *const c_void,
                    )
                };
                if err != ESP_OK {
                    error!("esp_lcd_panel_draw_bitmap error {err}");
                    break;
                }

                Delay::delay_ms(100);
                x = (x + 1) % (800 - 32);
            }
        }
    }

    // let sysloop = EspSystemEventLoop::take()?;

    // let wifi = wifi(peripherals.modem, sysloop)?;

    // let _mqtt_client = test_mqtt_client()?;

    // let mutex = Arc::new((Mutex::new(None), Condvar::new()));

    // let httpd = httpd(mutex.clone())?;

    // let _unused = mutex.0.lock().unwrap();

    // for s in 0..3 {
    //     info!("Shutting down in {} secs", 3 - s);
    //     thread::sleep(Duration::from_secs(1));
    // }

    // drop(httpd);
    // info!("Httpd stopped");

    // drop(wifi);
    // info!("Wifi stopped");

    if !panel.is_null() {
        unsafe {
            esp_lcd_panel_del(panel);
        }
    }

    Ok(())
}

#[derive(Copy, Clone, Debug)]
struct EventLoopMessage(Duration);

impl EventLoopMessage {
    pub fn new(duration: Duration) -> Self {
        Self(duration)
    }
}

impl EspTypedEventSource for EventLoopMessage {
    fn source() -> *const std::ffi::c_char {
        b"DEMO-SERVICE\0".as_ptr() as *const _
    }
}

impl EspTypedEventSerializer<EventLoopMessage> for EventLoopMessage {
    fn serialize<R>(
        event: &EventLoopMessage,
        f: impl for<'a> FnOnce(&'a EspEventPostData) -> R,
    ) -> R {
        f(&unsafe { EspEventPostData::new(Self::source(), Self::event_id(), event) })
    }
}

impl EspTypedEventDeserializer<EventLoopMessage> for EventLoopMessage {
    fn deserialize<R>(
        data: &EspEventFetchData,
        f: &mut impl for<'a> FnMut(&'a EventLoopMessage) -> R,
    ) -> R {
        f(unsafe { data.as_payload() })
    }
}

// fn test_eventloop() -> Result<(EspBackgroundEventLoop, EspBackgroundSubscription)> {
//     use embedded_svc::event_bus::EventBus;

//     info!("About to start a background event loop");
//     let eventloop = EspBackgroundEventLoop::new(&Default::default())?;

//     info!("About to subscribe to the background event loop");
//     let subscription = eventloop.subscribe(|message: &EventLoopMessage| {
//         info!("Got message from the event loop: {:?}", message.0);
//     })?;

//     Ok((eventloop, subscription))
// }

// fn test_mqtt_client() -> Result<EspMqttClient<ConnState<MessageImpl, EspError>>> {
//     info!("About to start MQTT client");

//     let conf = MqttClientConfiguration {
//         client_id: Some("rust-esp32-std-demo"),
//         crt_bundle_attach: Some(esp_idf_sys::esp_crt_bundle_attach),

//         ..Default::default()
//     };

//     let (mut client, mut connection) =
//         EspMqttClient::new_with_conn("mqtts://broker.emqx.io:8883", &conf)?;

//     info!("MQTT client started");

//     // Need to immediately start pumping the connection for messages, or else subscribe() and publish() below will not work
//     // Note that when using the alternative constructor - `EspMqttClient::new` - you don't need to
//     // spawn a new thread, as the messages will be pumped with a backpressure into the callback you provide.
//     // Yet, you still need to efficiently process each message in the callback without blocking for too long.
//     //
//     // Note also that if you go to http://tools.emqx.io/ and then connect and send a message to topic
//     // "rust-esp32-std-demo", the client configured here should receive it.
//     thread::spawn(move || {
//         info!("MQTT Listening for messages");

//         while let Some(msg) = connection.next() {
//             match msg {
//                 Err(e) => info!("MQTT Message ERROR: {}", e),
//                 Ok(msg) => info!("MQTT Message: {:?}", msg),
//             }
//         }

//         info!("MQTT connection loop exit");
//     });

//     client.subscribe("rust-esp32-std-demo", QoS::AtMostOnce)?;

//     info!("Subscribed to all topics (rust-esp32-std-demo)");

//     client.publish(
//         "rust-esp32-std-demo",
//         QoS::AtMostOnce,
//         false,
//         "Hello from rust-esp32-std-demo!".as_bytes(),
//     )?;

//     info!("Published a hello message to topic \"rust-esp32-std-demo\"");

//     Ok(client)
// }

#[allow(dead_code)]
fn wifi(
    modem: impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> Result<Box<EspWifi<'static>>> {
    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), None)?;

    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;

    info!("Starting wifi...");

    wifi.start()?;

    info!("Scanning...");

    let ap_infos = wifi.scan()?;

    let ours = ap_infos.into_iter().find(|a| a.ssid == SSID);

    let channel = if let Some(ours) = ours {
        info!(
            "Found configured access point {} on channel {}",
            SSID, ours.channel
        );
        Some(ours.channel)
    } else {
        info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            SSID
        );
        None
    };

    wifi.set_configuration(&Configuration::Mixed(
        ClientConfiguration {
            ssid: SSID.into(),
            password: PASS.into(),
            channel,
            ..Default::default()
        },
        AccessPointConfiguration {
            ssid: "aptest".into(),
            channel: channel.unwrap_or(1),
            ..Default::default()
        },
    ))?;

    info!("Connecting wifi...");

    wifi.connect()?;

    info!("Waiting for DHCP lease...");

    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    info!("Wifi DHCP info: {:?}", ip_info);

    ping(ip_info.subnet.gateway)?;

    Ok(Box::new(esp_wifi))
}

fn ping(ip: ipv4::Ipv4Addr) -> Result<()> {
    info!("About to do some pings for {:?}", ip);

    let ping_summary = ping::EspPing::default().ping(ip, &Default::default())?;
    if ping_summary.transmitted != ping_summary.received {
        bail!("Pinging IP {} resulted in timeouts", ip);
    }

    info!("Pinging done");

    Ok(())
}
