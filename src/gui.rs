use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};
use crossbeam::channel::{self, Receiver, Sender, TryRecvError};
use eframe::egui::{self, ComboBox, Slider, Visuals};

use crate::config::Settings;
use crate::net::control::{ControlMessage, DeviceInfo};

enum GuiMode {
    Server,
    Client,
}

enum BackendEvent {
    ServerStarted {
        port: u16,
    },
    ServerStopped,
    ClientConnected {
        server: String,
    },
    ClientDisconnected,
    DeviceList {
        devices: Vec<DeviceInfo>,
    },
    Status {
        running: bool,
        clients: u32,
        uptime: u64,
    },
    Error {
        msg: String,
    },
    Log {
        msg: String,
    },
    DeviceDiscovered {
        name: String,
        addr: String,
    },
}

enum GuiCommand {
    StartServer {
        device: String,
        channels: u16,
        sample_rate: u32,
        buffer_frames: usize,
    },
    StopServer,
    ConnectClient {
        addr: String,
    },
    DisconnectClient,
    StartStream,
    StopStream,
    ListDevices,
    SetVolume {
        channel: u16,
        volume: f32,
    },
    DiscoverServers,
}

pub fn run_gui(settings: Settings, cli_server: Option<String>) -> Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([850.0, 620.0])
            .with_min_inner_size([700.0, 500.0])
            .with_title("RJ45 Sound Card — Control Panel"),
        ..Default::default()
    };

    eframe::run_native(
        "RJ45 Sound Card Control",
        opts,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(Visuals::light());
            Ok(Box::new(App::new(settings, cli_server)))
        }),
    )?;
    Ok(())
}

struct App {
    #[allow(dead_code)]
    settings: Settings,
    mode: GuiMode,
    server_running: bool,
    server_handle: Option<JoinHandle<()>>,
    client_connected: bool,
    streaming: bool,
    server_addr: String,
    devices: Vec<DeviceInfo>,
    selected_device: usize,
    channels: u16,
    sample_rate: u32,
    buffer_frames: usize,
    volumes: Vec<f32>,
    status_text: String,
    error_text: String,
    status_clients: u32,
    status_uptime: u64,
    discovered_servers: Vec<String>,
    log_lines: Vec<String>,
    cmd_tx: Sender<GuiCommand>,
    evt_rx: Receiver<BackendEvent>,
    _stop_flag: Arc<AtomicBool>,
}

impl App {
    fn new(settings: Settings, cli_server: Option<String>) -> Self {
        let (cmd_tx, cmd_rx) = channel::unbounded::<GuiCommand>();
        let (evt_tx, evt_rx) = channel::unbounded::<BackendEvent>();
        let stop_flag = Arc::new(AtomicBool::new(false));

        let sf = stop_flag.clone();
        let s = settings.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
            rt.block_on(backend_loop(s, cmd_rx, evt_tx, sf));
        });

        let addr = cli_server.unwrap_or_default();
        Self {
            settings,
            mode: GuiMode::Server,
            server_running: false,
            server_handle: None,
            client_connected: false,
            streaming: false,
            server_addr: addr,
            devices: Vec::new(),
            selected_device: 0,
            channels: 2,
            sample_rate: 48000,
            buffer_frames: 256,
            volumes: vec![1.0; 64],
            status_text: "Ready.".into(),
            error_text: String::new(),
            status_clients: 0,
            status_uptime: 0,
            discovered_servers: Vec::new(),
            log_lines: vec!["RJ45 Sound Card started.".into()],
            cmd_tx,
            evt_rx,
            _stop_flag: stop_flag,
        }
    }

    fn send_cmd(&mut self, cmd: GuiCommand) {
        if let Err(e) = self.cmd_tx.send(cmd) {
            self.error_text = format!("Command error: {}", e);
        }
    }

    fn poll_events(&mut self) {
        loop {
            match self.evt_rx.try_recv() {
                Ok(BackendEvent::ServerStarted { port }) => {
                    self.server_running = true;
                    self.status_text = format!("Server running on port {}", port);
                    self.log(format!("Server started on port {}", port));
                }
                Ok(BackendEvent::ServerStopped) => {
                    self.server_running = false;
                    self.server_handle = None;
                    self.status_text = "Server stopped.".into();
                    self.log("Server stopped.".into());
                }
                Ok(BackendEvent::ClientConnected { server }) => {
                    self.client_connected = true;
                    self.status_text = format!("Connected to {}", server);
                    self.log(format!("Connected to {}", server));
                }
                Ok(BackendEvent::ClientDisconnected) => {
                    self.client_connected = false;
                    self.streaming = false;
                    self.status_text = "Disconnected.".into();
                    self.log("Disconnected from server.".into());
                }
                Ok(BackendEvent::DeviceList { devices }) => {
                    let count = devices.len();
                    self.devices = devices;
                    self.log(format!("Found {} device(s)", count));
                }
                Ok(BackendEvent::Status {
                    running,
                    clients,
                    uptime,
                }) => {
                    self.streaming = running;
                    self.status_clients = clients;
                    self.status_uptime = uptime;
                }
                Ok(BackendEvent::Error { msg }) => {
                    self.error_text = msg.clone();
                    self.log(format!("ERROR: {}", msg));
                }
                Ok(BackendEvent::Log { msg }) => {
                    self.log(msg);
                }
                Ok(BackendEvent::DeviceDiscovered { name, addr }) => {
                    let entry = format!("{} @ {}", name, addr);
                    if !self.discovered_servers.contains(&entry) {
                        self.discovered_servers.push(entry.clone());
                        self.log(format!("Discovered: {}", entry));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.log("Backend disconnected.".into());
                    break;
                }
            }
        }
    }

    fn log(&mut self, msg: String) {
        self.log_lines.push(msg);
        if self.log_lines.len() > 1000 {
            self.log_lines.remove(0);
        }
    }

    fn list_devices_action(&mut self) {
        self.send_cmd(GuiCommand::ListDevices);
    }

    fn start_server_action(&mut self) {
        let dev = if self.selected_device < self.devices.len() {
            self.devices[self.selected_device].name.clone()
        } else {
            "@default".to_string()
        };
        self.send_cmd(GuiCommand::StartServer {
            device: dev,
            channels: self.channels,
            sample_rate: self.sample_rate,
            buffer_frames: self.buffer_frames,
        });
    }

    fn stop_server_action(&mut self) {
        self.send_cmd(GuiCommand::StopServer);
    }

    fn connect_action(&mut self) {
        let addr = self.server_addr.clone();
        if addr.is_empty() {
            self.error_text = "Enter a server address.".into();
            return;
        }
        self.send_cmd(GuiCommand::ConnectClient { addr });
    }

    fn discover_action(&mut self) {
        self.send_cmd(GuiCommand::DiscoverServers);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();

        egui::Panel::top("top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("RJ45 Sound Card");
                ui.separator();

                if self.server_running {
                    ui.label("*");
                    ui.colored_label(egui::Color32::GREEN, "SERVER RUNNING");
                } else if self.client_connected {
                    ui.label("*");
                    ui.colored_label(egui::Color32::GREEN, "CONNECTED");
                } else {
                    ui.label("o");
                    ui.colored_label(egui::Color32::GRAY, "IDLE");
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.streaming {
                        ui.colored_label(egui::Color32::GREEN, "Streaming");
                    }
                    if self.status_clients > 0 {
                        ui.label(format!("Clients: {}", self.status_clients));
                    }
                });
            });
        });

        egui::Panel::left("left_panel")
            .resizable(true)
            .default_size(300.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Mode");
                    ui.horizontal(|ui| {
                        let mode_is_server = matches!(self.mode, GuiMode::Server);
                        let mode_is_client = matches!(self.mode, GuiMode::Client);
                        if ui.selectable_label(mode_is_server, "Server").clicked()
                            && !mode_is_server
                        {
                            self.mode = GuiMode::Server;
                            self.error_text.clear();
                        }
                        if ui.selectable_label(mode_is_client, "Client").clicked()
                            && !mode_is_client
                        {
                            self.mode = GuiMode::Client;
                            self.error_text.clear();
                        }
                    });

                    ui.separator();
                    ui.heading("Settings");

                    match self.mode {
                        GuiMode::Server => self.server_panel(ui),
                        GuiMode::Client => self.client_panel(ui),
                    }

                    if !self.error_text.is_empty() {
                        ui.separator();
                        ui.colored_label(egui::Color32::RED, &self.error_text);
                        if ui.button("Clear error").clicked() {
                            self.error_text.clear();
                        }
                    }
                });
            });

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Status");
            ui.label(&self.status_text);
            ui.separator();

            if !self.devices.is_empty() {
                ui.heading("Available Devices");
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for (i, d) in self.devices.iter().enumerate() {
                            let selected = i == self.selected_device;
                            let resp = ui.selectable_label(
                                selected,
                                format!(
                                    "{} — {} in / {} out",
                                    d.name, d.input_channels, d.output_channels
                                ),
                            );
                            if resp.clicked() {
                                self.selected_device = i;
                            }
                        }
                    });
            }

            ui.separator();

            if matches!(self.mode, GuiMode::Client) && self.client_connected {
                ui.heading("Volume");
                ui.separator();
                let ch = self.channels.min(8) as usize;
                for c in 0..ch {
                    ui.horizontal(|ui| {
                        ui.label(format!("Ch {}:", c + 1));
                        let mut vol = self.volumes[c];
                        if ui.add(Slider::new(&mut vol, 0.0..=1.0).text("")).changed() {
                            self.volumes[c] = vol;
                            self.send_cmd(GuiCommand::SetVolume {
                                channel: c as u16,
                                volume: vol,
                            });
                        }
                    });
                }
            }

            ui.separator();

            ui.heading("Log");
            ui.separator();
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(250.0)
                .show(ui, |ui| {
                    for line in self.log_lines.iter().rev().take(50).rev() {
                        ui.label(line);
                    }
                });
        });
    }
}

impl App {
    fn server_panel(&mut self, ui: &mut egui::Ui) {
        ui.label("Input Device:");
        if !self.devices.is_empty() {
            let names: Vec<String> = self.devices.iter().map(|d| d.name.clone()).collect();
            let current = self.selected_device.min(names.len() - 1);
            ComboBox::from_id_salt("device_selector")
                .selected_text(&names[current])
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        if ui.selectable_label(i == current, name).clicked() {
                            self.selected_device = i;
                        }
                    }
                });
        } else {
            ui.horizontal(|ui| {
                ui.label("(no devices loaded)");
                if ui.button("Refresh").clicked() {
                    self.list_devices_action();
                }
            });
        }

        ui.add_space(8.0);
        if ui.button("List audio devices").clicked() {
            self.list_devices_action();
        }

        if self.devices.is_empty() {
            self.list_devices_action();
        }

        ui.add_space(12.0);
        ui.label("Format:");
        ui.horizontal(|ui| {
            ui.label("Channels:");
            ui.add(Slider::new(&mut self.channels, 1..=64));
        });
        ui.horizontal(|ui| {
            ui.label("Sample rate:");
            let rates = [44100, 48000, 96000, 192000];
            for &r in &rates {
                if ui
                    .selectable_label(self.sample_rate == r, format!("{} Hz", r / 1000))
                    .clicked()
                {
                    self.sample_rate = r;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Buffer:");
            for &b in &[64usize, 128, 256, 512, 1024] {
                if ui
                    .selectable_label(self.buffer_frames == b, format!("{}", b))
                    .clicked()
                {
                    self.buffer_frames = b;
                }
            }
        });

        ui.add_space(12.0);
        if self.server_running {
            if ui.button("Stop Server").clicked() {
                self.stop_server_action();
            }
        } else if ui.button("Start Server").clicked() {
            self.start_server_action();
        }
    }

    fn client_panel(&mut self, ui: &mut egui::Ui) {
        ui.label("Server Address:");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.server_addr).hint_text("192.168.1.100:42002"),
            );
            if ui.button("Discover").clicked() {
                self.discover_action();
            }
        });

        if !self.discovered_servers.is_empty() {
            ui.label("Discovered servers:");
            for sv in self.discovered_servers.clone() {
                if ui.button(&sv).clicked() {
                    if let Some(addr) = sv.split(" @ ").nth(1) {
                        self.server_addr = addr.to_string();
                    }
                }
            }
        }

        ui.add_space(8.0);
        if !self.client_connected {
            if ui.button("Connect").clicked() {
                self.connect_action();
            }
        } else if ui.button("Disconnect").clicked() {
            self.send_cmd(GuiCommand::DisconnectClient);
        }

        ui.add_space(12.0);
        if self.client_connected {
            ui.label("Stream:");
            if self.streaming {
                if ui.button("Stop Stream").clicked() {
                    self.send_cmd(GuiCommand::StopStream);
                }
            } else if ui.button("Start Stream").clicked() {
                self.send_cmd(GuiCommand::StartStream);
            }
        }
    }
}

async fn backend_loop(
    settings: Settings,
    cmd_rx: Receiver<GuiCommand>,
    evt_tx: Sender<BackendEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    use crate::net::control;
    use tokio::net::TcpStream;

    let mut server_thread: Option<(JoinHandle<()>, Arc<AtomicBool>)> = None;
    let mut client_stream: Option<TcpStream> = None;

    let send_evt = |evt_tx: &Sender<BackendEvent>, evt| {
        let _ = evt_tx.send(evt);
    };

    loop {
        if stop_flag.load(Ordering::SeqCst) {
            if let Some((handle, _)) = server_thread.take() {
                let _ = handle.join();
            }
            break;
        }

        match cmd_rx.recv() {
            Ok(GuiCommand::ListDevices) => {
                let devices = list_audio_devices().await;
                send_evt(&evt_tx, BackendEvent::DeviceList { devices });
            }
            Ok(GuiCommand::StartServer {
                device,
                channels,
                sample_rate,
                buffer_frames,
            }) => {
                let evt = evt_tx.clone();
                let mut cfg = settings.clone();
                cfg.audio.input_device = device;
                cfg.audio.channels = channels;
                cfg.audio.sample_rate = sample_rate;
                cfg.audio.buffer_frames = buffer_frames;

                let server_stop_flag = Arc::new(AtomicBool::new(false));
                let ssf = server_stop_flag.clone();
                let handle = std::thread::spawn(move || {
                    let rt =
                        tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
                    rt.block_on(async move {
                        if let Err(e) = crate::server::run(cfg, ssf).await {
                            send_evt(
                                &evt,
                                BackendEvent::Error {
                                    msg: format!("Server: {}", e),
                                },
                            );
                        }
                        send_evt(&evt, BackendEvent::ServerStopped);
                    });
                });
                server_thread = Some((handle, server_stop_flag));
                send_evt(
                    &evt_tx,
                    BackendEvent::ServerStarted {
                        port: settings.network.control_port,
                    },
                );
            }
            Ok(GuiCommand::StopServer) => {
                if let Some((handle, server_stop_flag)) = server_thread.take() {
                    server_stop_flag.store(true, Ordering::SeqCst);
                    let _ = handle.join();
                    send_evt(&evt_tx, BackendEvent::ServerStopped);
                }
            }
            Ok(GuiCommand::ConnectClient { addr }) => match TcpStream::connect(&addr).await {
                Ok(stream) => {
                    client_stream = Some(stream);
                    send_evt(&evt_tx, BackendEvent::ClientConnected { server: addr });
                }
                Err(e) => {
                    send_evt(
                        &evt_tx,
                        BackendEvent::Error {
                            msg: format!("Connection failed: {}", e),
                        },
                    );
                }
            },
            Ok(GuiCommand::DisconnectClient) => {
                client_stream = None;
                send_evt(&evt_tx, BackendEvent::ClientDisconnected);
            }
            Ok(GuiCommand::StartStream) => {
                if let Some(ref mut stream) = client_stream {
                    match control::send_control(
                        stream,
                        &ControlMessage::StartStream {
                            direction: "both".to_string(),
                        },
                    )
                    .await
                    {
                        Ok(()) => send_evt(
                            &evt_tx,
                            BackendEvent::Status {
                                running: true,
                                clients: 0,
                                uptime: 0,
                            },
                        ),
                        Err(e) => send_evt(
                            &evt_tx,
                            BackendEvent::Error {
                                msg: format!("Start stream: {}", e),
                            },
                        ),
                    }
                }
            }
            Ok(GuiCommand::StopStream) => {
                if let Some(ref mut stream) = client_stream {
                    let _ = control::send_control(stream, &ControlMessage::StopStream).await;
                    send_evt(
                        &evt_tx,
                        BackendEvent::Status {
                            running: false,
                            clients: 0,
                            uptime: 0,
                        },
                    );
                }
            }
            Ok(GuiCommand::SetVolume { channel, volume }) => {
                if let Some(ref mut stream) = client_stream {
                    let _ = control::send_control(
                        stream,
                        &ControlMessage::SetVolume { channel, volume },
                    )
                    .await;
                }
            }
            Ok(GuiCommand::DiscoverServers) => {
                match crate::net::discovery::discover_servers(3).await {
                    Ok(servers) => {
                        for (addr, msg) in &servers {
                            send_evt(
                                &evt_tx,
                                BackendEvent::DeviceDiscovered {
                                    name: msg.device_name.clone(),
                                    addr: format!("{}:{}", addr.ip(), msg.control_port),
                                },
                            );
                        }
                        send_evt(
                            &evt_tx,
                            BackendEvent::Log {
                                msg: format!("Discovery found {} server(s)", servers.len()),
                            },
                        );
                    }
                    Err(e) => {
                        send_evt(
                            &evt_tx,
                            BackendEvent::Error {
                                msg: format!("Discovery error: {}", e),
                            },
                        );
                    }
                }
            }
            Err(channel::RecvError) => break,
        }
    }

    if let Some((handle, _)) = server_thread {
        let _ = handle.join();
    }
}

async fn list_audio_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let mut devices = Vec::new();
    if let Ok(dev_iter) = host.devices() {
        for (i, device) in dev_iter.enumerate() {
            let name = device.name().unwrap_or_else(|_| format!("Device {}", i));
            let input_configs = device
                .supported_input_configs()
                .ok()
                .and_then(|mut c| c.next())
                .map(|c| c.channels())
                .unwrap_or(0);
            let output_configs = device
                .supported_output_configs()
                .ok()
                .and_then(|mut c| c.next())
                .map(|c| c.channels())
                .unwrap_or(0);

            let mut sample_rates = Vec::new();
            if let Ok(confs) = device.supported_input_configs() {
                for c in confs {
                    for sr in [c.min_sample_rate().0, c.max_sample_rate().0] {
                        if !sample_rates.contains(&sr) {
                            sample_rates.push(sr);
                        }
                    }
                }
            }
            sample_rates.sort();
            sample_rates.dedup();

            devices.push(DeviceInfo {
                index: i,
                name,
                input_channels: input_configs,
                output_channels: output_configs,
                sample_rates,
            });
        }
    }
    devices
}
